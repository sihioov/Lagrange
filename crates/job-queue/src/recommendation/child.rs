//! Bounded process boundary for the shipped Python target generators.
//!
//! Generator code is trusted repository code selected from a literal allow-list;
//! request data cannot choose an import, executable, or process command, and the
//! shipped generators do not launch subprocesses. This boundary contains ordinary
//! descendants, but it is not an OS-hard sandbox against compromised trusted code
//! deliberately escaping its process group (for example with `setsid`). Deployment
//! isolation such as cgroups or namespaces is separate defense in depth.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
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
    #[error("target child owner task failed")]
    OwnerTask,
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
            Self::OwnerTask => "TARGET_CHILD_OWNER_TASK_FAILED",
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
                if matches!(
                    code.as_str(),
                    "INVALID_TARGET" | "RESULT_TOO_LARGE" | "TARGET_GENERATOR_INTERNAL"
                ) =>
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
            | Self::OwnerTask
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
    let owned_request = request.clone();
    let invocation_dir = create_invocation_directory(&validated.temp_root, job_id)?;
    // There must be no cancellation point between acquiring this directory
    // and handing it to its owner. Dropping the caller detaches this task;
    // it does not cancel child termination/reaping or scratch cleanup.
    let owner = tokio::spawn(own_invocation(
        validated,
        invocation_dir,
        owned_request,
        operation_deadline,
        cleanup_deadline,
    ));
    owner.await.map_err(|_| TargetChildError::OwnerTask)?
}

async fn own_invocation(
    paths: ValidatedPaths,
    invocation_dir: PathBuf,
    request: TargetChildRequest,
    operation_deadline: tokio::time::Instant,
    cleanup_deadline: tokio::time::Instant,
) -> Result<TargetChildOutput, TargetChildError> {
    let lifecycle_dir = invocation_dir.clone();
    let lifecycle = tokio::spawn(async move {
        run_in_scratch(
            &paths,
            &lifecycle_dir,
            &request,
            operation_deadline,
            cleanup_deadline,
        )
        .await
    });
    let result = lifecycle.await.unwrap_or(Err(TargetChildError::OwnerTask));
    let cleanup = cleanup_scratch(invocation_dir, cleanup_deadline).await;
    if cleanup.is_err() {
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

#[derive(Clone)]
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

    let mut process_tree = ProcessTree::prepare()?;

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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command
            .as_std_mut()
            .creation_flags(windows_child_creation_flags());
    }
    #[cfg(windows)]
    {
        command.env("SystemRoot", trusted_windows_root()?);
        let runtime_temp = std::env::temp_dir()
            .canonicalize()
            .map_err(|_| TargetChildError::UnsafePath)?;
        if !runtime_temp.is_dir() {
            return Err(TargetChildError::UnsafePath);
        }
        command.env("TEMP", &runtime_temp).env("TMP", runtime_temp);
    }

    let mut child = command.spawn().map_err(|_| TargetChildError::Launch)?;
    if process_tree.attach(&child).is_err() || process_tree.start(&child).is_err() {
        let _ = process_tree.terminate();
        let _ = settle_direct_child(&mut child, cleanup_deadline).await;
        return Err(TargetChildError::Termination);
    }
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let tree_terminated = process_tree.terminate().is_ok();
            let direct_reaped = settle_direct_child(&mut child, cleanup_deadline).await;
            if direct_reaped {
                process_tree.disarm_reuse_sensitive_identity();
            }
            let tree_confirmed = tree_terminated
                && confirm_process_tree_after_reap(&process_tree, cleanup_deadline).await;
            if tree_confirmed && direct_reaped {
                process_tree.disarm_reuse_sensitive_identity();
                return Err(TargetChildError::Launch);
            }
            return Err(TargetChildError::Termination);
        }
    };
    let mut stderr_reader = tokio::spawn(drain_bounded(stderr, MAX_STDERR_BYTES));
    let exit = wait_for_child_boundary(
        &mut process_tree,
        &mut child,
        &mut stderr_reader,
        operation_deadline,
        cleanup_deadline,
    )
    .await?;

    if tokio::time::Instant::now() >= operation_deadline {
        return Err(TargetChildError::Timeout);
    }

    if let Some(status) = read_status(&status_path, operation_deadline).await? {
        return Err(TargetChildError::ChildStatus { code: status.code });
    }
    if !exit.success() {
        return Err(TargetChildError::Exited);
    }
    read_result(&result_path, operation_deadline)
        .await?
        .ok_or(TargetChildError::NoResult)
}

