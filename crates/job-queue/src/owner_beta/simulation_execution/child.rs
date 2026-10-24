//! Bounded child protocol and owned process-tree lifecycle.

use std::{
    ffi::OsString,
    path::PathBuf,
    process::ExitStatus,
    sync::{Arc, atomic::AtomicU8},
    time::Duration,
};

#[cfg(target_os = "linux")]
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    thread,
    time::Instant,
};

#[cfg(any(target_os = "linux", test))]
use std::io::{self, Read};

use serde::{Deserialize, Serialize};

use crate::owner_beta::simulation::parse_strict_json;

#[cfg(target_os = "linux")]
use crate::owner_beta::simulation::{
    SimulationRunIdentity, SimulationRunRequest, canonical_json,
    run_approved_historical_price_only_simulation,
};

use super::runner::SimulationWorkerError;

#[cfg(target_os = "linux")]
use super::SimulationJobInput;

#[cfg(target_os = "linux")]
use super::provenance::TrustedSimulationRuntime;

pub(super) const MAX_CHILD_REQUEST_BYTES: usize = 256 * 1024;
pub(super) const MAX_CHILD_STDOUT_BYTES: usize = 64 * 1024 * 1024;
pub(super) const CHILD_DEADLINE: Duration = Duration::from_secs(1_200);
#[cfg(target_os = "linux")]
const CHILD_TERM_GRACE: Duration = Duration::from_secs(5);
#[cfg(target_os = "linux")]
const CHILD_KILL_CONFIRMATION: Duration = Duration::from_secs(5);

pub(super) const CONTROL_RUNNING: u8 = 0;
pub(super) const CONTROL_CANCEL: u8 = 1;
#[cfg(target_os = "linux")]
const CONTROL_OUTPUT_TOO_LARGE: u8 = 2;

