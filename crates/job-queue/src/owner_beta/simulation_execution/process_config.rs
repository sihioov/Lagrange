use std::fmt;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::owner_beta::simulation::parse_strict_json;

const CONFIG_SIZE_LIMIT: usize = 16 * 1024;
const PROCESS_CONFIG_SCHEMA: &str = "owner-beta-simulation-process-v1";
const MANIFEST_FORMAT: &str = "LAGRANGE_RELEASE_MANIFEST_V3";
const BUILD_ORIGIN: &str = "official-release-v1";
const PACKAGE: &str = "job-queue";
const BIN: &str = "owner-beta-simulation-runner";
const TARGET: &str = "x86_64-unknown-linux-musl";
const PROFILE: &str = "release";
const TOOLCHAIN: &str = "1.97.1";

const CONFIG_PATH: &str = "/run/owner-beta-simulation/config.json";
const ARTIFACT_ROOT: &str = "/data/owner-beta-artifacts";
const RECEIPT_PATH: &str = "/usr/share/lagrange/owner-beta-simulation/artifact.json";
const COMPLETE_PATH: &str = "/usr/share/lagrange/owner-beta-simulation/COMPLETE";
const EXECUTABLE_PATH: &str = "/usr/local/bin/owner-beta-simulation-runner";
const WORKER_ID: &str = "owner-beta-simulation-runner";

const MAX_I64: u64 = i64::MAX as u64;
const MAX_I32: u64 = i32::MAX as u64;

/// Value-free failures for the checked simulation process configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationProcessConfigError {
    #[error("simulation process config exceeds the size limit")]
    SizeLimitExceeded,
    #[error("simulation process config is invalid")]
    Invalid,
}

/// Checked process limits parsed from the protected configuration grammar.
///
/// Values are available only after `SimulationProcessConfig::from_json` has
/// checked the full configuration and its exact input-byte digest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SimulationProcessLimits {
    memory_bytes: u64,
    nano_cpus: u64,
    health_tmpfs_bytes: u64,
    pids_limit: u64,
    startup_timeout_seconds: u64,
    health_max_age_seconds: u64,
    stop_timeout_seconds: u64,
}

impl fmt::Debug for SimulationProcessLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationProcessLimits")
    }
}

impl SimulationProcessLimits {
    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    pub fn nano_cpus(&self) -> u64 {
        self.nano_cpus
    }

    pub fn health_tmpfs_bytes(&self) -> u64 {
        self.health_tmpfs_bytes
    }

    pub fn pids_limit(&self) -> u64 {
        self.pids_limit
    }

    pub fn startup_timeout_seconds(&self) -> u64 {
        self.startup_timeout_seconds
    }

    pub fn health_max_age_seconds(&self) -> u64 {
        self.health_max_age_seconds
    }

    pub fn stop_timeout_seconds(&self) -> u64 {
        self.stop_timeout_seconds
    }
}

/// Closed process configuration whose fields are bound to the input bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SimulationProcessConfig {
    config_sha256: String,
    code_commit: String,
    manifest_sha256: String,
    image_id: String,
    receipt_sha256: String,
    binary_sha256: String,
    source_input_sha256: String,
    recipe_sha256: String,
    owner: Uuid,
    entitlement_reference: String,
    entitlement_sha256: String,
    historical_root: String,
    reader_uid: u32,
    reader_gid: u32,
    limits: SimulationProcessLimits,
}

impl fmt::Debug for SimulationProcessConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationProcessConfig")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationProcessConfigWire {
    schema: String,
    code_commit: String,
    manifest_format: String,
    manifest_sha256: String,
    image_id: String,
    receipt_sha256: String,
    binary_sha256: String,
    source_input_sha256: String,
    recipe_sha256: String,
    build_origin: String,
    package: String,
    bin: String,
    target: String,
    profile: String,
    toolchain: String,
    owner: String,
    entitlement_reference: String,
    entitlement_sha256: String,
    historical_root: String,
    reader_uid: u32,
    reader_gid: u32,
    paths: SimulationProcessPathsWire,
    limits: SimulationProcessLimitsWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationProcessPathsWire {
    config: String,
    artifact_root: String,
    receipt: String,
    complete: String,
    executable: String,
    worker_id: String,
}

