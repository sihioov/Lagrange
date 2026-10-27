#[cfg(target_os = "linux")]
use crate::owner_beta::simulation::parse_strict_json;

use super::SimulationProcessConfig;
#[cfg(target_os = "linux")]
use super::{
    SimulationHealthError, SimulationHealthRecord, SimulationProcessGeneration,
    SimulationProcessObservation, SimulationProcessObservationError,
};

#[cfg(target_os = "linux")]
const CONFIG_SIZE_LIMIT: usize = 16 * 1024;
#[cfg(target_os = "linux")]
const HEALTH_SIZE_LIMIT: usize = 8 * 1024;

/// Value-free failures at the fixed process config and health read boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationProcessReadError {
    #[error("protected process reads are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("a fixed process read path is unsafe")]
    UnsafePath,
    #[error("fixed process read metadata is invalid")]
    InvalidMetadata,
    #[error("a fixed process read exceeds its size limit")]
    SizeLimit,
    #[error("a fixed process read changed while being checked")]
    Changed,
    #[error("a fixed process read operation is unavailable")]
    Unavailable,
    #[error("the protected process config is rejected")]
    ConfigRejected,
    #[error("the observed process generation is invalid")]
    GenerationInvalid,
    #[error("the observed process clock is invalid")]
    ClockInvalid,
    #[error("the process health record is rejected")]
    HealthRejected,
}

/// Reads and checks the fixed protected process config using independently
/// admitted commit and raw-config digest pins.
#[cfg(target_os = "linux")]
pub fn read_simulation_process_config(
    expected_commit: &str,
    expected_config_sha256: &str,
) -> Result<SimulationProcessConfig, SimulationProcessReadError> {
    linux::read_config_real(expected_commit, expected_config_sha256)
}

/// Returns healthy only when the checked record belongs to the observed
/// process generation and remains fresh at the observer's final BOOTTIME.
#[cfg(target_os = "linux")]
pub fn check_simulation_process_health(
    config: &SimulationProcessConfig,
) -> Result<(), SimulationProcessReadError> {
    linux::check_health_real(config)
}

/// Non-Linux builds fail before inspecting pins, config values, PID, clock, or
/// the process observer.
#[cfg(not(target_os = "linux"))]
pub fn read_simulation_process_config(
    _expected_commit: &str,
    _expected_config_sha256: &str,
) -> Result<SimulationProcessConfig, SimulationProcessReadError> {
    Err(SimulationProcessReadError::UnsupportedPlatform)
}