#[derive(Clone)]
pub(super) struct ChildCommand {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub request: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopReason {
    TimedOut,
    Canceled,
    OutputTooLarge,
}

pub(super) struct BoundedChildResult {
    #[cfg(test)]
    pub pid: u32,
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stop_reason: Option<StopReason>,
    #[cfg(test)]
    pub kill_escalated: bool,
    #[cfg(test)]
    pub exited_unreaped_descendants: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildStartError {
    InvalidRequest,
    SpawnFailed,
    IsolationUnavailable,
    PipeUnavailable,
    CleanupUnconfirmed,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct ProcessLimits {
    deadline: Duration,
    term_grace: Duration,
    max_stdout_bytes: usize,
}

#[cfg(target_os = "linux")]
impl ProcessLimits {
    const fn production() -> Self {
        Self {
            deadline: CHILD_DEADLINE,
            term_grace: CHILD_TERM_GRACE,
            max_stdout_bytes: MAX_CHILD_STDOUT_BYTES,
        }
    }
}

/// Runs one absolute executable without a shell or inherited environment.
/// The caller moves its process-slot permit into this blocking operation so
/// it remains held until the leader, descendants, and both pipe tasks settle.
pub(super) fn run_bounded_child(
    command: ChildCommand,
    cancel: Arc<AtomicU8>,
) -> Result<BoundedChildResult, ChildStartError> {
    #[cfg(target_os = "linux")]
    {
        run_with_limits(command, cancel, ProcessLimits::production())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (command, cancel);
        Err(ChildStartError::IsolationUnavailable)
    }
}

#[cfg(target_os = "linux")]
fn run_with_limits(
    command_spec: ChildCommand,
    cancel: Arc<AtomicU8>,
    limits: ProcessLimits,
) -> Result<BoundedChildResult, ChildStartError> {
    if !command_spec.executable.is_absolute()
        || command_spec.request.len() > MAX_CHILD_REQUEST_BYTES
        || limits.max_stdout_bytes == usize::MAX
    {
        return Err(ChildStartError::InvalidRequest);
    }
    if cancel.load(Ordering::Acquire) != CONTROL_RUNNING {
        return Err(ChildStartError::PipeUnavailable);
    }

    let mut command = Command::new(&command_spec.executable);
    command
        .args(&command_spec.args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    use std::os::unix::process::CommandExt;
    command.process_group(0);

    let mut child = command.spawn().map_err(|_| ChildStartError::SpawnFailed)?;
    let child_pid = child.id();

    let (stdin, stdout) = match (child.stdin.take(), child.stdout.take()) {
        (Some(stdin), Some(stdout)) => (stdin, stdout),
        (stdin, stdout) => {
            drop(stdin);
            drop(stdout);
            return Err(cleanup_startup_failure(child, child_pid, limits.term_grace));
        }
    };
    if set_nonblocking(&stdin).is_err() || set_nonblocking(&stdout).is_err() {
        drop(stdin);
        drop(stdout);
        return Err(cleanup_startup_failure(child, child_pid, limits.term_grace));
    }

    let writer_cancel = cancel.clone();
    let writer = match thread::Builder::new()
        .name("simulation-child-stdin".to_owned())
        .spawn(move || write_request(stdin, &command_spec.request, &writer_cancel))
    {
        Ok(writer) => writer,
        Err(_) => {
            drop(stdout);
            return Err(cleanup_startup_failure(child, child_pid, limits.term_grace));
        }
    };

    let reader_cancel = cancel.clone();
    let max_stdout = limits.max_stdout_bytes;
    let reader = match thread::Builder::new()
        .name("simulation-child-stdout".to_owned())
        .spawn(move || read_stdout(stdout, max_stdout, reader_cancel))
    {
        Ok(reader) => reader,
        Err(_) => {
            cancel.store(CONTROL_CANCEL, Ordering::Release);
            let cleanup = terminate_owned_group(&mut child, child_pid, None, limits.term_grace);
            let writer_join = writer.join();
            if cleanup.is_err() {
                handoff_unconfirmed_child(child);
                return Err(ChildStartError::CleanupUnconfirmed);
            }
            let _ = writer_join;
            return Err(ChildStartError::PipeUnavailable);
        }
    };

    let start = Instant::now();
    let (initial_status, mut stop_reason) = loop {
        let control = cancel.load(Ordering::Acquire);
        let stop_reason = match control {
            CONTROL_CANCEL => Some(StopReason::Canceled),
            CONTROL_OUTPUT_TOO_LARGE => Some(StopReason::OutputTooLarge),
            _ if start.elapsed() >= limits.deadline => Some(StopReason::TimedOut),
            _ => None,
        };
        if let Some(stop_reason) = stop_reason {
            if stop_reason == StopReason::TimedOut {
                let _ = cancel.compare_exchange(
                    CONTROL_RUNNING,
                    CONTROL_CANCEL,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            break (None, Some(stop_reason));
        }
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                cancel.store(CONTROL_CANCEL, Ordering::Release);
                break (None, Some(StopReason::Canceled));
            }
        }
    };

    let (status, _kill_escalated, _exited_unreaped_descendants) =
        match terminate_owned_group(&mut child, child_pid, initial_status, limits.term_grace) {
            Ok(cleanup) => cleanup,
            Err(()) => {
                cancel.store(CONTROL_CANCEL, Ordering::Release);
                let writer_join = writer.join();
                let reader_join = reader.join();
                let _ = (writer_join, reader_join);
                handoff_unconfirmed_child(child);
                return Err(ChildStartError::CleanupUnconfirmed);
            }
        };

    let writer_join = writer.join();
    let reader_join = reader.join();
    let (writer_result, reader_result) = match (writer_join, reader_join) {
        (Ok(writer_result), Ok(reader_result)) => (writer_result, reader_result),
        _ => return Err(ChildStartError::PipeUnavailable),
    };
    let stdout = reader_result.map_err(|_| ChildStartError::PipeUnavailable)?;

    stop_reason = reconcile_stop_reason(stop_reason, cancel.load(Ordering::Acquire));
    if stop_reason.is_none() && writer_result.is_err() {
        return Err(ChildStartError::PipeUnavailable);
    }
    Ok(BoundedChildResult {
        #[cfg(test)]
        pid: child_pid,
        status,
        stdout,
        stop_reason,
        #[cfg(test)]
        kill_escalated: _kill_escalated,
        #[cfg(test)]
        exited_unreaped_descendants: _exited_unreaped_descendants,
    })
}

#[cfg(target_os = "linux")]
fn reconcile_stop_reason(current: Option<StopReason>, control: u8) -> Option<StopReason> {
    current.or(match control {
        CONTROL_OUTPUT_TOO_LARGE => Some(StopReason::OutputTooLarge),
        CONTROL_CANCEL => Some(StopReason::Canceled),
        _ => None,
    })
}

#[cfg(target_os = "linux")]
fn cleanup_startup_failure(
    mut child: std::process::Child,
    process_group: u32,
    term_grace: Duration,
) -> ChildStartError {
    if terminate_owned_group(&mut child, process_group, None, term_grace).is_ok() {
        ChildStartError::PipeUnavailable
    } else {
        handoff_unconfirmed_child(child);
        ChildStartError::CleanupUnconfirmed
    }
}

#[cfg(target_os = "linux")]
fn handoff_unconfirmed_child(child: std::process::Child) {
    use std::sync::Mutex;

    // Cleanup is bounded in the caller. If group state remains unknown, keep
    // ownership of the direct child in one reaper thread and let the caller
    // poison its one-child slot. A failed thread spawn leaks the handle rather
    // than dropping an unreaped child.
    let child = Arc::new(Mutex::new(Some(child)));
    let reaper_child = child.clone();
    match thread::Builder::new()
        .name("simulation-child-reaper".to_owned())
        .spawn(move || {
            let mut child_slot = reaper_child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(mut child) = child_slot.take() {
                let _ = child.wait();
            }
        }) {
        Ok(reaper) => drop(reaper),
        Err(_) => {
            let mut child_slot = child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(child) = child_slot.take() {
                std::mem::forget(child);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn set_nonblocking<T: std::os::fd::AsRawFd>(file: &T) -> io::Result<()> {
    // SAFETY: fcntl reads and updates flags on this owned pipe descriptor.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor remains owned by `file` for the duration of the call.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_request(mut stdin: impl Write, request: &[u8], cancel: &AtomicU8) -> io::Result<()> {
    let mut offset = 0;
    while offset < request.len() {
        if cancel.load(Ordering::Acquire) != CONTROL_RUNNING {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "child request stopped",
            ));
        }
        match stdin.write(&request[offset..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => offset += written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn read_stdout(
    mut stdout: impl Read,
    max_bytes: usize,
    cancel: Arc<AtomicU8>,
) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(max_bytes.min(1024 * 1024));
    let mut buffer = [0u8; 8192];
    loop {
        if cancel.load(Ordering::Acquire) != CONTROL_RUNNING {
            return Ok(bytes);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(read) if bytes.len().saturating_add(read) > max_bytes => {
                let _ = cancel.compare_exchange(
                    CONTROL_RUNNING,
                    CONTROL_OUTPUT_TOO_LARGE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                return Ok(Vec::new());
            }
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ProcessGroupSnapshot {
    live_members: usize,
    exited_unreaped_descendants: usize,
}

#[cfg(target_os = "linux")]
fn parse_proc_stat_group(stat: &str) -> Option<(u8, i32)> {
    let (_, fields) = stat.rsplit_once(')')?;
    let mut fields = fields.split_ascii_whitespace();
    let state = *fields.next()?.as_bytes().first()?;
    let _parent_pid = fields.next()?;
    let process_group = fields.next()?.parse().ok()?;
    Some((state, process_group))
}

#[cfg(target_os = "linux")]
fn process_group_snapshot(process_group: u32) -> io::Result<ProcessGroupSnapshot> {
    let mut snapshot = ProcessGroupSnapshot::default();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let stat_path = entry.path().join("stat");
        let stat = match std::fs::read_to_string(stat_path) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let Some((state, actual_group)) = parse_proc_stat_group(&stat) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid proc stat",
            ));
        };
        if actual_group != process_group as i32 {
            continue;
        }
        if state == b'Z' || state == b'X' {
            snapshot.exited_unreaped_descendants += 1;
        } else {
            snapshot.live_members += 1;
        }
    }
    Ok(snapshot)
}

#[cfg(target_os = "linux")]
fn signal_process_group(process_group: u32, signal: libc::c_int) -> io::Result<()> {
    // SAFETY: the child was started in a new process group whose id is its pid.
    if unsafe { libc::kill(-(process_group as libc::pid_t), signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
fn terminate_owned_group(
    child: &mut std::process::Child,
    process_group: u32,
    initial_status: Option<ExitStatus>,
    grace: Duration,
) -> Result<(ExitStatus, bool, usize), ()> {
    let mut status = initial_status;
    let mut snapshot = process_group_snapshot(process_group).ok();
    let mut escalated = false;

    if let (Some(status), Some(group)) = (status, snapshot)
        && group.live_members == 0
    {
        return Ok((status, false, group.exited_unreaped_descendants));
    }

    let term_sent = signal_process_group(process_group, libc::SIGTERM).is_ok();
    // If TERM failed, skip its grace period and proceed to the checked KILL
    // phase. A failed signal is never evidence that the group exited.
    let term_deadline = Instant::now() + if term_sent { grace } else { Duration::ZERO };
    while Instant::now() < term_deadline {
        if status.is_none() {
            if let Ok(Some(exited)) = child.try_wait() {
                status = Some(exited);
            }
        }
        snapshot = process_group_snapshot(process_group).ok();
        if let (Some(status), Some(group)) = (status, snapshot)
            && group.live_members == 0
        {
            return Ok((status, false, group.exited_unreaped_descendants));
        }
        thread::sleep(Duration::from_millis(20));
    }

    match signal_process_group(process_group, libc::SIGKILL) {
        Ok(()) => escalated = true,
        Err(_signal_error) => {
            // A failed kill is not success: only a later complete observation
            // with a reaped leader and no live group members can close cleanup.
        }
    }
    let kill_deadline = Instant::now() + CHILD_KILL_CONFIRMATION;
    while Instant::now() < kill_deadline {
        if status.is_none() {
            if let Ok(Some(exited)) = child.try_wait() {
                status = Some(exited);
            }
        }
        snapshot = process_group_snapshot(process_group).ok();
        if let (Some(status), Some(group)) = (status, snapshot)
            && group.live_members == 0
        {
            return Ok((status, escalated, group.exited_unreaped_descendants));
        }
        thread::sleep(Duration::from_millis(20));
    }

    Err(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChildPairWire {
    schema: String,
    illustrative_canonical: String,
    zero_cost_canonical: String,
}

pub(super) fn parse_child_pair(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SimulationWorkerError> {
    if bytes.is_empty() || bytes.len() > MAX_CHILD_STDOUT_BYTES {
        return Err(SimulationWorkerError::ResultInvalid);
    }
    let value = parse_strict_json(bytes).map_err(|_| SimulationWorkerError::ResultInvalid)?;
    let wire: ChildPairWire =
        serde_json::from_value(value).map_err(|_| SimulationWorkerError::ResultInvalid)?;
    if wire.schema != "owner-beta-simulation-child-pair-v1" {
        return Err(SimulationWorkerError::ResultInvalid);
    }
    Ok((
        wire.illustrative_canonical.into_bytes(),
        wire.zero_cost_canonical.into_bytes(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildExitCode {
    #[cfg(target_os = "linux")]
    ProvenanceInvalid = 21,
    #[cfg(target_os = "linux")]
    InputInvalid = 22,
    #[cfg(target_os = "linux")]
    ComputationFailed = 23,
    #[cfg(target_os = "linux")]
    ResultInvalid = 24,
    #[cfg(not(target_os = "linux"))]
    UnsupportedPlatform = 25,
}

#[cfg(not(target_os = "linux"))]
pub(super) fn run_child_mode(
    _artifact_root: &std::path::Path,
    _receipt_path: &std::path::Path,
    _expected_receipt_sha256: &str,
) -> Result<(), ChildExitCode> {
    Err(ChildExitCode::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
pub(super) fn run_child_mode(
    artifact_root: &std::path::Path,
    receipt_path: &std::path::Path,
    expected_receipt_sha256: &str,
) -> Result<(), ChildExitCode> {
    let runtime =
        TrustedSimulationRuntime::load(artifact_root, receipt_path, expected_receipt_sha256)
            .map_err(|_| ChildExitCode::ProvenanceInvalid)?;
    let request =
        read_child_request(io::stdin().lock()).map_err(|_| ChildExitCode::InputInvalid)?;
    let input = SimulationJobInput::from_json(&request).map_err(|_| ChildExitCode::InputInvalid)?;
    let simulation_request = SimulationRunRequest::new(input.run_id(), input.strategy_config_id())
        .with_evaluation_window(Some(input.evaluation_start()), Some(input.evaluation_end()));
    let identity = SimulationRunIdentity::new(runtime.engine().clone());
    let results = run_approved_historical_price_only_simulation(
        runtime.artifact_root(),
        &simulation_request,
        &identity,
    )
    .map_err(map_simulation_error)?;
    let illustrative = canonical_json(results.illustrative_10bp_per_side().result())
        .map_err(|_| ChildExitCode::ResultInvalid)?;
    let zero_cost = canonical_json(results.zero_cost_reference().result())
        .map_err(|_| ChildExitCode::ResultInvalid)?;
    let illustrative = String::from_utf8(illustrative).map_err(|_| ChildExitCode::ResultInvalid)?;
    let zero_cost = String::from_utf8(zero_cost).map_err(|_| ChildExitCode::ResultInvalid)?;
    let response = ChildPairWire {
        schema: "owner-beta-simulation-child-pair-v1".to_owned(),
        illustrative_canonical: illustrative,
        zero_cost_canonical: zero_cost,
    };
    let bytes = serde_json::to_vec(&response).map_err(|_| ChildExitCode::ResultInvalid)?;
    if bytes.len() > MAX_CHILD_STDOUT_BYTES {
        return Err(ChildExitCode::ResultInvalid);
    }
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|_| stdout.flush())
        .map_err(|_| ChildExitCode::ResultInvalid)
}

#[cfg(any(target_os = "linux", test))]
fn read_child_request(mut stdin: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stdin
        .by_ref()
        .take(MAX_CHILD_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_CHILD_REQUEST_BYTES {
        return Err(io::Error::other("child request limit exceeded"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn map_simulation_error(error: crate::owner_beta::simulation::SimulationRunError) -> ChildExitCode {
    use crate::owner_beta::simulation::SimulationRunError;
    match error {
        SimulationRunError::ApprovalRejected | SimulationRunError::InputInvalid => {
            ChildExitCode::InputInvalid
        }
        SimulationRunError::ResultInvalid => ChildExitCode::ResultInvalid,
        SimulationRunError::SignalComputationFailed
        | SimulationRunError::FactorIdentityUnavailable => ChildExitCode::ComputationFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn child_request_and_pair_wire_are_bounded_and_strict() {
        assert!(read_child_request(&b"{}"[..]).is_ok());
        assert!(read_child_request(&vec![b'x'; MAX_CHILD_REQUEST_BYTES + 1][..]).is_err());
        let valid = serde_json::to_vec(&json!({
            "schema": "owner-beta-simulation-child-pair-v1",
            "illustrative_canonical": "{}",
            "zero_cost_canonical": "{}"
        }))
        .expect("wire serializes");
        assert_eq!(
            parse_child_pair(&valid).expect("wire parses"),
            (b"{}".to_vec(), b"{}".to_vec())
        );
        assert!(parse_child_pair(
            br#"{"schema":"owner-beta-simulation-child-pair-v1","schema":"x","illustrative_canonical":"{}","zero_cost_canonical":"{}"}"#
        )
        .is_err());
        assert!(parse_child_pair(
            br#"{"schema":"owner-beta-simulation-child-pair-v1","illustrative_canonical":"{}","zero_cost_canonical":"{}","extra":true}"#
        )
        .is_err());
        assert!(parse_child_pair(b"{} {} ").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_process_uses_clean_environment_and_returns_handshake() {
        let output = fixture(
            "printf '{\"pid\":%s,\"database\":\"%s\",\"build\":\"%s\"}' \"$$\" \"${DATABASE_URL-unset}\" \"${R4_SIMULATION_BUILD_GIT_COMMIT-unset}\"",
            Duration::from_secs(2),
            Duration::from_millis(40),
            4096,
        );
        assert_eq!(output.stop_reason, None);
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("handshake json");
        assert_eq!(value["database"], "unset");
        assert_eq!(value["build"], "unset");
        assert_eq!(value["pid"].as_u64(), Some(u64::from(output.pid)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_process_escalates_reaps_leader_and_leaves_no_live_group_member() {
        let output = fixture(
            "trap '' TERM; (trap '' TERM; exec sleep 30) & child=$!; printf '%s\\n' \"$child\"; wait",
            Duration::from_millis(100),
            Duration::from_millis(50),
            4096,
        );
        assert_eq!(output.stop_reason, Some(StopReason::TimedOut));
        assert!(output.kill_escalated);
        assert_direct_child_reaped(output.pid as i32);
        assert_eq!(
            process_group_snapshot(output.pid)
                .expect("process group is observable")
                .live_members,
            0
        );
        let descendant = std::str::from_utf8(&output.stdout)
            .expect("pid output")
            .trim()
            .parse::<i32>()
            .expect("numeric pid");
        let observation = assert_process_not_live(descendant);
        if observation == TestProcessObservation::Zombie {
            assert!(output.exited_unreaped_descendants > 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_process_cleans_descendant_after_leader_exits() {
        let output = fixture(
            "sleep 30 & child=$!; printf '%s\\n' \"$child\"; exit 0",
            Duration::from_secs(2),
            Duration::from_millis(50),
            4096,
        );
        assert_eq!(output.stop_reason, None);
        assert!(output.status.success());
        assert_direct_child_reaped(output.pid as i32);
        assert_eq!(process_group_snapshot(output.pid).unwrap().live_members, 0);
        let descendant = std::str::from_utf8(&output.stdout)
            .expect("pid output")
            .trim()
            .parse::<i32>()
            .expect("numeric pid");
        let observation = assert_process_not_live(descendant);
        if observation == TestProcessObservation::Zombie {
            assert!(output.exited_unreaped_descendants > 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_process_cancellation_reaps_the_owned_group() {
        let cancel = Arc::new(AtomicU8::new(CONTROL_RUNNING));
        let worker_cancel = cancel.clone();
        let worker = thread::spawn(move || {
            run_with_limits(
                ChildCommand {
                    executable: PathBuf::from("/bin/sh"),
                    args: vec![
                        OsString::from("-c"),
                        OsString::from("sleep 30 & child=$!; printf '%s\\n' \"$child\"; wait"),
                    ],
                    request: Vec::new(),
                },
                worker_cancel,
                ProcessLimits {
                    deadline: Duration::from_secs(3),
                    term_grace: Duration::from_millis(50),
                    max_stdout_bytes: 4096,
                },
            )
        });
        thread::sleep(Duration::from_millis(100));
        cancel.store(CONTROL_CANCEL, Ordering::Release);
        let output = worker
            .join()
            .expect("process supervisor joins")
            .expect("process supervisor completes");
        assert_eq!(output.stop_reason, Some(StopReason::Canceled));
        let descendant = std::str::from_utf8(&output.stdout)
            .expect("pid output")
            .trim()
            .parse::<i32>()
            .expect("numeric pid");
        assert_direct_child_reaped(output.pid as i32);
        assert_eq!(process_group_snapshot(output.pid).unwrap().live_members, 0);
        let observation = assert_process_not_live(descendant);
        if observation == TestProcessObservation::Zombie {
            assert!(output.exited_unreaped_descendants > 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixture_process_enforces_stdout_limit_while_reading() {
        let output = fixture(
            "head -c 1024 /dev/zero",
            Duration::from_secs(2),
            Duration::from_millis(40),
            64,
        );
        assert_eq!(output.stop_reason, Some(StopReason::OutputTooLarge));
        assert!(output.stdout.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn quickly_exiting_oversized_child_remains_output_too_large_after_join() {
        let oversized = "x".repeat(1024);
        let script = format!("printf '%s' '{oversized}'; exit 0");
        let output = fixture(
            &script,
            Duration::from_secs(2),
            Duration::from_millis(50),
            64,
        );
        assert!(output.status.success());
        assert_eq!(output.stop_reason, Some(StopReason::OutputTooLarge));
        assert!(output.stdout.is_empty());
        assert_eq!(
            reconcile_stop_reason(None, CONTROL_OUTPUT_TOO_LARGE),
            Some(StopReason::OutputTooLarge)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn broken_child_stdin_cannot_be_accepted_as_successful_input_delivery() {
        let result = run_with_limits(
            ChildCommand {
                executable: PathBuf::from("/bin/sh"),
                args: vec![
                    OsString::from("-c"),
                    OsString::from("exec 0<&-; sleep 0.1; exit 0"),
                ],
                request: vec![b'x'; MAX_CHILD_REQUEST_BYTES],
            },
            Arc::new(AtomicU8::new(CONTROL_RUNNING)),
            ProcessLimits {
                deadline: Duration::from_secs(2),
                term_grace: Duration::from_millis(50),
                max_stdout_bytes: 4096,
            },
        );
        assert!(matches!(result, Err(ChildStartError::PipeUnavailable)));
    }

    #[cfg(target_os = "linux")]
    fn fixture(
        script: &str,
        deadline: Duration,
        term_grace: Duration,
        max_stdout_bytes: usize,
    ) -> BoundedChildResult {
        let cancel = Arc::new(AtomicU8::new(CONTROL_RUNNING));
        run_with_limits(
            ChildCommand {
                executable: PathBuf::from("/bin/sh"),
                args: vec![OsString::from("-c"), OsString::from(script)],
                request: Vec::new(),
            },
            cancel,
            ProcessLimits {
                deadline,
                term_grace,
                max_stdout_bytes,
            },
        )
        .expect("fixture process lifecycle")
    }

    #[cfg(target_os = "linux")]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestProcessObservation {
        Absent,
        Zombie,
        Live,
    }

    #[cfg(target_os = "linux")]
    fn process_observation(process_id: i32) -> TestProcessObservation {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{process_id}/stat")) else {
            return TestProcessObservation::Absent;
        };
        match parse_proc_stat_group(&stat).map(|(state, _)| state) {
            Some(b'Z' | b'X') => TestProcessObservation::Zombie,
            Some(_) => TestProcessObservation::Live,
            None => TestProcessObservation::Live,
        }
    }

    #[cfg(target_os = "linux")]
    fn assert_direct_child_reaped(process_id: i32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if process_observation(process_id) == TestProcessObservation::Absent {
                return;
            }
            assert!(Instant::now() < deadline, "direct child was not reaped");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(target_os = "linux")]
    fn assert_process_not_live(process_id: i32) -> TestProcessObservation {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let observation = process_observation(process_id);
            if observation != TestProcessObservation::Live {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "fixture descendant remained live"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