#[cfg(windows)]
async fn wait_for_child_boundary(
    process_tree: &mut ProcessTree,
    child: &mut tokio::process::Child,
    stderr_reader: &mut tokio::task::JoinHandle<Result<Vec<u8>, std::io::Error>>,
    operation_deadline: tokio::time::Instant,
    cleanup_deadline: tokio::time::Instant,
) -> Result<std::process::ExitStatus, TargetChildError> {
    let exit = match tokio::time::timeout_at(operation_deadline, child.wait()).await {
        Ok(Ok(exit)) => exit,
        Ok(Err(_)) => {
            terminate_and_settle(process_tree, child, stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Exited);
        }
        Err(_) => {
            terminate_and_settle(process_tree, child, stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Timeout);
        }
    };
    match tokio::time::timeout_at(operation_deadline, &mut *stderr_reader).await {
        Ok(Ok(Ok(_stderr))) => {}
        Ok(Ok(Err(_))) | Ok(Err(_)) => {
            let tree_terminated = process_tree.terminate().is_ok();
            let direct_reaped = settle_direct_child(child, cleanup_deadline).await;
            if !tree_terminated || !direct_reaped {
                return Err(TargetChildError::Termination);
            }
            if !confirm_process_tree_empty(process_tree, cleanup_deadline).await {
                return Err(TargetChildError::Termination);
            }
            process_tree.disarm_reuse_sensitive_identity();
            return Err(TargetChildError::Exited);
        }
        Err(_) => {
            terminate_and_settle(process_tree, child, stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Timeout);
        }
    }

    // EOF only proves no descendant inherited this pipe. A descendant may
    // have closed or redirected stderr, so explicitly empty the retained Job.
    if process_tree.terminate().is_err()
        || !confirm_process_tree_empty(process_tree, operation_deadline).await
    {
        return Err(TargetChildError::Termination);
    }
    process_tree.disarm_reuse_sensitive_identity();
    Ok(exit)
}

#[cfg(unix)]
async fn wait_for_child_boundary(
    process_tree: &mut ProcessTree,
    child: &mut tokio::process::Child,
    stderr_reader: &mut tokio::task::JoinHandle<Result<Vec<u8>, std::io::Error>>,
    operation_deadline: tokio::time::Instant,
    cleanup_deadline: tokio::time::Instant,
) -> Result<std::process::ExitStatus, TargetChildError> {
    let pid = child.id().ok_or(TargetChildError::Termination)?;
    match observe_unix_child_exit(pid, operation_deadline).await {
        Ok(()) => {}
        Err(TargetChildError::Timeout) => {
            terminate_and_settle(process_tree, child, stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Timeout);
        }
        Err(_) => {
            terminate_and_settle(process_tree, child, stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Exited);
        }
    }

    // The exited group leader is intentionally still waitable here, so its
    // PID/PGID cannot be reused while this signal targets ordinary descendants.
    let tree_terminated = process_tree.terminate().is_ok();
    let exit = match tokio::time::timeout_at(cleanup_deadline, child.wait()).await {
        Ok(Ok(exit)) => {
            process_tree.disarm_reuse_sensitive_identity();
            exit
        }
        Ok(Err(_)) => {
            // The wait operation may have consumed OS state before reporting
            // an error. Never signal this numeric PGID again.
            process_tree.disarm_reuse_sensitive_identity();
            abort_reader_bounded(stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Termination);
        }
        Err(_) => {
            abort_reader_bounded(stderr_reader, cleanup_deadline).await?;
            return Err(TargetChildError::Termination);
        }
    };
    if !tree_terminated {
        abort_reader_bounded(stderr_reader, cleanup_deadline).await?;
        return Err(TargetChildError::Termination);
    }

    match tokio::time::timeout_at(operation_deadline, &mut *stderr_reader).await {
        Ok(Ok(Ok(_stderr))) => Ok(exit),
        Ok(Ok(Err(_))) | Ok(Err(_)) => Err(TargetChildError::Exited),
        Err(_) => {
            abort_reader_bounded(stderr_reader, cleanup_deadline).await?;
            Err(TargetChildError::Timeout)
        }
    }
}

