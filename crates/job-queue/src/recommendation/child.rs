//! Bounded process boundary for the shipped Python target generators.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use uuid::Uuid;

use crate::types::ErrorClass;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 1024 * 1024;
const MAX_STATUS_BYTES: u64 = 16 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;
const CLEANUP_GRACE: Duration = Duration::from_secs(2);
const CREATE_ATTEMPTS: usize = 8;

#[derive(Debug, Clone)]
pub struct TargetChildPaths {
    /// Absolute executable path. The child launcher never searches `PATH`.
    pub uv_bin: PathBuf,
    /// Absolute repository root containing the `nt` uv project.
    pub repo_root: PathBuf,
    /// Absolute, pre-created non-symlink directory for per-job scratch data.
    pub temp_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetProvenance {
    pub dataset_version_id: Uuid,
    pub dataset_id: String,
    pub dataset_version: String,
    pub curated_version: u32,
    pub dataset_manifest_sha256: String,
    pub universe_snapshot_id: String,
    pub factor_snapshot_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetChildRequest {
    pub strategy_id: String,
    pub strategy_version: String,
    pub parameters: serde_json::Value,
    pub as_of: String,
    pub universe: Vec<String>,
    pub factors: BTreeMap<String, BTreeMap<String, Option<f64>>>,
    pub provenance: TargetProvenance,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetChildOutput {
    pub as_of: String,
    pub strategy_version: String,
    pub universe_snapshot_id: String,
    pub factor_snapshot_hash: String,
    pub dataset_version_id: Uuid,
    pub dataset_id: String,
    pub dataset_version: String,
    pub curated_version: u32,
    pub dataset_manifest_sha256: String,
    pub targets: Vec<TargetRow>,
    pub exclusions: Vec<ExclusionRow>,
    #[serde(deserialize_with = "finite_f64")]
    pub cash_weight: f64,
    pub constraints: ConstraintSummary,
    pub portfolio_reasons: Vec<Reason>,
    pub portfolio_snapshot_id: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetRow {
    pub instrument_id: String,
    pub rank: usize,
    #[serde(deserialize_with = "finite_f64")]
    pub score: f64,
    #[serde(deserialize_with = "finite_factor_map")]
    pub factors: BTreeMap<String, f64>,
    #[serde(deserialize_with = "finite_f64")]
    pub target_weight: f64,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusionRow {
    pub instrument_id: String,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reason {
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub text_ko: String,
    pub text_en: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintSummary {
    pub top_n: usize,
    #[serde(deserialize_with = "finite_f64")]
    pub max_weight: f64,
    #[serde(deserialize_with = "finite_f64")]
    pub cash_floor: f64,
    pub weight_scale: u8,
    #[serde(deserialize_with = "finite_f64")]
    pub tolerance: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum TargetChildError {
    #[error("unsafe target child path configuration")]
    UnsafePath,
    #[error("target child scratch path is already in use")]
    ScratchCollision,
    #[error("target child request exceeds its size limit")]
    RequestTooLarge,
    #[error("target child request is structurally invalid")]
    InvalidRequest,
    #[error("target child request could not be staged")]
    RequestWrite,
    #[error("target child could not be launched")]
    Launch,
    #[error("target child timed out")]
    Timeout,
    #[error("target child could not be terminated cleanly")]
    Termination,
    #[error("target child status exceeds its size limit")]
    StatusTooLarge,
    #[error("target child status is malformed")]
    InvalidStatus,
    #[error("target child reported {code}")]
    ChildStatus { code: String },
    #[error("target child exited without a valid status")]
    Exited,
    #[error("target child did not publish a result")]
    NoResult,
    #[error("target child result exceeds its size limit")]
    ResultTooLarge,
    #[error("target child result is malformed")]
    InvalidResult,
    #[error("target child scratch cleanup failed")]
    Cleanup,
}

impl TargetChildError {
    pub fn code(&self) -> &str {
        match self {
            Self::UnsafePath => "TARGET_CHILD_UNSAFE_PATH",
            Self::ScratchCollision => "TARGET_CHILD_SCRATCH_COLLISION",
            Self::RequestTooLarge => "TARGET_CHILD_REQUEST_TOO_LARGE",
            Self::InvalidRequest => "TARGET_CHILD_INVALID_REQUEST",
            Self::RequestWrite => "TARGET_CHILD_REQUEST_UNAVAILABLE",
            Self::Launch => "TARGET_CHILD_LAUNCH_FAILED",
            Self::Timeout => "TARGET_CHILD_TIMEOUT",
            Self::Termination => "TARGET_CHILD_TERMINATION_FAILED",
            Self::StatusTooLarge => "TARGET_CHILD_STATUS_TOO_LARGE",
            Self::InvalidStatus => "TARGET_CHILD_INVALID_STATUS",
            Self::ChildStatus { code } => code,
            Self::Exited => "TARGET_CHILD_EXITED",
            Self::NoResult => "TARGET_CHILD_NO_RESULT",
            Self::ResultTooLarge => "TARGET_CHILD_RESULT_TOO_LARGE",
            Self::InvalidResult => "TARGET_CHILD_INVALID_RESULT",
            Self::Cleanup => "TARGET_CHILD_CLEANUP_FAILED",
        }
    }

    pub fn class(&self) -> ErrorClass {
        match self {
            Self::RequestTooLarge | Self::InvalidRequest => ErrorClass::Input,
            Self::ChildStatus { code }
                if matches!(code.as_str(), "INVALID_TARGET" | "RESULT_TOO_LARGE") =>
            {
                ErrorClass::Integrity
            }
            Self::ChildStatus { code }
                if matches!(
                    code.as_str(),
                    "REQUEST_UNAVAILABLE" | "CHILD_INTERNAL_ERROR"
                ) =>
            {
                ErrorClass::Transient
            }
            Self::ChildStatus { .. } => ErrorClass::Input,
            Self::UnsafePath
            | Self::StatusTooLarge
            | Self::InvalidStatus
            | Self::ResultTooLarge
            | Self::InvalidResult => ErrorClass::Integrity,
            Self::ScratchCollision
            | Self::RequestWrite
            | Self::Launch
            | Self::Timeout
            | Self::Termination
            | Self::Exited
            | Self::NoResult
            | Self::Cleanup => ErrorClass::Transient,
        }
    }

    /// Safe to persist: it contains no paths, stderr, environment, or request data.
    pub fn safe_summary(&self) -> String {
        match self {
            Self::ChildStatus { code } => format!("target generator rejected the request ({code})"),
            _ => self.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildStatus {
    code: String,
    #[allow(dead_code)]
    summary: String,
}

/// Run one shipped target generator in a bounded, job-scoped child process.
pub async fn run_target_child(
    paths: &TargetChildPaths,
    job_id: Uuid,
    request: &TargetChildRequest,
    deadline: Duration,
) -> Result<TargetChildOutput, TargetChildError> {
    let operation_deadline = tokio::time::Instant::now()
        .checked_add(deadline)
        .ok_or(TargetChildError::Timeout)?;
    let cleanup_deadline = operation_deadline
        .checked_add(CLEANUP_GRACE)
        .ok_or(TargetChildError::Timeout)?;
    let validated = ValidatedPaths::new(paths)?;
    validate_request_numbers(request)?;
    if deadline.is_zero() {
        return Err(TargetChildError::Timeout);
    }
    let invocation_dir = create_invocation_directory(&validated.temp_root, job_id)?;
    let result = run_in_scratch(
        &validated,
        &invocation_dir,
        request,
        operation_deadline,
        cleanup_deadline,
    )
    .await;
    let cleanup = cleanup_scratch(invocation_dir, cleanup_deadline).await;
    if cleanup.is_err() && result.is_ok() {
        return Err(TargetChildError::Cleanup);
    }
    result
}

fn create_invocation_directory(
    temp_root: &Path,
    job_id: Uuid,
) -> Result<PathBuf, TargetChildError> {
    for _ in 0..CREATE_ATTEMPTS {
        let invocation_id = Uuid::new_v4();
        let path = temp_root.join(format!(
            "recommendation-{}-invocation-{}",
            job_id.simple(),
            invocation_id.simple()
        ));
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(TargetChildError::RequestWrite),
        }
    }
    Err(TargetChildError::ScratchCollision)
}

async fn cleanup_scratch(
    path: PathBuf,
    cleanup_deadline: tokio::time::Instant,
) -> Result<(), TargetChildError> {
    let task = tokio::task::spawn_blocking(move || fs::remove_dir_all(path));
    match tokio::time::timeout_at(cleanup_deadline, task).await {
        Ok(Ok(Ok(()))) => Ok(()),
        _ => Err(TargetChildError::Cleanup),
    }
}

fn validate_request_numbers(request: &TargetChildRequest) -> Result<(), TargetChildError> {
    if request
        .factors
        .values()
        .flat_map(BTreeMap::values)
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(TargetChildError::InvalidRequest);
    }
    Ok(())
}

struct ValidatedPaths {
    uv_bin: PathBuf,
    repo_root: PathBuf,
    temp_root: PathBuf,
}

impl ValidatedPaths {
    fn new(paths: &TargetChildPaths) -> Result<Self, TargetChildError> {
        if !paths.uv_bin.is_absolute()
            || !paths.repo_root.is_absolute()
            || !paths.temp_root.is_absolute()
        {
            return Err(TargetChildError::UnsafePath);
        }
        let uv_bin = canonical_regular_file(&paths.uv_bin)?;
        let repo_root = canonical_directory(&paths.repo_root, false)?;
        let temp_root = canonical_directory(&paths.temp_root, true)?;
        let nt = repo_root.join("nt");
        if !nt.join("pyproject.toml").is_file() || !nt.join("strategies").is_dir() {
            return Err(TargetChildError::UnsafePath);
        }
        Ok(Self {
            uv_bin,
            repo_root,
            temp_root,
        })
    }
}

fn canonical_regular_file(path: &Path) -> Result<PathBuf, TargetChildError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| TargetChildError::UnsafePath)?;
    if !canonical.is_file() {
        return Err(TargetChildError::UnsafePath);
    }
    Ok(canonical)
}

fn canonical_directory(path: &Path, reject_symlink: bool) -> Result<PathBuf, TargetChildError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| TargetChildError::UnsafePath)?;
    if reject_symlink && metadata.file_type().is_symlink() {
        return Err(TargetChildError::UnsafePath);
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| TargetChildError::UnsafePath)?;
    if !canonical.is_dir() {
        return Err(TargetChildError::UnsafePath);
    }
    Ok(canonical)
}

async fn run_in_scratch(
    paths: &ValidatedPaths,
    invocation_dir: &Path,
    request: &TargetChildRequest,
    operation_deadline: tokio::time::Instant,
    cleanup_deadline: tokio::time::Instant,
) -> Result<TargetChildOutput, TargetChildError> {
    let request_path = invocation_dir.join("request.json");
    let result_path = invocation_dir.join("result.json");
    let status_path = invocation_dir.join("status.json");
    stage_request(&request_path, request)?;
    if tokio::time::Instant::now() >= operation_deadline {
        return Err(TargetChildError::Timeout);
    }

    let mut command = Command::new(&paths.uv_bin);
    command
        .arg("run")
        .arg("--project")
        .arg("nt")
        .arg("--no-sync")
        .arg("python")
        .arg("-m")
        .arg("strategies.recommendation_cli")
        .arg("--request")
        .arg(&request_path)
        .arg("--result")
        .arg(&result_path)
        .arg("--status")
        .arg(&status_path)
        .current_dir(&paths.repo_root)
        .env_clear()
        // This is the entire child environment. The platform variable on
        // Windows is needed by native process loading. The Windows temp path
        // is the canonical OS runtime directory; all other values are fixed.
        .env("PYTHONPATH", paths.repo_root.join("nt"))
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("UV_NO_CONFIG", "1")
        .env("UV_NO_PROGRESS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if cfg!(windows) {
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        let runtime_temp = std::env::temp_dir()
            .canonicalize()
            .map_err(|_| TargetChildError::UnsafePath)?;
        if !runtime_temp.is_dir() {
            return Err(TargetChildError::UnsafePath);
        }
        command.env("TEMP", &runtime_temp).env("TMP", runtime_temp);
    }

    let mut child = command.spawn().map_err(|_| TargetChildError::Launch)?;
    let stderr = child.stderr.take().ok_or(TargetChildError::Launch)?;
    let mut stderr_reader = tokio::spawn(drain_bounded(stderr, MAX_STDERR_BYTES));
    let exit = match tokio::time::timeout_at(operation_deadline, child.wait()).await {
        Ok(Ok(exit)) => exit,
        Ok(Err(_)) => {
            abort_reader_bounded(&mut stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Exited);
        }
        Err(_) => {
            terminate_and_settle(&mut child, &mut stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Timeout);
        }
    };
    match tokio::time::timeout_at(operation_deadline, &mut stderr_reader).await {
        Ok(Ok(Ok(_stderr))) => {}
        Ok(Ok(Err(_))) | Ok(Err(_)) => return Err(TargetChildError::Exited),
        Err(_) => {
            abort_reader_bounded(&mut stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Timeout);
        }
    }

    if tokio::time::Instant::now() >= operation_deadline {
        return Err(TargetChildError::Timeout);
    }

    if status_path.exists() {
        let status = read_status(&status_path)?;
        return Err(TargetChildError::ChildStatus { code: status.code });
    }
    if !result_path.exists() {
        return Err(if exit.success() {
            TargetChildError::NoResult
        } else {
            TargetChildError::Exited
        });
    }
    if !exit.success() {
        return Err(TargetChildError::Exited);
    }
    read_result(&result_path)
}

async fn terminate_and_settle(
    child: &mut tokio::process::Child,
    stderr_reader: &mut tokio::task::JoinHandle<Result<Vec<u8>, std::io::Error>>,
    cleanup_deadline: tokio::time::Instant,
) -> Result<(), TargetChildError> {
    if child.start_kill().is_err() {
        stderr_reader.abort();
        let _ = tokio::time::timeout_at(cleanup_deadline, stderr_reader).await;
        return Err(TargetChildError::Termination);
    }
    let reaped = tokio::time::timeout_at(cleanup_deadline, child.wait()).await;
    abort_reader_bounded(stderr_reader, cleanup_deadline).await?;
    match reaped {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(TargetChildError::Termination),
    }
}

async fn abort_reader_bounded(
    stderr_reader: &mut tokio::task::JoinHandle<Result<Vec<u8>, std::io::Error>>,
    cleanup_deadline: tokio::time::Instant,
) -> Result<(), TargetChildError> {
    stderr_reader.abort();
    match tokio::time::timeout_at(cleanup_deadline, stderr_reader).await {
        Ok(_) => Ok(()),
        Err(_) => Err(TargetChildError::Termination),
    }
}

fn stage_request(path: &Path, request: &TargetChildRequest) -> Result<(), TargetChildError> {
    let bytes = serde_json::to_vec(request).map_err(|_| TargetChildError::RequestWrite)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(TargetChildError::RequestTooLarge);
    }
    let temporary = path.with_extension("json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| TargetChildError::RequestWrite)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| TargetChildError::RequestWrite)?;
    fs::rename(&temporary, path).map_err(|_| TargetChildError::RequestWrite)
}

async fn drain_bounded(
    mut stderr: tokio::process::ChildStderr,
    limit: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let mut kept = Vec::with_capacity(limit);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stderr.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        if kept.len() < limit {
            let remaining = limit - kept.len();
            kept.extend_from_slice(&chunk[..read.min(remaining)]);
        }
    }
    Ok(kept)
}

fn read_status(path: &Path) -> Result<ChildStatus, TargetChildError> {
    if fs::metadata(path)
        .map_err(|_| TargetChildError::InvalidStatus)?
        .len()
        > MAX_STATUS_BYTES
    {
        return Err(TargetChildError::StatusTooLarge);
    }
    let bytes = fs::read(path).map_err(|_| TargetChildError::InvalidStatus)?;
    let status: ChildStatus =
        serde_json::from_slice(&bytes).map_err(|_| TargetChildError::InvalidStatus)?;
    if !is_stable_child_code(&status.code) || status.summary.len() > 512 {
        return Err(TargetChildError::InvalidStatus);
    }
    Ok(status)
}

fn is_stable_child_code(code: &str) -> bool {
    matches!(
        code,
        "UNKNOWN_STRATEGY"
            | "INVALID_REQUEST"
            | "UNSUPPORTED_VERSION"
            | "REQUEST_TOO_LARGE"
            | "INVALID_JSON"
            | "REQUEST_UNAVAILABLE"
            | "TARGET_GENERATION_FAILED"
            | "INVALID_TARGET"
            | "RESULT_TOO_LARGE"
            | "CHILD_INTERNAL_ERROR"
    )
}

fn read_result(path: &Path) -> Result<TargetChildOutput, TargetChildError> {
    if fs::metadata(path)
        .map_err(|_| TargetChildError::InvalidResult)?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err(TargetChildError::ResultTooLarge);
    }
    let bytes = fs::read(path).map_err(|_| TargetChildError::InvalidResult)?;
    serde_json::from_slice(&bytes).map_err(|_| TargetChildError::InvalidResult)
}

fn finite_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f64::deserialize(deserializer)?;
    if !value.is_finite() {
        return Err(serde::de::Error::custom("number must be finite"));
    }
    Ok(value)
}

fn finite_factor_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = BTreeMap::<String, f64>::deserialize(deserializer)?;
    if values.values().any(|value| !value.is_finite()) {
        return Err(serde::de::Error::custom("factor values must be finite"));
    }
    Ok(values)
}
