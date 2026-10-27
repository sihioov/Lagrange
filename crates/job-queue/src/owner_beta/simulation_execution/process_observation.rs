use std::fmt;

use uuid::Uuid;

use super::SimulationProcessGeneration;

const BOOT_ID_LIMIT: usize = 37;
const STAT_LIMIT: usize = 4096;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const MILLIS_PER_SECOND: u64 = 1_000;
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Value-free failures at the Linux process observation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationProcessObservationError {
    #[error("Linux process observation is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the fixed procfs observation path is unsafe")]
    UnsafeProcfs,
    #[error("a process observation operation is unavailable")]
    Unavailable,
    #[error("the process observation is invalid")]
    InvalidProcess,
    #[error("the process observation exceeds a size limit")]
    SizeLimit,
    #[error("the observed process generation changed")]
    Changed,
    #[error("the boot clock observation is invalid")]
    ClockInvalid,
    #[error("the process observation handle is permanently invalidated")]
    Invalidated,
}

/// A private-handle observation of one process generation.
///
/// Callers must provide the genuine fixed `/proc` view for the caller and
/// target PID namespace, keep the caller and any later health writer or probe
/// in the same time namespace, and prevent privileged namespace or mount
/// changes during the observation lifetime. This adapter checks procfs type
/// and retained-name identity; it does not certify namespace alignment,
/// executable identity, UID, configuration, database rights, useful progress,
/// readiness, or continued life after a method returns. A matching boot UUID
/// alone does not prove that the caller and target share a time namespace.
///
/// Linux observations use `CLOCK_BOOTTIME`, so future age checks can count
/// suspend time. Runtime clock behavior, kernel support, and namespace policy
/// remain external verification gates; this source does not claim to have
/// measured them.
pub struct SimulationProcessObservation {
    backend: Box<dyn ObservationBackend>,
    generation: SimulationProcessGeneration,
    observed_boot_ms: u64,
    invalidated: bool,
}

impl fmt::Debug for SimulationProcessObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationProcessObservation")
    }
}

impl SimulationProcessObservation {
    /// Returns the generation captured by the last successful observation.
    pub fn generation(&self) -> SimulationProcessGeneration {
        self.generation
    }

    /// Returns the BOOTTIME value from the last successful observation.
    ///
    /// This historical snapshot is not a process-health or readiness proof.
    pub fn observed_boot_ms(&self) -> u64 {
        self.observed_boot_ms
    }

    /// Rechecks the retained process and directory handles once.
    ///
    /// Any failure permanently invalidates this handle. A later call returns
    /// `Invalidated` without backend I/O; a caller that wants a new observation
    /// must explicitly create a new handle through one of the public factories.
    #[cfg(target_os = "linux")]
    pub fn revalidate(&mut self) -> Result<u64, SimulationProcessObservationError> {
        if self.invalidated {
            return Err(SimulationProcessObservationError::Invalidated);
        }

        match observe_flow(
            self.backend.as_mut(),
            self.generation.pid(),
            Some(self.generation),
            Some(self.observed_boot_ms),
        ) {
            Ok((generation, observed_boot_ms)) => {
                self.generation = generation;
                self.observed_boot_ms = observed_boot_ms;
                Ok(observed_boot_ms)
            }
            Err(error) => {
                self.invalidated = true;
                Err(error)
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn revalidate(&mut self) -> Result<u64, SimulationProcessObservationError> {
        self.invalidated = true;
        Err(SimulationProcessObservationError::UnsupportedPlatform)
    }
}

/// Captures a process generation with a retained pidfd and fixed procfs handles.
#[cfg(target_os = "linux")]
pub fn observe_simulation_process(
    pid: u32,
) -> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    capture_with_backend_factory(pid, linux::open_backend)
}

/// Captures the current process using its actual process ID, never an assumed PID.
#[cfg(target_os = "linux")]
pub fn observe_current_simulation_process()
-> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    capture_current_with_backend_factory(std::process::id, linux::open_backend)
}

/// Non-Linux platforms fail before reading the current PID or performing I/O.
#[cfg(not(target_os = "linux"))]
pub fn observe_simulation_process(
    _pid: u32,
) -> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    Err(SimulationProcessObservationError::UnsupportedPlatform)
}