async fn terminate_and_settle(
    process_tree: &mut ProcessTree,
    child: &mut tokio::process::Child,
    stderr_reader: &mut tokio::task::JoinHandle<Result<Vec<u8>, std::io::Error>>,
    cleanup_deadline: tokio::time::Instant,
) -> Result<(), TargetChildError> {
    let tree_terminated = process_tree.terminate().is_ok();
    let direct_reaped = settle_direct_child(child, cleanup_deadline).await;
    if direct_reaped {
        process_tree.disarm_reuse_sensitive_identity();
    }
    let reader_settled = abort_reader_bounded(stderr_reader, cleanup_deadline)
        .await
        .is_ok();
    let tree_confirmed =
        tree_terminated && confirm_process_tree_after_reap(process_tree, cleanup_deadline).await;
    if tree_confirmed && direct_reaped && reader_settled {
        process_tree.disarm_reuse_sensitive_identity();
        Ok(())
    } else {
        Err(TargetChildError::Termination)
    }
}

#[cfg(windows)]
async fn confirm_process_tree_empty(
    process_tree: &ProcessTree,
    deadline: tokio::time::Instant,
) -> bool {
    loop {
        if process_tree.is_empty() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(10)),
        )
        .await;
    }
}

#[cfg(windows)]
async fn confirm_process_tree_after_reap(
    process_tree: &ProcessTree,
    deadline: tokio::time::Instant,
) -> bool {
    confirm_process_tree_empty(process_tree, deadline).await
}

#[cfg(unix)]
async fn confirm_process_tree_after_reap(
    _process_tree: &ProcessTree,
    _deadline: tokio::time::Instant,
) -> bool {
    // The PGID was signaled while its leader was still unreaped. Any probe
    // after reap could target a recycled numeric identity, so none is made.
    true
}

#[cfg(unix)]
async fn observe_unix_child_exit(
    pid: u32,
    deadline: tokio::time::Instant,
) -> Result<(), TargetChildError> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| TargetChildError::Termination)?;
    loop {
        // SAFETY: waitid writes a siginfo_t and WNOWAIT explicitly preserves
        // the child as waitable; WNOHANG keeps each observation non-blocking.
        let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        let waited = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &raw mut information,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if waited == 0 {
            // SAFETY: waitid initialized `information`; si_pid is zero for the
            // WNOHANG no-state-change case and exact for P_PID otherwise.
            if unsafe { information.si_pid() } == pid {
                return Ok(());
            }
        } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return Err(TargetChildError::Termination);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(TargetChildError::Timeout);
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(10)),
        )
        .await;
    }
}

async fn settle_direct_child(
    child: &mut tokio::process::Child,
    cleanup_deadline: tokio::time::Instant,
) -> bool {
    let already_exited = matches!(child.try_wait(), Ok(Some(_)));
    if !already_exited {
        // Even if this fails, still perform the bounded wait below. The child
        // may have exited between `try_wait` and `start_kill`.
        let _ = child.start_kill();
    }
    matches!(
        tokio::time::timeout_at(cleanup_deadline, child.wait()).await,
        Ok(Ok(_))
    )
}

struct TerminationArm {
    armed: bool,
}

impl TerminationArm {
    const fn new_disarmed() -> Self {
        Self { armed: false }
    }

    fn arm(&mut self) {
        self.armed = true;
    }

    #[cfg(unix)]
    fn disarm(&mut self) {
        self.armed = false;
    }

    const fn is_armed(&self) -> bool {
        self.armed
    }
}

#[cfg(unix)]
struct ProcessTree {
    process_group: Option<i32>,
    arm: TerminationArm,
}

#[cfg(unix)]
impl ProcessTree {
    fn prepare() -> Result<Self, TargetChildError> {
        Ok(Self {
            process_group: None,
            arm: TerminationArm::new_disarmed(),
        })
    }

    fn attach(&mut self, child: &tokio::process::Child) -> Result<(), TargetChildError> {
        let pid = child.id().ok_or(TargetChildError::Termination)?;
        self.process_group = Some(i32::try_from(pid).map_err(|_| TargetChildError::Termination)?);
        self.arm.arm();
        Ok(())
    }

    fn start(&self, _child: &tokio::process::Child) -> Result<(), TargetChildError> {
        Ok(())
    }

    fn terminate(&self) -> Result<(), TargetChildError> {
        if !self.arm.is_armed() {
            return Ok(());
        }
        let process_group = self.process_group.ok_or(TargetChildError::Termination)?;
        // SAFETY: the spawned `uv` process is placed in its own process group
        // before spawn. A negative, checked child PID targets only that group.
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result == 0 {
            Ok(())
        } else {
            Err(TargetChildError::Termination)
        }
    }