impl SimulationProcessPathsWire {
    fn is_exact(&self) -> bool {
        self.config == CONFIG_PATH
            && self.artifact_root == ARTIFACT_ROOT
            && self.receipt == RECEIPT_PATH
            && self.complete == COMPLETE_PATH
            && self.executable == EXECUTABLE_PATH
            && self.worker_id == WORKER_ID
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationProcessLimitsWire {
    memory_bytes: u64,
    nano_cpus: u64,
    health_tmpfs_bytes: u64,
    pids_limit: u64,
    startup_timeout_seconds: u64,
    health_max_age_seconds: u64,
    stop_timeout_seconds: u64,
}

impl SimulationProcessLimitsWire {
    fn is_valid(&self) -> bool {
        in_range(self.memory_bytes, 1, MAX_I64)
            && in_range(self.nano_cpus, 1, MAX_I64)
            && in_range(self.health_tmpfs_bytes, 1, MAX_I64)
            && in_range(self.pids_limit, 1, MAX_I32)
            && in_range(self.startup_timeout_seconds, 1, MAX_I32)
            && in_range(self.health_max_age_seconds, 1, MAX_I32)
            && in_range(self.stop_timeout_seconds, 11, MAX_I32)
    }

    fn into_limits(self) -> SimulationProcessLimits {
        SimulationProcessLimits {
            memory_bytes: self.memory_bytes,
            nano_cpus: self.nano_cpus,
            health_tmpfs_bytes: self.health_tmpfs_bytes,
            pids_limit: self.pids_limit,
            startup_timeout_seconds: self.startup_timeout_seconds,
            health_max_age_seconds: self.health_max_age_seconds,
            stop_timeout_seconds: self.stop_timeout_seconds,
        }
    }
}

impl SimulationProcessConfig {
    /// Parses and checks protected process configuration bytes without I/O.
    pub fn from_json(
        bytes: &[u8],
        expected_commit: &str,
        expected_config_sha256: &str,
    ) -> Result<Self, SimulationProcessConfigError> {
        if bytes.len() > CONFIG_SIZE_LIMIT {
            return Err(SimulationProcessConfigError::SizeLimitExceeded);
        }
        if !is_commit(expected_commit) || !is_sha256(expected_config_sha256) {
            return Err(SimulationProcessConfigError::Invalid);
        }

        let value = parse_strict_json(bytes).map_err(|_| SimulationProcessConfigError::Invalid)?;
        let Some(root) = value.as_object() else {
            return Err(SimulationProcessConfigError::Invalid);
        };
        if !matches!(root.get("paths"), Some(serde_json::Value::Object(_)))
            || !matches!(root.get("limits"), Some(serde_json::Value::Object(_)))
        {
            return Err(SimulationProcessConfigError::Invalid);
        }
        let wire = serde_json::from_value::<SimulationProcessConfigWire>(value)
            .map_err(|_| SimulationProcessConfigError::Invalid)?;

        if wire.schema != PROCESS_CONFIG_SCHEMA
            || wire.manifest_format != MANIFEST_FORMAT
            || wire.build_origin != BUILD_ORIGIN
            || wire.package != PACKAGE
            || wire.bin != BIN
            || wire.target != TARGET
            || wire.profile != PROFILE
            || wire.toolchain != TOOLCHAIN
            || wire.code_commit != expected_commit
            || !is_sha256(&wire.manifest_sha256)
            || !is_sha256(&wire.image_id)
            || !is_sha256(&wire.receipt_sha256)
            || !is_sha256(&wire.binary_sha256)
            || !is_sha256(&wire.source_input_sha256)
            || !is_sha256(&wire.recipe_sha256)
            || !is_sha256(&wire.entitlement_sha256)
            || !wire.paths.is_exact()
            || !wire.limits.is_valid()
            || wire.reader_uid != 10001
            || wire.reader_gid != 10001
            || !is_valid_entitlement_reference(&wire.entitlement_reference)
            || !is_valid_historical_root(&wire.historical_root)
        {
            return Err(SimulationProcessConfigError::Invalid);
        }

        let owner =
            Uuid::parse_str(&wire.owner).map_err(|_| SimulationProcessConfigError::Invalid)?;
        if owner.is_nil() || owner.to_string() != wire.owner {
            return Err(SimulationProcessConfigError::Invalid);
        }

        let actual_config_sha256 = digest_sha256(bytes);
        if actual_config_sha256 != expected_config_sha256 {
            return Err(SimulationProcessConfigError::Invalid);
        }

        Ok(Self {
            config_sha256: actual_config_sha256,
            code_commit: wire.code_commit,
            manifest_sha256: wire.manifest_sha256,
            image_id: wire.image_id,
            receipt_sha256: wire.receipt_sha256,
            binary_sha256: wire.binary_sha256,
            source_input_sha256: wire.source_input_sha256,
            recipe_sha256: wire.recipe_sha256,
            owner,
            entitlement_reference: wire.entitlement_reference,
            entitlement_sha256: wire.entitlement_sha256,
            historical_root: wire.historical_root,
            reader_uid: wire.reader_uid,
            reader_gid: wire.reader_gid,
            limits: wire.limits.into_limits(),
        })
    }

    pub fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub fn code_commit(&self) -> &str {
        &self.code_commit
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn image_id(&self) -> &str {
        &self.image_id
    }

    pub fn receipt_sha256(&self) -> &str {
        &self.receipt_sha256
    }

    pub fn binary_sha256(&self) -> &str {
        &self.binary_sha256
    }

    pub fn source_input_sha256(&self) -> &str {
        &self.source_input_sha256
    }

    pub fn recipe_sha256(&self) -> &str {
        &self.recipe_sha256
    }

    pub fn owner(&self) -> Uuid {
        self.owner
    }

    pub fn entitlement_reference(&self) -> &str {
        &self.entitlement_reference
    }

    pub fn entitlement_sha256(&self) -> &str {
        &self.entitlement_sha256
    }

    pub fn historical_root(&self) -> &str {
        &self.historical_root
    }

    pub fn reader_uid(&self) -> u32 {
        self.reader_uid
    }

    pub fn reader_gid(&self) -> u32 {
        self.reader_gid
    }

    pub fn limits(&self) -> &SimulationProcessLimits {
        &self.limits
    }

    pub fn config_path(&self) -> &str {
        CONFIG_PATH
    }

    pub fn artifact_root(&self) -> &str {
        ARTIFACT_ROOT
    }

    pub fn receipt_path(&self) -> &str {
        RECEIPT_PATH
    }

    pub fn complete_path(&self) -> &str {
        COMPLETE_PATH
    }

    pub fn executable_path(&self) -> &str {
        EXECUTABLE_PATH
    }

    pub fn worker_id(&self) -> &str {
        WORKER_ID
    }
}

fn in_range(value: u64, minimum: u64, maximum: u64) -> bool {
    (minimum..=maximum).contains(&value)
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(is_lower_hex) && value.bytes().any(|byte| byte != b'0')
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(is_lower_hex))
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn digest_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_control(value: char) -> bool {
    matches!(value as u32, 0x00..=0x1f | 0x7f..=0x9f)
}

fn is_valid_entitlement_reference(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= 512
        && value.len() <= 4096
        && !value.chars().any(is_control)
}

fn is_valid_historical_root(value: &str) -> bool {
    value.starts_with('/')
        && value != "/"
        && !value.ends_with('/')
        && !value.chars().any(is_control)
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
}