/// Non-Linux platforms fail before reading the current PID or performing I/O.
#[cfg(not(target_os = "linux"))]
pub fn observe_current_simulation_process()
-> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    Err(SimulationProcessObservationError::UnsupportedPlatform)
}

trait ObservationBackend {
    fn validate_chain(&mut self) -> Result<(), SimulationProcessObservationError>;
    fn poll_pidfd(&mut self) -> Result<(), SimulationProcessObservationError>;
    fn read_boot_id(&mut self) -> Result<Vec<u8>, SimulationProcessObservationError>;
    fn read_stat(&mut self) -> Result<Vec<u8>, SimulationProcessObservationError>;
    fn boottime(&mut self) -> Result<ClockReading, SimulationProcessObservationError>;
}

#[derive(Clone, Copy)]
struct ClockReading {
    seconds: i128,
    nanoseconds: i128,
}

fn capture_with_backend_factory(
    pid: u32,
    factory: impl FnOnce(u32) -> Result<Box<dyn ObservationBackend>, SimulationProcessObservationError>,
) -> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    validate_pid(pid)?;
    let mut backend = factory(pid)?;
    let (generation, observed_boot_ms) = observe_flow(backend.as_mut(), pid, None, None)?;
    Ok(SimulationProcessObservation {
        backend,
        generation,
        observed_boot_ms,
        invalidated: false,
    })
}

fn capture_current_with_backend_factory(
    current_pid: impl FnOnce() -> u32,
    factory: impl FnOnce(u32) -> Result<Box<dyn ObservationBackend>, SimulationProcessObservationError>,
) -> Result<SimulationProcessObservation, SimulationProcessObservationError> {
    capture_with_backend_factory(current_pid(), factory)
}

fn validate_pid(pid: u32) -> Result<(), SimulationProcessObservationError> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }
    Ok(())
}

fn observe_flow(
    backend: &mut dyn ObservationBackend,
    pid: u32,
    expected_generation: Option<SimulationProcessGeneration>,
    previous_boot_ms: Option<u64>,
) -> Result<(SimulationProcessGeneration, u64), SimulationProcessObservationError> {
    backend.validate_chain()?;
    backend.poll_pidfd()?;

    let boot_a = parse_boot_id(&backend.read_boot_id()?)?;
    let stat_a = backend.read_stat()?;
    let generation_a = parse_stat(pid, boot_a, &stat_a)?;
    if expected_generation.is_some_and(|expected| expected != generation_a) {
        return Err(SimulationProcessObservationError::Changed);
    }

    let time_a = clock_to_boot_ms(backend.boottime()?)?;

    let stat_b = backend.read_stat()?;
    let boot_b = parse_boot_id(&backend.read_boot_id()?)?;
    let generation_b = parse_stat(pid, boot_b, &stat_b)?;
    if generation_b != generation_a
        || expected_generation.is_some_and(|expected| expected != generation_b)
    {
        return Err(SimulationProcessObservationError::Changed);
    }

    backend.validate_chain()?;
    let time_b = clock_to_boot_ms(backend.boottime()?)?;
    if time_b < time_a || previous_boot_ms.is_some_and(|previous| time_b < previous) {
        return Err(SimulationProcessObservationError::ClockInvalid);
    }

    backend.poll_pidfd()?;
    Ok((generation_a, time_b))
}