    fn disarm_reuse_sensitive_identity(&mut self) {
        self.arm.disarm();
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if self.arm.is_armed() {
            let _ = self.terminate();
        }
    }
}

#[cfg(windows)]
const fn windows_child_creation_flags() -> u32 {
    windows_sys::Win32::System::Threading::CREATE_SUSPENDED
}

#[cfg(windows)]
fn resume_suspended_child(child: &tokio::process::Child) -> Result<(), TargetChildError> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessIdOfThread, OpenThread, ResumeThread, THREAD_QUERY_LIMITED_INFORMATION,
        THREAD_SUSPEND_RESUME,
    };

    let pid = child.id().ok_or(TargetChildError::Termination)?;
    let entry_size = u32::try_from(std::mem::size_of::<THREADENTRY32>())
        .map_err(|_| TargetChildError::Termination)?;
    // SAFETY: this takes a read-only system thread snapshot and returns a new
    // owned handle, closed on every path below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(TargetChildError::Termination);
    }

    let mut entry = THREADENTRY32 {
        dwSize: entry_size,
        ..THREADENTRY32::default()
    };
    // SAFETY: `snapshot` is live and `entry` points to a correctly sized,
    // initialized THREADENTRY32.
    let mut has_entry = unsafe { Thread32First(snapshot, &raw mut entry) } != 0;
    let mut resumed = false;
    while has_entry {
        if entry.th32OwnerProcessID == pid {
            // The CREATE_SUSPENDED primary thread has never executed, so it
            // cannot have created another thread for this new process yet.
            // SAFETY: the enumerated thread ID belongs to the exact suspended
            // child PID. The non-inheritable handle is closed below.
            let thread = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                    0,
                    entry.th32ThreadID,
                )
            };
            if !thread.is_null() {
                // Revalidate ownership through the opened handle so a stale or
                // mismatched enumeration entry can never be resumed.
                // SAFETY: `thread` is a live thread handle.
                let owner = unsafe { GetProcessIdOfThread(thread) };
                // CREATE_SUSPENDED gives the primary thread a suspend count of
                // exactly one; only that state is accepted as safely resumed.
                // SAFETY: the validated handle has THREAD_SUSPEND_RESUME.
                let previous_count = if owner == pid {
                    unsafe { ResumeThread(thread) }
                } else {
                    u32::MAX
                };
                // SAFETY: `thread` is the owned handle opened above.
                unsafe { CloseHandle(thread) };
                resumed = owner == pid && previous_count == 1;
            }
            break;
        }
        // SAFETY: `snapshot` and `entry` remain valid for enumeration.
        has_entry = unsafe { Thread32Next(snapshot, &raw mut entry) } != 0;
    }
    // SAFETY: `snapshot` is the owned handle created above.
    unsafe { CloseHandle(snapshot) };

    if resumed {
        Ok(())
    } else {
        Err(TargetChildError::Termination)
    }
}

#[cfg(windows)]
fn trusted_windows_root() -> Result<PathBuf, TargetChildError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: `buffer` is writable for the advertised number of u16 values.
    let length = unsafe {
        GetWindowsDirectoryW(
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len()).map_err(|_| TargetChildError::UnsafePath)?,
        )
    };
    let length = usize::try_from(length).map_err(|_| TargetChildError::UnsafePath)?;
    if length == 0 || length >= buffer.len() {
        return Err(TargetChildError::UnsafePath);
    }
    let root = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    if !root.is_absolute() || !root.is_dir() {
        return Err(TargetChildError::UnsafePath);
    }
    Ok(root)
}

#[cfg(windows)]
struct ProcessTree {
    job: isize,
    arm: TerminationArm,
}