/// Non-Linux builds fail before config inspection, I/O, or process observation.
#[cfg(not(target_os = "linux"))]
pub fn check_simulation_process_health(
    _config: &SimulationProcessConfig,
) -> Result<(), SimulationProcessReadError> {
    Err(SimulationProcessReadError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn valid_expected_pins(commit: &str, config_sha256: &str) -> bool {
    commit.len() == 40
        && commit.bytes().all(is_lower_hex)
        && commit.bytes().any(|byte| byte != b'0')
        && config_sha256
            .strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(is_lower_hex))
}

#[cfg(target_os = "linux")]
fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::CStr,
        mem::MaybeUninit,
        os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    };

    use serde_json::Value;
    use uuid::Uuid;

    use super::{
        CONFIG_SIZE_LIMIT, HEALTH_SIZE_LIMIT, SimulationHealthError, SimulationHealthRecord,
        SimulationProcessConfig, SimulationProcessGeneration, SimulationProcessObservation,
        SimulationProcessObservationError as ObservationError, SimulationProcessReadError as Error,
        parse_strict_json, valid_expected_pins,
    };

    pub(super) const DIRECTORY_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    pub(super) const LEAF_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    pub(super) const NOFOLLOW: libc::c_int = libc::AT_SYMLINK_NOFOLLOW;

    const CONFIG_DIR: &CStr = c"owner-beta-simulation";
    const CONFIG_FILE: &CStr = c"config.json";
    const HEALTH_DIR: &CStr = c"owner-beta-simulation-health";
    const LOCK_FILE: &CStr = c"writer.lock";
    const RECORDS_DIR: &CStr = c"records";
    const HEALTH_FILE: &CStr = c"current.json";
    const RUN_DIR: &CStr = c"run";
    const ROOT_NAME: &CStr = c"/";
    const ACCESS_ACL: &CStr = c"system.posix_acl_access";
    const DEFAULT_ACL: &CStr = c"system.posix_acl_default";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Metadata {
        pub(super) device: u64,
        pub(super) inode: u64,
        pub(super) mode: u32,
        pub(super) uid: u32,
        pub(super) gid: u32,
        pub(super) links: u64,
        pub(super) size: i64,
        pub(super) mtime_seconds: i64,
        pub(super) mtime_nanoseconds: i64,
        pub(super) ctime_seconds: i64,
        pub(super) ctime_nanoseconds: i64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct DirectorySnapshot {
        device: u64,
        inode: u64,
        mode: u32,
        uid: u32,
        gid: u32,
    }

    impl From<Metadata> for DirectorySnapshot {
        fn from(value: Metadata) -> Self {
            Self {
                device: value.device,
                inode: value.inode,
                mode: value.mode,
                uid: value.uid,
                gid: value.gid,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct LeafSnapshot(Metadata);

    #[derive(Clone, Copy)]
    enum DirectoryPolicy {
        RootOrRun,
        Fixed {
            uid: u32,
            gid: u32,
            permissions: u32,
        },
    }

    #[derive(Clone, Copy)]
    struct LeafPolicy {
        uid: u32,
        gid: u32,
        permissions: u32,
        size: LeafSize,
    }

    #[derive(Clone, Copy)]
    enum LeafSize {
        PositiveAtMost(usize),
        ExactlyZero,
    }

    const ROOT_POLICY: DirectoryPolicy = DirectoryPolicy::RootOrRun;
    const RUN_POLICY: DirectoryPolicy = DirectoryPolicy::RootOrRun;
    const CONFIG_DIRECTORY_POLICY: DirectoryPolicy = DirectoryPolicy::Fixed {
        uid: 0,
        gid: 10001,
        permissions: 0o750,
    };
    const HEALTH_DIRECTORY_POLICY: DirectoryPolicy = DirectoryPolicy::Fixed {
        uid: 0,
        gid: 10001,
        permissions: 0o750,
    };
    const RECORDS_DIRECTORY_POLICY: DirectoryPolicy = DirectoryPolicy::Fixed {
        uid: 10001,
        gid: 10001,
        permissions: 0o700,
    };
    const CONFIG_FILE_POLICY: LeafPolicy = LeafPolicy {
        uid: 0,
        gid: 10001,
        permissions: 0o440,
        size: LeafSize::PositiveAtMost(CONFIG_SIZE_LIMIT),
    };
    const LOCK_FILE_POLICY: LeafPolicy = LeafPolicy {
        uid: 0,
        gid: 10001,
        permissions: 0o440,
        size: LeafSize::ExactlyZero,
    };
    const HEALTH_FILE_POLICY: LeafPolicy = LeafPolicy {
        uid: 10001,
        gid: 10001,
        permissions: 0o600,
        size: LeafSize::PositiveAtMost(HEALTH_SIZE_LIMIT),
    };

    /// The only kernel boundary used by acquisition, metadata policy, reads,
    /// and the private synthetic regressions.
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
        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<Metadata, libc::c_int>;
        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Metadata, libc::c_int>;
        fn fgetxattr_size(
            &mut self,
            descriptor: &Self::Handle,
            name: &CStr,
        ) -> Result<Option<usize>, libc::c_int>;
        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int>;
        fn read(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int>;
    }

    pub(super) trait ProcessObservationHandle {
        fn generation(&self) -> SimulationProcessGeneration;
        fn revalidate(&mut self) -> Result<u64, ObservationError>;
    }

    pub(super) trait ProcessObservationFactory {
        type Observation: ProcessObservationHandle;

        fn observe(&mut self, pid: u32) -> Result<Self::Observation, ObservationError>;
    }

    struct PublicObservationFactory;

    impl ProcessObservationHandle for SimulationProcessObservation {
        fn generation(&self) -> SimulationProcessGeneration {
            SimulationProcessObservation::generation(self)
        }

        fn revalidate(&mut self) -> Result<u64, ObservationError> {
            SimulationProcessObservation::revalidate(self)
        }
    }

    impl ProcessObservationFactory for PublicObservationFactory {
        type Observation = SimulationProcessObservation;

        fn observe(&mut self, pid: u32) -> Result<Self::Observation, ObservationError> {
            super::super::observe_simulation_process(pid)
        }
    }

    struct LinuxHandle(OwnedFd);

    struct RealLinuxSyscalls;

    struct ConfigFiles<S: LinuxSyscalls> {
        syscalls: S,
        root: S::Handle,
        run: S::Handle,
        config_directory: S::Handle,
        config_file: S::Handle,
        root_snapshot: DirectorySnapshot,
        run_snapshot: DirectorySnapshot,
        config_directory_snapshot: DirectorySnapshot,
        config_file_snapshot: LeafSnapshot,
    }

    struct HealthFiles<S: LinuxSyscalls> {
        syscalls: S,
        root: S::Handle,
        run: S::Handle,
        health_directory: S::Handle,
        lock_file: S::Handle,
        records_directory: S::Handle,
        health_file: S::Handle,
        root_snapshot: DirectorySnapshot,
        run_snapshot: DirectorySnapshot,
        health_directory_snapshot: DirectorySnapshot,
        lock_file_snapshot: LeafSnapshot,
        records_directory_snapshot: DirectorySnapshot,
        health_file_snapshot: LeafSnapshot,
    }

    impl<S: LinuxSyscalls> ConfigFiles<S> {
        fn open(mut syscalls: S) -> Result<Self, Error> {
            let root = open_root(&mut syscalls)?;
            let root_snapshot =
                verify_directory(&mut syscalls, &root, ROOT_NAME, &root, ROOT_POLICY, None)?;
            let run = open_directory(&mut syscalls, &root, RUN_DIR, RUN_POLICY)?;
            let run_snapshot =
                verify_directory(&mut syscalls, &root, RUN_DIR, &run, RUN_POLICY, None)?;
            let config_directory =
                open_directory(&mut syscalls, &run, CONFIG_DIR, CONFIG_DIRECTORY_POLICY)?;
            let config_directory_snapshot = verify_directory(
                &mut syscalls,
                &run,
                CONFIG_DIR,
                &config_directory,
                CONFIG_DIRECTORY_POLICY,
                None,
            )?;
            let config_file = open_leaf(
                &mut syscalls,
                &config_directory,
                CONFIG_FILE,
                CONFIG_FILE_POLICY,
            )?;
            let config_file_snapshot = verify_leaf(
                &mut syscalls,
                &config_directory,
                CONFIG_FILE,
                &config_file,
                CONFIG_FILE_POLICY,
                None,
            )?;

            Ok(Self {
                syscalls,
                root,
                run,
                config_directory,
                config_file,
                root_snapshot,
                run_snapshot,
                config_directory_snapshot,
                config_file_snapshot,
            })
        }

        fn validate(&mut self) -> Result<(), Error> {
            verify_directory(
                &mut self.syscalls,
                &self.root,
                ROOT_NAME,
                &self.root,
                ROOT_POLICY,
                Some(self.root_snapshot),
            )?;
            verify_directory(
                &mut self.syscalls,
                &self.root,
                RUN_DIR,
                &self.run,
                RUN_POLICY,
                Some(self.run_snapshot),
            )?;
            verify_directory(
                &mut self.syscalls,
                &self.run,
                CONFIG_DIR,
                &self.config_directory,
                CONFIG_DIRECTORY_POLICY,
                Some(self.config_directory_snapshot),
            )?;
            verify_leaf(
                &mut self.syscalls,
                &self.config_directory,
                CONFIG_FILE,
                &self.config_file,
                CONFIG_FILE_POLICY,
                Some(self.config_file_snapshot),
            )?;
            Ok(())
        }

        fn read_once(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
            self.validate()?;
            let read = read_bounded(
                &mut self.syscalls,
                &self.config_file,
                limit,
                self.config_file_snapshot.0.size,
            );
            let validation = self.validate();
            let bytes = read?;
            validation?;
            Ok(bytes)
        }
    }

    impl<S: LinuxSyscalls> HealthFiles<S> {
        fn open(mut syscalls: S) -> Result<Self, Error> {
            let root = open_root(&mut syscalls)?;
            let root_snapshot =
                verify_directory(&mut syscalls, &root, ROOT_NAME, &root, ROOT_POLICY, None)?;
            let run = open_directory(&mut syscalls, &root, RUN_DIR, RUN_POLICY)?;
            let run_snapshot =
                verify_directory(&mut syscalls, &root, RUN_DIR, &run, RUN_POLICY, None)?;
            let health_directory =
                open_directory(&mut syscalls, &run, HEALTH_DIR, HEALTH_DIRECTORY_POLICY)?;
            let health_directory_snapshot = verify_directory(
                &mut syscalls,
                &run,
                HEALTH_DIR,
                &health_directory,
                HEALTH_DIRECTORY_POLICY,
                None,
            )?;
            let lock_file = open_leaf(
                &mut syscalls,
                &health_directory,
                LOCK_FILE,
                LOCK_FILE_POLICY,
            )?;
            let lock_file_snapshot = verify_leaf(
                &mut syscalls,
                &health_directory,
                LOCK_FILE,
                &lock_file,
                LOCK_FILE_POLICY,
                None,
            )?;
            let records_directory = open_directory(
                &mut syscalls,
                &health_directory,
                RECORDS_DIR,
                RECORDS_DIRECTORY_POLICY,
            )?;
            let records_directory_snapshot = verify_directory(
                &mut syscalls,
                &health_directory,
                RECORDS_DIR,
                &records_directory,
                RECORDS_DIRECTORY_POLICY,
                None,
            )?;
            verify_tmpfs(&mut syscalls, &records_directory)?;
            if records_directory_snapshot.device == health_directory_snapshot.device
                || lock_file_snapshot.0.device != health_directory_snapshot.device
            {
                return Err(Error::InvalidMetadata);
            }
            let health_file = open_leaf(
                &mut syscalls,
                &records_directory,
                HEALTH_FILE,
                HEALTH_FILE_POLICY,
            )?;
            let health_file_snapshot = verify_leaf(
                &mut syscalls,
                &records_directory,
                HEALTH_FILE,
                &health_file,
                HEALTH_FILE_POLICY,
                None,
            )?;
            if health_file_snapshot.0.device != records_directory_snapshot.device {
                return Err(Error::InvalidMetadata);
            }

            Ok(Self {
                syscalls,
                root,
                run,
                health_directory,
                lock_file,
                records_directory,
                health_file,
                root_snapshot,
                run_snapshot,
                health_directory_snapshot,
                lock_file_snapshot,
                records_directory_snapshot,
                health_file_snapshot,
            })
        }

        fn validate(&mut self) -> Result<(), Error> {
            verify_directory(
                &mut self.syscalls,
                &self.root,
                ROOT_NAME,
                &self.root,
                ROOT_POLICY,
                Some(self.root_snapshot),
            )?;
            verify_directory(
                &mut self.syscalls,
                &self.root,
                RUN_DIR,
                &self.run,
                RUN_POLICY,
                Some(self.run_snapshot),
            )?;
            let health_directory = verify_directory(
                &mut self.syscalls,
                &self.run,
                HEALTH_DIR,
                &self.health_directory,
                HEALTH_DIRECTORY_POLICY,
                Some(self.health_directory_snapshot),
            )?;
            let lock_file = verify_leaf(
                &mut self.syscalls,
                &self.health_directory,
                LOCK_FILE,
                &self.lock_file,
                LOCK_FILE_POLICY,
                Some(self.lock_file_snapshot),
            )?;
            let records_directory = verify_directory(
                &mut self.syscalls,
                &self.health_directory,
                RECORDS_DIR,
                &self.records_directory,
                RECORDS_DIRECTORY_POLICY,
                Some(self.records_directory_snapshot),
            )?;
            verify_tmpfs(&mut self.syscalls, &self.records_directory)?;
            if records_directory.device == health_directory.device
                || lock_file.0.device != health_directory.device
            {
                return Err(Error::Changed);
            }
            let health_file = verify_leaf(
                &mut self.syscalls,
                &self.records_directory,
                HEALTH_FILE,
                &self.health_file,
                HEALTH_FILE_POLICY,
                Some(self.health_file_snapshot),
            )?;
            if health_file.0.device != records_directory.device {
                return Err(Error::Changed);
            }
            Ok(())
        }

        fn read_once(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
            self.validate()?;
            let read = read_bounded(
                &mut self.syscalls,
                &self.health_file,
                limit,
                self.health_file_snapshot.0.size,
            );
            let validation = self.validate();
            let bytes = read?;
            validation?;
            Ok(bytes)
        }
    }

    pub(super) fn read_config_real(
        expected_commit: &str,
        expected_config_sha256: &str,
    ) -> Result<SimulationProcessConfig, Error> {
        read_config_with(RealLinuxSyscalls, expected_commit, expected_config_sha256)
    }

    pub(super) fn read_config_with<S: LinuxSyscalls>(
        syscalls: S,
        expected_commit: &str,
        expected_config_sha256: &str,
    ) -> Result<SimulationProcessConfig, Error> {
        if !valid_expected_pins(expected_commit, expected_config_sha256) {
            return Err(Error::ConfigRejected);
        }
        let mut files = ConfigFiles::open(syscalls)?;
        let bytes = files.read_once(CONFIG_SIZE_LIMIT)?;
        let parsed =
            SimulationProcessConfig::from_json(&bytes, expected_commit, expected_config_sha256)
                .map_err(map_config_error);
        let validation = files.validate();
        let config = parsed?;
        validation?;
        Ok(config)
    }

    pub(super) fn check_health_real(config: &SimulationProcessConfig) -> Result<(), Error> {
        check_health_with(RealLinuxSyscalls, &mut PublicObservationFactory, config)
    }

    pub(super) fn check_health_with<S, F>(
        syscalls: S,
        observer_factory: &mut F,
        config: &SimulationProcessConfig,
    ) -> Result<(), Error>
    where
        S: LinuxSyscalls,
        F: ProcessObservationFactory,
    {
        let mut files = HealthFiles::open(syscalls)?;
        let bytes = files.read_once(HEALTH_SIZE_LIMIT)?;
        let tentative_generation = tentative_generation(&bytes)?;
        let parsed = SimulationHealthRecord::from_json(&bytes, config, &tentative_generation)
            .map_err(map_health_error);
        let validation = files.validate();
        let record = parsed?;
        validation?;

        let mut observation = observer_factory
            .observe(record.generation().pid())
            .map_err(map_observation_error)?;
        let generation_matches = observation.generation() == *record.generation();
        files.validate()?;
        if !generation_matches {
            return Err(Error::GenerationInvalid);
        }
        let final_boot_ms = observation.revalidate().map_err(map_observation_error)?;
        record
            .check_ready_at(final_boot_ms)
            .map_err(map_health_error)
    }

    fn tentative_generation(bytes: &[u8]) -> Result<SimulationProcessGeneration, Error> {
        let value = parse_strict_json(bytes).map_err(|_| Error::HealthRejected)?;
        let root = value.as_object().ok_or(Error::HealthRejected)?;
        let generation = root
            .get("generation")
            .and_then(Value::as_object)
            .ok_or(Error::HealthRejected)?;
        if generation.len() != 3
            || !generation.contains_key("pid")
            || !generation.contains_key("start_time_ticks")
            || !generation.contains_key("boot_id")
        {
            return Err(Error::HealthRejected);
        }
        let pid = generation
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(Error::HealthRejected)?;
        let start_time_ticks = generation
            .get("start_time_ticks")
            .and_then(Value::as_u64)
            .ok_or(Error::HealthRejected)?;
        let boot_id_text = generation
            .get("boot_id")
            .and_then(Value::as_str)
            .ok_or(Error::HealthRejected)?;
        let boot_id = Uuid::parse_str(boot_id_text).map_err(|_| Error::HealthRejected)?;
        if boot_id.is_nil() || boot_id.to_string() != boot_id_text {
            return Err(Error::HealthRejected);
        }
        SimulationProcessGeneration::new(pid, start_time_ticks, boot_id)
            .map_err(|_| Error::HealthRejected)
    }

    fn map_config_error(error: super::super::SimulationProcessConfigError) -> Error {
        match error {
            super::super::SimulationProcessConfigError::SizeLimitExceeded => Error::SizeLimit,
            super::super::SimulationProcessConfigError::Invalid => Error::ConfigRejected,
        }
    }

    fn map_health_error(error: SimulationHealthError) -> Error {
        match error {
            SimulationHealthError::SizeLimitExceeded => Error::SizeLimit,
            SimulationHealthError::ClockInvalid => Error::ClockInvalid,
            _ => Error::HealthRejected,
        }
    }

    fn map_observation_error(error: ObservationError) -> Error {
        match error {
            ObservationError::UnsupportedPlatform => Error::UnsupportedPlatform,
            ObservationError::Unavailable => Error::Unavailable,
            ObservationError::ClockInvalid => Error::ClockInvalid,
            _ => Error::GenerationInvalid,
        }
    }

    fn open_root<S: LinuxSyscalls>(syscalls: &mut S) -> Result<S::Handle, Error> {
        syscalls
            .open_path(ROOT_NAME, DIRECTORY_FLAGS)
            .map_err(classify_open_error)
    }

    fn open_directory<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        policy: DirectoryPolicy,
    ) -> Result<S::Handle, Error> {
        let directory = syscalls
            .open_at(parent, name, DIRECTORY_FLAGS)
            .map_err(classify_open_error)?;
        verify_directory(syscalls, parent, name, &directory, policy, None)?;
        Ok(directory)
    }

    fn open_leaf<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        policy: LeafPolicy,
    ) -> Result<S::Handle, Error> {
        let named = syscalls
            .fstat_at(parent, name, NOFOLLOW)
            .map_err(classify_named_error)?;
        validate_leaf_metadata(named, policy)?;
        let leaf = syscalls
            .open_at(parent, name, LEAF_FLAGS)
            .map_err(classify_open_error)?;
        let snapshot = verify_leaf(syscalls, parent, name, &leaf, policy, None)?;
        if snapshot.0 != named {
            return Err(Error::Changed);
        }
        Ok(leaf)
    }

    fn verify_directory<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        directory: &S::Handle,
        policy: DirectoryPolicy,
        expected: Option<DirectorySnapshot>,
    ) -> Result<DirectorySnapshot, Error> {
        let held = syscalls.fstat(directory).map_err(|_| Error::Unavailable)?;
        validate_directory_metadata(held, policy)?;
        check_acls(syscalls, directory)?;
        let named = syscalls
            .fstat_at(parent, name, NOFOLLOW)
            .map_err(classify_named_error)?;
        validate_directory_metadata(named, policy)?;
        if DirectorySnapshot::from(held) != DirectorySnapshot::from(named) {
            return Err(Error::Changed);
        }
        let snapshot = DirectorySnapshot::from(held);
        if expected.is_some_and(|value| value != snapshot) {
            return Err(Error::Changed);
        }
        Ok(snapshot)
    }

    fn verify_leaf<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        leaf: &S::Handle,
        policy: LeafPolicy,
        expected: Option<LeafSnapshot>,
    ) -> Result<LeafSnapshot, Error> {
        let held = syscalls.fstat(leaf).map_err(|_| Error::Unavailable)?;
        validate_leaf_metadata(held, policy)?;
        check_acls(syscalls, leaf)?;
        let named = syscalls
            .fstat_at(parent, name, NOFOLLOW)
            .map_err(classify_named_error)?;
        validate_leaf_metadata(named, policy)?;
        if held != named {
            return Err(Error::Changed);
        }
        let snapshot = LeafSnapshot(held);
        if expected.is_some_and(|value| value != snapshot) {
            return Err(Error::Changed);
        }
        Ok(snapshot)
    }

    #[allow(clippy::unnecessary_cast)] // Keep libc mode normalization explicit.
    fn validate_directory_metadata(
        metadata: Metadata,
        policy: DirectoryPolicy,
    ) -> Result<(), Error> {
        if metadata.mode & libc::S_IFMT as u32 != libc::S_IFDIR as u32 {
            return Err(Error::UnsafePath);
        }
        match policy {
            DirectoryPolicy::RootOrRun => {
                if metadata.uid != 0 || metadata.mode & (0o022 | 0o7000) != 0 {
                    return Err(Error::InvalidMetadata);
                }
            }
            DirectoryPolicy::Fixed {
                uid,
                gid,
                permissions,
            } => {
                if metadata.uid != uid
                    || metadata.gid != gid
                    || metadata.mode & 0o7777 != permissions
                {
                    return Err(Error::InvalidMetadata);
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::unnecessary_cast)] // Keep libc mode normalization explicit.
    fn validate_leaf_metadata(metadata: Metadata, policy: LeafPolicy) -> Result<(), Error> {
        if metadata.mode & libc::S_IFMT as u32 != libc::S_IFREG as u32 {
            return Err(Error::UnsafePath);
        }
        if metadata.uid != policy.uid
            || metadata.gid != policy.gid
            || metadata.mode & 0o7777 != policy.permissions
            || metadata.links != 1
        {
            return Err(Error::InvalidMetadata);
        }
        match policy.size {
            LeafSize::PositiveAtMost(limit) => {
                if metadata.size <= 0 {
                    return Err(Error::InvalidMetadata);
                }
                if u64::try_from(metadata.size).map_or(true, |size| size > limit as u64) {
                    return Err(Error::SizeLimit);
                }
                usize::try_from(metadata.size).map_err(|_| Error::SizeLimit)?;
            }
            LeafSize::ExactlyZero => {
                if metadata.size != 0 {
                    return Err(Error::InvalidMetadata);
                }
            }
        }
        Ok(())
    }

    fn check_acls<S: LinuxSyscalls>(syscalls: &mut S, descriptor: &S::Handle) -> Result<(), Error> {
        for name in [ACCESS_ACL, DEFAULT_ACL] {
            match syscalls.fgetxattr_size(descriptor, name) {
                Ok(None) => {}
                Ok(Some(_)) => return Err(Error::InvalidMetadata),
                Err(error)
                    if error == libc::ENODATA
                        || error == libc::ENOTSUP
                        || error == libc::EOPNOTSUPP => {}
                Err(_) => return Err(Error::Unavailable),
            }
        }
        Ok(())
    }

    fn verify_tmpfs<S: LinuxSyscalls>(syscalls: &mut S, records: &S::Handle) -> Result<(), Error> {
        let filesystem_type = syscalls
            .fstatfs_type(records)
            .map_err(|_| Error::Unavailable)?;
        if filesystem_type != libc::TMPFS_MAGIC as u64 {
            return Err(Error::InvalidMetadata);
        }
        Ok(())
    }

    fn read_bounded<S: LinuxSyscalls>(
        syscalls: &mut S,
        descriptor: &S::Handle,
        limit: usize,
        initial_size: i64,
    ) -> Result<Vec<u8>, Error> {
        let capacity = limit.checked_add(1).ok_or(Error::SizeLimit)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| Error::SizeLimit)?;
        bytes.resize(capacity, 0);
        let mut filled = 0_usize;
        while filled < capacity {
            let remaining = capacity - filled;
            let count = syscalls
                .read(descriptor, &mut bytes[filled..])
                .map_err(|_| Error::Unavailable)?;
            if count == 0 {
                break;
            }
            if count > remaining {
                return Err(Error::Unavailable);
            }
            filled = filled.checked_add(count).ok_or(Error::SizeLimit)?;
        }
        if filled > limit {
            return Err(Error::SizeLimit);
        }
        if i64::try_from(filled).map_err(|_| Error::SizeLimit)? != initial_size {
            return Err(Error::Changed);
        }
        bytes.truncate(filled);
        Ok(bytes)
    }

    fn classify_open_error(error: libc::c_int) -> Error {
        if matches!(error, libc::ELOOP | libc::ENOTDIR | libc::EISDIR) {
            Error::UnsafePath
        } else {
            Error::Unavailable
        }
    }

    fn classify_named_error(error: libc::c_int) -> Error {
        if error == libc::ENOTDIR {
            Error::UnsafePath
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
            let descriptor = unsafe { libc::open(path.as_ptr(), flags) };
            own_opened_fd(descriptor)
        }

        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let descriptor = unsafe { libc::openat(parent.0.as_raw_fd(), name.as_ptr(), flags) };
            own_opened_fd(descriptor)
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<Metadata, libc::c_int> {
            let mut value = MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(descriptor.0.as_raw_fd(), value.as_mut_ptr()) } != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(metadata(&value))
        }

        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Metadata, libc::c_int> {
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
            Ok(metadata(&value))
        }

        fn fgetxattr_size(
            &mut self,
            descriptor: &Self::Handle,
            name: &CStr,
        ) -> Result<Option<usize>, libc::c_int> {
            let result = unsafe {
                libc::fgetxattr(
                    descriptor.0.as_raw_fd(),
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result < 0 {
                Err(last_errno())
            } else {
                Ok(Some(result as usize))
            }
        }

        // libc::statfs::f_type width varies by platform.
        #[allow(clippy::unnecessary_cast)]
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
                Ok(result as usize)
            }
        }
    }

    fn own_opened_fd(descriptor: RawFd) -> Result<LinuxHandle, libc::c_int> {
        if descriptor < 0 {
            Err(last_errno())
        } else {
            Ok(LinuxHandle(unsafe { OwnedFd::from_raw_fd(descriptor) }))
        }
    }

    fn last_errno() -> libc::c_int {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    #[allow(clippy::unnecessary_cast)] // libc field widths are platform dependent.
    fn metadata(value: &libc::stat) -> Metadata {
        Metadata {
            device: value.st_dev as u64,
            inode: value.st_ino as u64,
            mode: value.st_mode as u32,
            uid: value.st_uid,
            gid: value.st_gid,
            links: value.st_nlink as u64,
            size: value.st_size as i64,
            mtime_seconds: value.st_mtime as i64,
            mtime_nanoseconds: value.st_mtime_nsec as i64,
            ctime_seconds: value.st_ctime as i64,
            ctime_nanoseconds: value.st_ctime_nsec as i64,
        }
    }
}

#[cfg(test)]
mod tests;