fn clock_to_boot_ms(reading: ClockReading) -> Result<u64, SimulationProcessObservationError> {
    if reading.seconds < 0 || reading.nanoseconds < 0 || reading.nanoseconds >= NANOS_PER_SECOND {
        return Err(SimulationProcessObservationError::ClockInvalid);
    }

    let seconds = u64::try_from(reading.seconds)
        .map_err(|_| SimulationProcessObservationError::ClockInvalid)?;
    let nanoseconds = u64::try_from(reading.nanoseconds)
        .map_err(|_| SimulationProcessObservationError::ClockInvalid)?;
    seconds
        .checked_mul(MILLIS_PER_SECOND)
        .and_then(|whole| whole.checked_add(nanoseconds / NANOS_PER_MILLISECOND))
        .ok_or(SimulationProcessObservationError::ClockInvalid)
}

fn parse_boot_id(bytes: &[u8]) -> Result<Uuid, SimulationProcessObservationError> {
    if bytes.len() > BOOT_ID_LIMIT {
        return Err(SimulationProcessObservationError::SizeLimit);
    }
    if bytes.len() != BOOT_ID_LIMIT || bytes[36] != b'\n' {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    let text = std::str::from_utf8(&bytes[..36])
        .map_err(|_| SimulationProcessObservationError::InvalidProcess)?;
    let boot_id =
        Uuid::parse_str(text).map_err(|_| SimulationProcessObservationError::InvalidProcess)?;
    if boot_id.is_nil() || boot_id.to_string().as_bytes() != &bytes[..36] {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }
    Ok(boot_id)
}

fn parse_stat(
    expected_pid: u32,
    boot_id: Uuid,
    bytes: &[u8],
) -> Result<SimulationProcessGeneration, SimulationProcessObservationError> {
    if bytes.len() > STAT_LIMIT {
        return Err(SimulationProcessObservationError::SizeLimit);
    }
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    let pid = expected_pid.to_string();
    let prefix = pid.as_bytes();
    if bytes.len() <= prefix.len() + 1
        || &bytes[..prefix.len()] != prefix
        || bytes[prefix.len()] != b' '
        || bytes[prefix.len() + 1] != b'('
    {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    let comm_start = prefix.len() + 2;
    let close = bytes[comm_start..]
        .iter()
        .rposition(|byte| *byte == b')')
        .map(|offset| comm_start + offset)
        .ok_or(SimulationProcessObservationError::InvalidProcess)?;
    let suffix_start = close + 1;
    if suffix_start >= bytes.len() || !bytes[suffix_start].is_ascii_whitespace() {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    let fields: Vec<&[u8]> = bytes[suffix_start..].split_ascii_whitespace().collect();
    if fields.len() < 20 || fields[0].len() != 1 {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }
    if !matches!(fields[0][0], b'R' | b'S' | b'D' | b'T' | b't' | b'I') {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    if !fields[1..19].iter().copied().all(is_signed_decimal) {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }
    if fields[20..]
        .iter()
        .copied()
        .any(|field| !is_signed_decimal(field))
    {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    // Kernel start ticks are opaque generation identity; never convert them
    // using HZ or treat them as an elapsed-time measurement.
    let start_ticks = parse_positive_canonical_u64(fields[19])?;
    SimulationProcessGeneration::new(expected_pid, start_ticks, boot_id)
        .map_err(|_| SimulationProcessObservationError::InvalidProcess)
}

fn is_signed_decimal(bytes: &[u8]) -> bool {
    let digits = bytes.strip_prefix(b"-").unwrap_or(bytes);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn parse_positive_canonical_u64(bytes: &[u8]) -> Result<u64, SimulationProcessObservationError> {
    if bytes.is_empty() || !(b'1'..=b'9').contains(&bytes[0]) {
        return Err(SimulationProcessObservationError::InvalidProcess);
    }

    let mut value = 0_u64;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(SimulationProcessObservationError::InvalidProcess);
        }
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(u64::from(*byte - b'0')))
            .ok_or(SimulationProcessObservationError::InvalidProcess)?;
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::{CStr, CString},
        mem::MaybeUninit,
        os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    };

    use super::{
        BOOT_ID_LIMIT, ClockReading, ObservationBackend, STAT_LIMIT,
        SimulationProcessObservationError as Error,
    };

    const DIRECTORY_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const REGULAR_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const PROC_NAMES: [&CStr; 3] = [c"sys", c"kernel", c"random"];

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct FileIdentity {
        pub(super) device: u64,
        pub(super) inode: u64,
        pub(super) kind: libc::mode_t,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct PollOutcome {
        pub(super) ready: libc::c_int,
        pub(super) revents: libc::c_short,
    }

    /// The private syscall boundary used by both production policy and
    /// synthetic descriptor regressions. Associated handles own their close
    /// behavior, so partial acquisition follows the same drop path as Linux.
    pub(super) trait LinuxSyscalls {
        type Handle;

        fn open_path(
            &mut self,
            path: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int>;
        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int>;
        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<FileIdentity, libc::c_int>;
        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<FileIdentity, libc::c_int>;
        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int>;
        fn read(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int>;
        fn pidfd_open(
            &mut self,
            pid: u32,
            flags: libc::c_uint,
        ) -> Result<Self::Handle, libc::c_int>;
        fn poll(
            &mut self,
            descriptor: &Self::Handle,
            events: libc::c_short,
            timeout_ms: libc::c_int,
        ) -> Result<PollOutcome, libc::c_int>;
        fn boottime(&mut self) -> Result<ClockReading, libc::c_int>;
    }

    struct LinuxHandle(OwnedFd);

    struct RealLinuxSyscalls;

    pub(super) struct LinuxBackend<S: LinuxSyscalls> {
        syscalls: S,
        pid_name: CString,
        root: S::Handle,
        proc_dir: S::Handle,
        sys_dir: S::Handle,
        kernel_dir: S::Handle,
        random_dir: S::Handle,
        target_dir: S::Handle,
        pidfd: S::Handle,
    }

    impl<S: LinuxSyscalls> LinuxBackend<S> {
        #[cfg(test)]
        pub(super) fn open_with(syscalls: S, pid: u32) -> Result<Self, Error> {
            Self::open_impl(syscalls, pid)
        }

        fn open_syscalls(syscalls: S, pid: u32) -> Result<Self, Error> {
            Self::open_impl(syscalls, pid)
        }

        fn open_impl(mut syscalls: S, pid: u32) -> Result<Self, Error> {
            super::validate_pid(pid)?;

            let root = open_root(&mut syscalls)?;
            verify_root(&mut syscalls, &root)?;
            let proc_dir = open_directory(&mut syscalls, &root, c"proc", true)?;
            let sys_dir = open_directory(&mut syscalls, &proc_dir, PROC_NAMES[0], true)?;
            let kernel_dir = open_directory(&mut syscalls, &sys_dir, PROC_NAMES[1], true)?;
            let random_dir = open_directory(&mut syscalls, &kernel_dir, PROC_NAMES[2], true)?;
            let pid_name = CString::new(pid.to_string()).map_err(|_| Error::InvalidProcess)?;
            let target_dir = open_directory(&mut syscalls, &proc_dir, &pid_name, true)?;
            let pidfd = open_pidfd(&mut syscalls, pid)?;

            Ok(Self {
                syscalls,
                pid_name,
                root,
                proc_dir,
                sys_dir,
                kernel_dir,
                random_dir,
                target_dir,
                pidfd,
            })
        }
    }

    impl<S: LinuxSyscalls> ObservationBackend for LinuxBackend<S> {
        fn validate_chain(&mut self) -> Result<(), Error> {
            let chain = DirectoryChain {
                root: &self.root,
                proc_dir: &self.proc_dir,
                sys_dir: &self.sys_dir,
                kernel_dir: &self.kernel_dir,
                random_dir: &self.random_dir,
                target_dir: &self.target_dir,
                pid_name: &self.pid_name,
            };
            validate_directories(&mut self.syscalls, &chain)
        }

        fn poll_pidfd(&mut self) -> Result<(), Error> {
            poll_pidfd(&mut self.syscalls, &self.pidfd)
        }

        fn read_boot_id(&mut self) -> Result<Vec<u8>, Error> {
            let chain = DirectoryChain {
                root: &self.root,
                proc_dir: &self.proc_dir,
                sys_dir: &self.sys_dir,
                kernel_dir: &self.kernel_dir,
                random_dir: &self.random_dir,
                target_dir: &self.target_dir,
                pid_name: &self.pid_name,
            };
            read_leaf(
                &mut self.syscalls,
                &chain,
                &self.random_dir,
                c"boot_id",
                BOOT_ID_LIMIT,
            )
        }

        fn read_stat(&mut self) -> Result<Vec<u8>, Error> {
            let chain = DirectoryChain {
                root: &self.root,
                proc_dir: &self.proc_dir,
                sys_dir: &self.sys_dir,
                kernel_dir: &self.kernel_dir,
                random_dir: &self.random_dir,
                target_dir: &self.target_dir,
                pid_name: &self.pid_name,
            };
            read_leaf(
                &mut self.syscalls,
                &chain,
                &self.target_dir,
                c"stat",
                STAT_LIMIT,
            )
        }

        fn boottime(&mut self) -> Result<ClockReading, Error> {
            self.syscalls.boottime().map_err(|_| Error::ClockInvalid)
        }
    }

    pub(super) fn open_backend(pid: u32) -> Result<Box<dyn ObservationBackend>, Error> {
        let backend = LinuxBackend::open_syscalls(RealLinuxSyscalls, pid)?;
        Ok(Box::new(backend))
    }

    struct DirectoryChain<'a, H> {
        root: &'a H,
        proc_dir: &'a H,
        sys_dir: &'a H,
        kernel_dir: &'a H,
        random_dir: &'a H,
        target_dir: &'a H,
        pid_name: &'a CStr,
    }

    fn validate_directories<S: LinuxSyscalls>(
        syscalls: &mut S,
        chain: &DirectoryChain<'_, S::Handle>,
    ) -> Result<(), Error> {
        verify_root(syscalls, chain.root)?;
        verify_named(syscalls, chain.root, c"proc", chain.proc_dir, true)?;
        verify_named(syscalls, chain.proc_dir, PROC_NAMES[0], chain.sys_dir, true)?;
        verify_named(
            syscalls,
            chain.sys_dir,
            PROC_NAMES[1],
            chain.kernel_dir,
            true,
        )?;
        verify_named(
            syscalls,
            chain.kernel_dir,
            PROC_NAMES[2],
            chain.random_dir,
            true,
        )?;
        verify_named(
            syscalls,
            chain.proc_dir,
            chain.pid_name,
            chain.target_dir,
            true,
        )?;
        Ok(())
    }

    fn open_root<S: LinuxSyscalls>(syscalls: &mut S) -> Result<S::Handle, Error> {
        syscalls
            .open_path(c"/", DIRECTORY_FLAGS)
            .map_err(|_| Error::Unavailable)
    }

    fn open_directory<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        procfs: bool,
    ) -> Result<S::Handle, Error> {
        let directory = syscalls
            .open_at(parent, name, DIRECTORY_FLAGS)
            .map_err(classify_path_open_error)?;
        verify_named(syscalls, parent, name, &directory, procfs)?;
        Ok(directory)
    }

    fn open_regular<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
    ) -> Result<S::Handle, Error> {
        let file = syscalls
            .open_at(parent, name, REGULAR_FLAGS)
            .map_err(classify_path_open_error)?;
        verify_named_fd(syscalls, parent, name, &file, libc::S_IFREG, true)?;
        Ok(file)
    }

    fn open_pidfd<S: LinuxSyscalls>(syscalls: &mut S, pid: u32) -> Result<S::Handle, Error> {
        syscalls
            .pidfd_open(pid, 0)
            .map_err(classify_pidfd_open_error)
    }

    fn verify_root<S: LinuxSyscalls>(syscalls: &mut S, root: &S::Handle) -> Result<(), Error> {
        // An absolute fixed name makes fstatat compare the retained root to
        // the caller's currently named `/`, including a changed process root.
        verify_named_fd(syscalls, root, c"/", root, libc::S_IFDIR, false)
    }

    fn verify_named<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        child: &S::Handle,
        procfs: bool,
    ) -> Result<(), Error> {
        verify_named_fd(syscalls, parent, name, child, libc::S_IFDIR, procfs)
    }

    fn verify_named_fd<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        descriptor: &S::Handle,
        expected_kind: libc::mode_t,
        procfs: bool,
    ) -> Result<(), Error> {
        let held = syscalls.fstat(descriptor).map_err(|_| Error::Unavailable)?;
        if held.kind != expected_kind {
            return Err(Error::UnsafeProcfs);
        }

        let named = syscalls
            .fstat_at(parent, name, libc::AT_SYMLINK_NOFOLLOW)
            .map_err(classify_named_error)?;
        if named.kind != expected_kind {
            return Err(Error::UnsafeProcfs);
        }
        if held != named {
            return Err(Error::Changed);
        }
        if procfs {
            verify_procfs(syscalls, descriptor)?;
        }
        Ok(())
    }

    fn verify_procfs<S: LinuxSyscalls>(
        syscalls: &mut S,
        descriptor: &S::Handle,
    ) -> Result<(), Error> {
        let filesystem_type = syscalls
            .fstatfs_type(descriptor)
            .map_err(|_| Error::Unavailable)?;
        if filesystem_type != libc::PROC_SUPER_MAGIC as u64 {
            return Err(Error::UnsafeProcfs);
        }
        Ok(())
    }

    fn read_leaf<S: LinuxSyscalls>(
        syscalls: &mut S,
        chain: &DirectoryChain<'_, S::Handle>,
        parent: &S::Handle,
        name: &CStr,
        limit: usize,
    ) -> Result<Vec<u8>, Error> {
        validate_directories(syscalls, chain)?;
        let descriptor = open_regular(syscalls, parent, name)?;

        let read_result = read_bounded(syscalls, &descriptor, limit);
        let leaf_result = verify_named_fd(syscalls, parent, name, &descriptor, libc::S_IFREG, true);
        let chain_result = validate_directories(syscalls, chain);

        let bytes = read_result?;
        leaf_result?;
        chain_result?;
        Ok(bytes)
    }

    fn read_bounded<S: LinuxSyscalls>(
        syscalls: &mut S,
        descriptor: &S::Handle,
        limit: usize,
    ) -> Result<Vec<u8>, Error> {
        let capacity = limit.checked_add(1).ok_or(Error::SizeLimit)?;
        let mut bytes = vec![0_u8; capacity];
        let mut filled = 0;
        while filled < capacity {
            let remaining = capacity - filled;
            let read = syscalls
                .read(descriptor, &mut bytes[filled..])
                .map_err(|_| Error::Unavailable)?;
            if read == 0 {
                break;
            }
            if read > remaining {
                return Err(Error::Unavailable);
            }
            filled = filled.checked_add(read).ok_or(Error::SizeLimit)?;
        }
        if filled > limit {
            return Err(Error::SizeLimit);
        }
        bytes.truncate(filled);
        Ok(bytes)
    }

    fn poll_pidfd<S: LinuxSyscalls>(syscalls: &mut S, pidfd: &S::Handle) -> Result<(), Error> {
        let outcome = syscalls
            .poll(pidfd, libc::POLLIN, 0)
            .map_err(|_| Error::Unavailable)?;
        if outcome.ready != 0 || outcome.revents != 0 {
            return Err(Error::Changed);
        }
        Ok(())
    }

    pub(super) fn classify_pidfd_open_error(errno: libc::c_int) -> Error {
        if errno == libc::ENOSYS || errno == libc::EINVAL {
            Error::UnsupportedPlatform
        } else {
            Error::Unavailable
        }
    }

    pub(super) fn classify_path_open_error(errno: libc::c_int) -> Error {
        if matches!(errno, libc::ELOOP | libc::ENOTDIR | libc::EISDIR) {
            Error::UnsafeProcfs
        } else {
            Error::Unavailable
        }
    }

    pub(super) fn classify_named_error(errno: libc::c_int) -> Error {
        if errno == libc::ENOTDIR {
            Error::UnsafeProcfs
        } else {
            Error::Unavailable
        }
    }

    impl LinuxSyscalls for RealLinuxSyscalls {
        type Handle = LinuxHandle;

        fn open_path(
            &mut self,
            path: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let fd = unsafe { libc::open(path.as_ptr(), flags) };
            own_opened_fd(fd)
        }

        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let fd = unsafe { libc::openat(parent.0.as_raw_fd(), name.as_ptr(), flags) };
            own_opened_fd(fd)
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<FileIdentity, libc::c_int> {
            let mut value = MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(descriptor.0.as_raw_fd(), value.as_mut_ptr()) } != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(identity(&value))
        }

        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<FileIdentity, libc::c_int> {
            let mut value = MaybeUninit::<libc::stat>::uninit();
            let result = unsafe {
                libc::fstatat(
                    parent.0.as_raw_fd(),
                    name.as_ptr(),
                    value.as_mut_ptr(),
                    flags,
                )
            };
            if result != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(identity(&value))
        }

        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int> {
            let mut value = MaybeUninit::<libc::statfs>::uninit();
            if unsafe { libc::fstatfs(descriptor.0.as_raw_fd(), value.as_mut_ptr()) } != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(value.f_type as u64)
        }

        fn read(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let result = unsafe {
                libc::read(
                    descriptor.0.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if result < 0 {
                Err(last_errno())
            } else {
                // Linux ssize_t and size_t are pointer-width; nonnegative
                // results therefore retain their value in usize.
                Ok(result as usize)
            }
        }

        fn pidfd_open(
            &mut self,
            pid: u32,
            flags: libc::c_uint,
        ) -> Result<Self::Handle, libc::c_int> {
            let result = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, flags) };
            if result < 0 {
                return Err(last_errno());
            }
            let fd = result as RawFd;
            Ok(LinuxHandle(unsafe { OwnedFd::from_raw_fd(fd) }))
        }

        fn poll(
            &mut self,
            descriptor: &Self::Handle,
            events: libc::c_short,
            timeout_ms: libc::c_int,
        ) -> Result<PollOutcome, libc::c_int> {
            let mut poll_descriptor = libc::pollfd {
                fd: descriptor.0.as_raw_fd(),
                events,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll_descriptor, 1, timeout_ms) };
            if ready < 0 {
                return Err(last_errno());
            }
            Ok(PollOutcome {
                ready,
                revents: poll_descriptor.revents,
            })
        }

        fn boottime(&mut self) -> Result<ClockReading, libc::c_int> {
            let mut value = MaybeUninit::<libc::timespec>::uninit();
            let result = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, value.as_mut_ptr()) };
            if result != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(ClockReading {
                seconds: value.tv_sec as i128,
                nanoseconds: value.tv_nsec as i128,
            })
        }
    }

    fn own_opened_fd(fd: RawFd) -> Result<LinuxHandle, libc::c_int> {
        if fd < 0 {
            return Err(last_errno());
        }
        Ok(LinuxHandle(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn identity(value: &libc::stat) -> FileIdentity {
        FileIdentity {
            device: value.st_dev as u64,
            inode: value.st_ino as u64,
            kind: value.st_mode & libc::S_IFMT,
        }
    }

    fn last_errno() -> libc::c_int {
        std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or_default()
    }
}
#[cfg(test)]
mod tests;