#[cfg(windows)]
impl ProcessTree {
    fn prepare() -> Result<Self, TargetChildError> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let limits_size =
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .map_err(|_| TargetChildError::Termination)?;
        // SAFETY: null security attributes/name request an unnamed job with
        // default security. The returned owned handle is closed in `Drop`.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(TargetChildError::Termination);
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` has the exact layout and byte length required by
        // `JobObjectExtendedLimitInformation`; `job` is live and owned here.
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                limits_size,
            )
        };
        if configured == 0 {
            // SAFETY: `job` is a valid owned handle and has not been closed.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(job) };
            return Err(TargetChildError::Termination);
        }
        Ok(Self {
            job: job as isize,
            arm: TerminationArm::new_disarmed(),
        })
    }

    fn attach(&mut self, child: &tokio::process::Child) -> Result<(), TargetChildError> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let process = child.raw_handle().ok_or(TargetChildError::Termination)?;
        // SAFETY: both handles are live. This runs synchronously immediately
        // after spawn, before any await can observe child completion.
        let assigned = unsafe { AssignProcessToJobObject(self.handle(), process.cast()) };
        if assigned == 0 {
            Err(TargetChildError::Termination)
        } else {
            self.arm.arm();
            Ok(())
        }
    }

    fn start(&self, child: &tokio::process::Child) -> Result<(), TargetChildError> {
        resume_suspended_child(child)
    }

    fn terminate(&self) -> Result<(), TargetChildError> {
        if !self.arm.is_armed() {
            return Ok(());
        }
        // SAFETY: the job handle is owned by `self` and remains live until
        // `Drop`, including after the direct child handle has been reaped.
        let terminated =
            unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle(), 1) };
        if terminated == 0 {
            Err(TargetChildError::Termination)
        } else {
            Ok(())
        }
    }

    fn is_empty(&self) -> bool {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };

        if !self.arm.is_armed() {
            return true;
        }
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let Ok(size) = u32::try_from(std::mem::size_of_val(&accounting)) else {
            return false;
        };
        // SAFETY: `accounting` is the exact query type and `self.handle()` is
        // owned and live for the duration of this synchronous call.
        let queried = unsafe {
            QueryInformationJobObject(
                self.handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size,
                std::ptr::null_mut(),
            )
        };
        queried != 0 && accounting.ActiveProcesses == 0
    }

    fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.job as windows_sys::Win32::Foundation::HANDLE
    }

    fn disarm_reuse_sensitive_identity(&mut self) {
        // A Job Object handle cannot be retargeted to unrelated future
        // processes, so retaining KILL_ON_JOB_CLOSE remains the safer policy.
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // KILL_ON_JOB_CLOSE makes this a final best-effort tree termination
        // even when the surrounding future unwinds unexpectedly.
        // SAFETY: this is the one close of the handle owned by `self`.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle()) };
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

async fn read_status(
    path: &Path,
    deadline: tokio::time::Instant,
) -> Result<Option<ChildStatus>, TargetChildError> {
    let Some(bytes) =
        read_bounded_regular(path, MAX_STATUS_BYTES, OutputFileKind::Status, deadline).await?
    else {
        return Ok(None);
    };
    let status: ChildStatus =
        serde_json::from_slice(&bytes).map_err(|_| TargetChildError::InvalidStatus)?;
    if !is_stable_child_code(&status.code) || status.summary.len() > 512 {
        return Err(TargetChildError::InvalidStatus);
    }
    Ok(Some(status))
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
            | "TARGET_GENERATOR_INTERNAL"
            | "INVALID_TARGET"
            | "RESULT_TOO_LARGE"
            | "CHILD_INTERNAL_ERROR"
    )
}

async fn read_result(
    path: &Path,
    deadline: tokio::time::Instant,
) -> Result<Option<TargetChildOutput>, TargetChildError> {
    let Some(bytes) =
        read_bounded_regular(path, MAX_RESULT_BYTES, OutputFileKind::Result, deadline).await?
    else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| TargetChildError::InvalidResult)
}

#[derive(Clone, Copy)]
enum OutputFileKind {
    Status,
    Result,
}

impl OutputFileKind {
    const fn invalid(self) -> TargetChildError {
        match self {
            Self::Status => TargetChildError::InvalidStatus,
            Self::Result => TargetChildError::InvalidResult,
        }
    }

    const fn too_large(self) -> TargetChildError {
        match self {
            Self::Status => TargetChildError::StatusTooLarge,
            Self::Result => TargetChildError::ResultTooLarge,
        }
    }
}

async fn read_bounded_regular(
    path: &Path,
    limit: u64,
    kind: OutputFileKind,
    deadline: tokio::time::Instant,
) -> Result<Option<Vec<u8>>, TargetChildError> {
    let owned_path = path.to_path_buf();
    let task = tokio::task::spawn_blocking(move || {
        read_bounded_regular_blocking(&owned_path, limit, kind)
    });
    match tokio::time::timeout_at(deadline, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(kind.invalid()),
        Err(_) => Err(TargetChildError::Timeout),
    }
}

fn read_bounded_regular_blocking(
    path: &Path,
    limit: u64,
    kind: OutputFileKind,
) -> Result<Option<Vec<u8>>, TargetChildError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(kind.invalid()),
    };
    let metadata = file.metadata().map_err(|_| kind.invalid())?;
    if !metadata.is_file() {
        return Err(kind.invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(kind.invalid());
        }
    }
    if metadata.len() > limit {
        return Err(kind.too_large());
    }

    let read_limit = limit.checked_add(1).ok_or_else(|| kind.too_large())?;
    let initial_capacity = usize::try_from(read_limit.min(8192)).map_err(|_| kind.too_large())?;
    let mut bytes = Vec::with_capacity(initial_capacity);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|_| kind.invalid())?;
    if u64::try_from(bytes.len()).map_err(|_| kind.too_large())? > limit {
        return Err(kind.too_large());
    }
    Ok(Some(bytes))
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

#[cfg(test)]
mod lifecycle_state_tests {
    use super::*;

    #[test]
    fn concurrent_output_growth_never_returns_more_than_limit() {
        use std::sync::{Arc, Barrier};

        let root = tempfile::tempdir().expect("temp output root");
        let path = root.path().join("growing-result.json");
        fs::write(&path, b"seed").expect("seed result");
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = Arc::clone(&barrier);
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            let mut file = OpenOptions::new()
                .append(true)
                .open(writer_path)
                .expect("open growing result");
            writer_barrier.wait();
            for _ in 0..512 {
                file.write_all(&[b'x'; 4096]).expect("grow result");
                std::thread::yield_now();
            }
        });
        barrier.wait();

        let outcome = read_bounded_regular_blocking(&path, 64 * 1024, OutputFileKind::Result);
        writer.join().expect("writer joins");

        match outcome {
            Ok(Some(bytes)) => assert!(bytes.len() <= 64 * 1024),
            Err(TargetChildError::ResultTooLarge) => {}
            other => panic!("unexpected bounded read outcome: {other:?}"),
        }
    }

    #[test]
    fn termination_arm_starts_disarmed_and_can_be_armed() {
        let mut arm = TerminationArm::new_disarmed();
        assert!(!arm.is_armed());

        arm.arm();
        assert!(arm.is_armed());
    }

    #[cfg(unix)]
    #[test]
    fn unix_termination_arm_requires_an_explicit_clean_disarm() {
        let mut arm = TerminationArm::new_disarmed();
        arm.arm();

        arm.disarm();
        assert!(!arm.is_armed());
    }

    #[cfg(unix)]
    #[test]
    fn unix_clean_disarm_prevents_drop_from_signaling_the_old_process_group() {
        use std::os::unix::process::CommandExt;

        let mut sleeper = std::process::Command::new("/bin/sleep");
        sleeper.arg("5").process_group(0);
        let mut sleeper = sleeper.spawn().expect("spawn isolated sleeper");
        let process_group = i32::try_from(sleeper.id()).expect("PID fits i32");
        let mut tree = ProcessTree {
            process_group: Some(process_group),
            arm: TerminationArm { armed: true },
        };

        tree.disarm_reuse_sensitive_identity();
        drop(tree);

        let survived_drop = sleeper.try_wait().expect("query sleeper").is_none();
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        assert!(
            survived_drop,
            "a clean disarm must make ProcessTree::drop a no-op"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_exit_is_observed_and_group_terminated_before_leader_reap() {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("/bin/true");
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().expect("spawn group leader");
        let pid = child.id().expect("leader pid");
        let mut tree = ProcessTree {
            process_group: Some(i32::try_from(pid).expect("PID fits i32")),
            arm: TerminationArm { armed: true },
        };

        observe_unix_child_exit(pid, tokio::time::Instant::now() + Duration::from_secs(2))
            .await
            .expect("observe without reaping");
        tree.terminate()
            .expect("group identity remains valid before reap");
        let exit = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .expect("bounded reap")
            .expect("reap leader");
        tree.disarm_reuse_sensitive_identity();

        assert!(exit.success());
    }

    #[cfg(windows)]
    #[test]
    fn windows_children_are_created_suspended_before_job_assignment() {
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        assert_eq!(
            windows_child_creation_flags() & CREATE_SUSPENDED,
            CREATE_SUSPENDED
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_root_is_derived_from_the_os_not_the_parent_environment() {
        let root = trusted_windows_root().expect("trusted Windows root");
        assert!(root.is_absolute());
        assert!(root.is_dir());
    }
}
