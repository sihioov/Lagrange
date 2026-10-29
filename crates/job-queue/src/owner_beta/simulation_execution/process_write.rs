use std::fmt;

use super::{SimulationHealthEvent, SimulationProcessConfig};
#[cfg(target_os = "linux")]
use super::{SimulationHealthRecord, SimulationHealthTracker, SimulationProcessGeneration};

/// Value-free failures at the fixed simulation health writer boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SimulationHealthWriteError {
    #[error("the simulation health writer is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the simulation health writer requires the configured identity")]
    InvalidIdentity,
    #[error("the fixed simulation health path is unsafe")]
    UnsafePath,
    #[error("fixed simulation health metadata is invalid")]
    InvalidMetadata,
    #[error("the simulation health records directory is not fresh")]
    DirtyRecords,
    #[error("the simulation health writer lock is unavailable")]
    LockUnavailable,
    #[error("the simulation health writer observed a changed object")]
    Changed,
    #[error("a simulation health writer operation is unavailable")]
    Unavailable,
    #[error("a simulation health writer value exceeds its size limit")]
    SizeLimit,
    #[error("the current simulation process generation is invalid")]
    GenerationInvalid,
    #[error("the current simulation process clock is invalid")]
    ClockInvalid,
    #[error("the simulation health event or record is rejected")]
    HealthRejected,
    #[error("simulation health publication is unconfirmed")]
    PublicationUnconfirmed,
    #[error("the simulation health writer is permanently invalidated")]
    Invalidated,
}

trait WriterSession {
    fn apply(&mut self, event: SimulationHealthEvent) -> Result<(), SimulationHealthWriteError>;
}

/// Opaque, lifetime-locked writer for one observed simulation process.
pub struct SimulationHealthWriter {
    session: Box<dyn WriterSession>,
}

impl fmt::Debug for SimulationHealthWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SimulationHealthWriter")
    }
}

impl SimulationHealthWriter {
    /// Applies one verified health event and atomically publishes the result.
    ///
    /// A failed call permanently invalidates this writer. Its lock and owned
    /// descriptors remain held until the writer is dropped.
    pub fn apply(
        &mut self,
        event: SimulationHealthEvent,
    ) -> Result<(), SimulationHealthWriteError> {
        self.session.apply(event)
    }
}

/// Opens the single fixed-path writer after checking the caller and the
/// retained image-provisioned health tree.
#[cfg(target_os = "linux")]
pub fn open_simulation_health_writer(
    config: &SimulationProcessConfig,
) -> Result<SimulationHealthWriter, SimulationHealthWriteError> {
    linux::open_public(config)
}

/// Non-Linux platforms fail before inspecting the checked config or doing I/O.
#[cfg(not(target_os = "linux"))]
pub fn open_simulation_health_writer(
    _config: &SimulationProcessConfig,
) -> Result<SimulationHealthWriter, SimulationHealthWriteError> {
    Err(SimulationHealthWriteError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::HashSet,
        ffi::CStr,
        mem::MaybeUninit,
        os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    };

    use super::super::SimulationProcessObservationError as ObservationError;
    use super::{
        SimulationHealthEvent, SimulationHealthRecord, SimulationHealthTracker,
        SimulationHealthWriteError as Error, SimulationHealthWriter, SimulationProcessConfig,
        SimulationProcessGeneration, WriterSession,
    };

    const HEALTH_ROOT_NAME: &CStr = c"owner-beta-simulation-health";
    const LOCK_NAME: &CStr = c"writer.lock";
    const RECORDS_NAME: &CStr = c"records";
    const CURRENT_NAME: &CStr = c"current.json";
    const PENDING_NAME: &CStr = c".pending.json";
    const RUN_NAME: &CStr = c"run";
    const ROOT_NAME: &CStr = c"/";
    const ACCESS_ACL: &CStr = c"system.posix_acl_access";
    const DEFAULT_ACL: &CStr = c"system.posix_acl_default";

    const DIRECTORY_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const LOCK_FLAGS: libc::c_int =
        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const PENDING_FLAGS: libc::c_int = libc::O_RDWR
        | libc::O_CREAT
        | libc::O_EXCL
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_CLOEXEC;
    const NOFOLLOW: libc::c_int = libc::AT_SYMLINK_NOFOLLOW;
    const ENUMERATION_CALL_BYTES: usize = 1_024;
    const ENUMERATION_TOTAL_BYTES: usize = 8_192;
    const RECORD_SIZE_LIMIT: usize = 8_192;
    const RECORD_READ_LIMIT: usize = RECORD_SIZE_LIMIT + 1;
    const TMPFS_TYPE: u64 = libc::TMPFS_MAGIC as u64;
    const RENAME_NOREPLACE: libc::c_uint = libc::RENAME_NOREPLACE as libc::c_uint;

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
    pub(super) struct LeafPolicy {
        uid: u32,
        gid: u32,
        permissions: u32,
        size: LeafSize,
    }

    #[derive(Clone, Copy)]
    enum LeafSize {
        ExactlyZero,
        PositiveAtMost(usize),
    }

    const ROOT_POLICY: DirectoryPolicy = DirectoryPolicy::RootOrRun;
    const RUN_POLICY: DirectoryPolicy = DirectoryPolicy::RootOrRun;
    const HEALTH_POLICY: DirectoryPolicy = DirectoryPolicy::Fixed {
        uid: 0,
        gid: 10001,
        permissions: 0o750,
    };
    const RECORDS_POLICY: DirectoryPolicy = DirectoryPolicy::Fixed {
        uid: 10001,
        gid: 10001,
        permissions: 0o700,
    };
    const LOCK_POLICY: LeafPolicy = LeafPolicy {
        uid: 0,
        gid: 10001,
        permissions: 0o440,
        size: LeafSize::ExactlyZero,
    };
    const EMPTY_DATA_POLICY: LeafPolicy = LeafPolicy {
        uid: 10001,
        gid: 10001,
        permissions: 0o600,
        size: LeafSize::ExactlyZero,
    };
    pub(super) const RECORD_POLICY: LeafPolicy = LeafPolicy {
        uid: 10001,
        gid: 10001,
        permissions: 0o600,
        size: LeafSize::PositiveAtMost(RECORD_SIZE_LIMIT),
    };

    /// All filesystem, lock, inventory, and publication calls cross this
    /// private seam. Production and fake regression inputs share WriterPolicy.
    pub(super) trait LinuxSyscalls {
        type Handle;

        fn effective_ids(&mut self) -> (u32, u32);
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
            mode: libc::mode_t,
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
        fn flock_exclusive_nonblocking(
            &mut self,
            descriptor: &Self::Handle,
        ) -> Result<(), libc::c_int>;
        fn seek_directory_start(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int>;
        fn getdents64(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int>;
        fn write(&mut self, descriptor: &Self::Handle, bytes: &[u8]) -> Result<usize, libc::c_int>;
        fn pread(
            &mut self,
            descriptor: &Self::Handle,
            offset: i64,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int>;
        fn fsync(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int>;
        fn rename_at(
            &mut self,
            old_parent: &Self::Handle,
            old_name: &CStr,
            new_parent: &Self::Handle,
            new_name: &CStr,
            no_replace: bool,
        ) -> Result<(), libc::c_int>;
    }

    pub(super) trait ObservationHandle {
        fn generation(&self) -> SimulationProcessGeneration;
        fn initial_boot_ms(&self) -> u64;
        fn revalidate(&mut self) -> Result<u64, ObservationError>;
    }

    pub(super) trait ObservationFactory {
        type Observation: ObservationHandle;

        fn observe_current(&mut self) -> Result<Self::Observation, ObservationError>;
    }

    struct PublicObservationFactory;

    impl ObservationHandle for super::super::SimulationProcessObservation {
        fn generation(&self) -> SimulationProcessGeneration {
            super::super::SimulationProcessObservation::generation(self)
        }

        fn initial_boot_ms(&self) -> u64 {
            super::super::SimulationProcessObservation::observed_boot_ms(self)
        }

        fn revalidate(&mut self) -> Result<u64, ObservationError> {
            super::super::SimulationProcessObservation::revalidate(self)
        }
    }

    impl ObservationFactory for PublicObservationFactory {
        type Observation = super::super::SimulationProcessObservation;

        fn observe_current(&mut self) -> Result<Self::Observation, ObservationError> {
            super::super::observe_current_simulation_process()
        }
    }

    struct LinuxHandle(OwnedFd);

    struct RealLinuxSyscalls;

    struct Tree<S: LinuxSyscalls> {
        syscalls: S,
        root: S::Handle,
        run: S::Handle,
        health: S::Handle,
        lock: S::Handle,
        records: S::Handle,
        root_snapshot: DirectorySnapshot,
        run_snapshot: DirectorySnapshot,
        health_snapshot: DirectorySnapshot,
        lock_snapshot: LeafSnapshot,
        records_snapshot: DirectorySnapshot,
    }

    struct LastPublication<H> {
        descriptor: H,
        snapshot: LeafSnapshot,
        record: SimulationHealthRecord,
        bytes: Vec<u8>,
    }

    pub(super) struct WriterPolicy<S: LinuxSyscalls, O: ObservationHandle> {
        tree: Tree<S>,
        config: SimulationProcessConfig,
        observer: O,
        generation: SimulationProcessGeneration,
        tracker: SimulationHealthTracker,
        last: Option<LastPublication<S::Handle>>,
        invalidated: bool,
    }

    pub(super) fn open_public(
        config: &SimulationProcessConfig,
    ) -> Result<SimulationHealthWriter, Error> {
        let policy = open_with(RealLinuxSyscalls, PublicObservationFactory, config)?;
        Ok(SimulationHealthWriter {
            session: Box::new(policy),
        })
    }

    pub(super) fn open_with<S, F>(
        mut syscalls: S,
        mut observer_factory: F,
        config: &SimulationProcessConfig,
    ) -> Result<WriterPolicy<S, F::Observation>, Error>
    where
        S: LinuxSyscalls,
        F: ObservationFactory,
    {
        check_identity(&mut syscalls)?;
        let mut tree = Tree::open(syscalls)?;
        tree.require_inventory(&[])?;
        tree.acquire_lock()?;
        tree.validate()?;
        tree.require_inventory(&[])?;

        let observer = observer_factory
            .observe_current()
            .map_err(map_observation_error)?;
        let generation = observer.generation();
        let started_boot_ms = observer.initial_boot_ms();
        let tracker = SimulationHealthTracker::new(config, generation, started_boot_ms);
        let mut writer = WriterPolicy {
            tree,
            config: config.clone(),
            observer,
            generation,
            tracker,
            last: None,
            invalidated: false,
        };
        writer.publish()?;
        Ok(writer)
    }

    impl<S, O> WriterSession for WriterPolicy<S, O>
    where
        S: LinuxSyscalls + 'static,
        S::Handle: 'static,
        O: ObservationHandle + 'static,
    {
        fn apply(&mut self, event: SimulationHealthEvent) -> Result<(), Error> {
            if self.invalidated {
                return Err(Error::Invalidated);
            }
            let result = self.apply_inner(event);
            if result.is_err() {
                self.invalidated = true;
            }
            result
        }
    }

    impl<S: LinuxSyscalls, O: ObservationHandle> WriterPolicy<S, O> {
        fn apply_inner(&mut self, event: SimulationHealthEvent) -> Result<(), Error> {
            check_identity(&mut self.tree.syscalls)?;
            self.tree.validate()?;
            self.validate_last()?;
            self.require_current_inventory()?;

            let observed_boot_ms = self.revalidate_observer()?;
            self.tracker
                .apply(event, observed_boot_ms)
                .map_err(map_health_error)?;
            self.publish()
        }

        fn validate_last(&mut self) -> Result<(), Error> {
            if let Some(last) = self.last.as_ref() {
                self.tree.verify_record(
                    CURRENT_NAME,
                    &last.descriptor,
                    Some(last.snapshot),
                    &self.config,
                    &self.generation,
                    &last.record,
                    Some(&last.bytes),
                )?;
            } else {
                return Err(Error::Changed);
            }
            Ok(())
        }

        fn require_current_inventory(&mut self) -> Result<(), Error> {
            self.tree.require_inventory(&[b"current.json"])?;
            Ok(())
        }

        fn revalidate_observer(&mut self) -> Result<u64, Error> {
            let observed = self.observer.revalidate().map_err(map_observation_error)?;
            if self.observer.generation() != self.generation {
                return Err(Error::GenerationInvalid);
            }
            if observed < self.tracker.record().observed_boot_ms() {
                return Err(Error::ClockInvalid);
            }
            Ok(observed)
        }

        fn publish(&mut self) -> Result<(), Error> {
            let record = self.tracker.record();
            let bytes = record.to_json().map_err(map_health_error)?;
            if bytes.is_empty() || bytes.len() > RECORD_SIZE_LIMIT {
                return Err(Error::SizeLimit);
            }

            self.tree.validate()?;
            self.validate_last_for_publish()?;
            self.require_pre_create_inventory()?;

            let temporary = self.tree.create_pending()?;
            let empty_snapshot = self
                .tree
                .verify_empty_data(PENDING_NAME, &temporary, None)?;
            let written = write_all(&mut self.tree.syscalls, &temporary, &bytes)?;
            if written != bytes.len() {
                return Err(Error::Unavailable);
            }
            self.tree
                .syscalls
                .fsync(&temporary)
                .map_err(|_| Error::Unavailable)?;
            let temporary_snapshot = self.tree.verify_record(
                PENDING_NAME,
                &temporary,
                None,
                &self.config,
                &self.generation,
                &record,
                Some(bytes.as_slice()),
            )?;
            if empty_snapshot.0.device != temporary_snapshot.0.device {
                return Err(Error::Changed);
            }
            self.tree.validate()?;
            self.require_pre_rename_inventory()?;
            if let Some(last) = self.last.as_ref() {
                self.tree.verify_record(
                    CURRENT_NAME,
                    &last.descriptor,
                    Some(last.snapshot),
                    &self.config,
                    &self.generation,
                    &last.record,
                    Some(&last.bytes),
                )?;
            }
            self.tree.verify_record(
                PENDING_NAME,
                &temporary,
                Some(temporary_snapshot),
                &self.config,
                &self.generation,
                &record,
                Some(bytes.as_slice()),
            )?;
            self.revalidate_observer()?;

            let rename_result = self.tree.syscalls.rename_at(
                &self.tree.records,
                PENDING_NAME,
                &self.tree.records,
                CURRENT_NAME,
                self.last.is_none(),
            );
            if rename_result.is_err() {
                return Err(Error::PublicationUnconfirmed);
            }

            let published = self.finish_publication(temporary, temporary_snapshot, &record, &bytes);
            match published {
                Ok(publication) => {
                    self.last = Some(publication);
                    Ok(())
                }
                Err(_) => Err(Error::PublicationUnconfirmed),
            }
        }

        fn validate_last_for_publish(&mut self) -> Result<(), Error> {
            match self.last.as_ref() {
                Some(last) => {
                    self.tree.verify_record(
                        CURRENT_NAME,
                        &last.descriptor,
                        Some(last.snapshot),
                        &self.config,
                        &self.generation,
                        &last.record,
                        Some(&last.bytes),
                    )?;
                    Ok(())
                }
                None => Ok(()),
            }
        }

        fn require_pre_create_inventory(&mut self) -> Result<(), Error> {
            if self.last.is_some() {
                self.tree.require_inventory(&[b"current.json"])
            } else {
                self.tree.require_inventory(&[])
            }
        }

        fn require_pre_rename_inventory(&mut self) -> Result<(), Error> {
            if self.last.is_some() {
                self.tree
                    .require_inventory(&[b"current.json", b".pending.json"])
            } else {
                self.tree.require_inventory(&[b".pending.json"])
            }
        }

        fn finish_publication(
            &mut self,
            temporary: S::Handle,
            _temporary_snapshot: LeafSnapshot,
            record: &SimulationHealthRecord,
            bytes: &[u8],
        ) -> Result<LastPublication<S::Handle>, Error> {
            self.tree
                .syscalls
                .fsync(&self.tree.records)
                .map_err(|_| Error::Unavailable)?;
            self.tree.validate()?;
            self.tree.require_inventory(&[b"current.json"])?;
            let snapshot = self.tree.verify_record(
                CURRENT_NAME,
                &temporary,
                None,
                &self.config,
                &self.generation,
                record,
                Some(bytes),
            )?;
            self.revalidate_observer()?;
            Ok(LastPublication {
                descriptor: temporary,
                snapshot,
                record: record.clone(),
                bytes: bytes.to_vec(),
            })
        }
    }

    impl<S: LinuxSyscalls, O: ObservationHandle> WriterPolicy<S, O> {
        #[cfg(test)]
        pub(super) fn is_invalidated(&self) -> bool {
            self.invalidated
        }
    }

    impl<S: LinuxSyscalls> Tree<S> {
        fn open(mut syscalls: S) -> Result<Self, Error> {
            let root = syscalls
                .open_path(ROOT_NAME, DIRECTORY_FLAGS)
                .map_err(classify_path_error)?;
            let root_snapshot =
                verify_directory(&mut syscalls, &root, ROOT_NAME, &root, ROOT_POLICY, None)?;
            let run = open_directory(&mut syscalls, &root, RUN_NAME, RUN_POLICY)?;
            let run_snapshot =
                verify_directory(&mut syscalls, &root, RUN_NAME, &run, RUN_POLICY, None)?;
            let health = open_directory(&mut syscalls, &run, HEALTH_ROOT_NAME, HEALTH_POLICY)?;
            let health_snapshot = verify_directory(
                &mut syscalls,
                &run,
                HEALTH_ROOT_NAME,
                &health,
                HEALTH_POLICY,
                None,
            )?;
            let lock = open_leaf(&mut syscalls, &health, LOCK_NAME, LOCK_POLICY)?;
            let lock_snapshot =
                verify_leaf(&mut syscalls, &health, LOCK_NAME, &lock, LOCK_POLICY, None)?;
            let records = open_directory(&mut syscalls, &health, RECORDS_NAME, RECORDS_POLICY)?;
            let records_snapshot = verify_directory(
                &mut syscalls,
                &health,
                RECORDS_NAME,
                &records,
                RECORDS_POLICY,
                None,
            )?;
            verify_tmpfs(&mut syscalls, &records, false)?;
            if records_snapshot.device == health_snapshot.device
                || lock_snapshot.0.device != health_snapshot.device
            {
                return Err(Error::InvalidMetadata);
            }
            Ok(Self {
                syscalls,
                root,
                run,
                health,
                lock,
                records,
                root_snapshot,
                run_snapshot,
                health_snapshot,
                lock_snapshot,
                records_snapshot,
            })
        }

        fn acquire_lock(&mut self) -> Result<(), Error> {
            match self.syscalls.flock_exclusive_nonblocking(&self.lock) {
                Ok(()) => Ok(()),
                Err(error) if error == libc::EAGAIN || error == libc::EWOULDBLOCK => {
                    Err(Error::LockUnavailable)
                }
                Err(_) => Err(Error::Unavailable),
            }
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
                RUN_NAME,
                &self.run,
                RUN_POLICY,
                Some(self.run_snapshot),
            )?;
            let health = verify_directory(
                &mut self.syscalls,
                &self.run,
                HEALTH_ROOT_NAME,
                &self.health,
                HEALTH_POLICY,
                Some(self.health_snapshot),
            )?;
            let lock = verify_leaf(
                &mut self.syscalls,
                &self.health,
                LOCK_NAME,
                &self.lock,
                LOCK_POLICY,
                Some(self.lock_snapshot),
            )?;
            let records = verify_directory(
                &mut self.syscalls,
                &self.health,
                RECORDS_NAME,
                &self.records,
                RECORDS_POLICY,
                Some(self.records_snapshot),
            )?;
            verify_tmpfs(&mut self.syscalls, &self.records, true)?;
            if records.device == health.device || lock.0.device != health.device {
                return Err(Error::Changed);
            }
            Ok(())
        }

        fn require_inventory(&mut self, expected_files: &[&[u8]]) -> Result<(), Error> {
            let observed = inventory(&mut self.syscalls, &self.records)?;
            let mut expected = HashSet::from([b".".to_vec(), b"..".to_vec()]);
            for name in expected_files {
                expected.insert(name.to_vec());
            }
            if observed != expected {
                return Err(Error::DirtyRecords);
            }
            Ok(())
        }

        fn create_pending(&mut self) -> Result<S::Handle, Error> {
            self.syscalls
                .open_at(&self.records, PENDING_NAME, PENDING_FLAGS, 0o600)
                .map_err(classify_path_error)
        }

        fn verify_empty_data(
            &mut self,
            name: &CStr,
            descriptor: &S::Handle,
            expected: Option<LeafSnapshot>,
        ) -> Result<LeafSnapshot, Error> {
            let snapshot = verify_leaf(
                &mut self.syscalls,
                &self.records,
                name,
                descriptor,
                EMPTY_DATA_POLICY,
                expected,
            )?;
            if snapshot.0.device != self.records_snapshot.device {
                return Err(Error::InvalidMetadata);
            }
            Ok(snapshot)
        }

        #[allow(clippy::too_many_arguments)]
        fn verify_record(
            &mut self,
            name: &CStr,
            descriptor: &S::Handle,
            expected_snapshot: Option<LeafSnapshot>,
            config: &SimulationProcessConfig,
            generation: &SimulationProcessGeneration,
            record: &SimulationHealthRecord,
            expected_bytes: Option<&[u8]>,
        ) -> Result<LeafSnapshot, Error> {
            let snapshot = verify_leaf(
                &mut self.syscalls,
                &self.records,
                name,
                descriptor,
                RECORD_POLICY,
                expected_snapshot,
            )?;
            if snapshot.0.device != self.records_snapshot.device {
                return Err(Error::InvalidMetadata);
            }
            let bytes = pread_bounded(&mut self.syscalls, descriptor)?;
            if i64::try_from(bytes.len()).map_err(|_| Error::SizeLimit)? != snapshot.0.size {
                return Err(Error::Changed);
            }
            if expected_bytes.is_some_and(|expected| expected != bytes) {
                return Err(Error::Changed);
            }
            let parsed = SimulationHealthRecord::from_json(&bytes, config, generation)
                .map_err(map_health_error)?;
            if parsed != *record {
                return Err(Error::HealthRejected);
            }
            verify_leaf(
                &mut self.syscalls,
                &self.records,
                name,
                descriptor,
                RECORD_POLICY,
                Some(snapshot),
            )
        }
    }

    fn check_identity<S: LinuxSyscalls>(syscalls: &mut S) -> Result<(), Error> {
        if syscalls.effective_ids() != (10001, 10001) {
            return Err(Error::InvalidIdentity);
        }
        Ok(())
    }

    fn open_directory<S: LinuxSyscalls>(
        syscalls: &mut S,
        parent: &S::Handle,
        name: &CStr,
        policy: DirectoryPolicy,
    ) -> Result<S::Handle, Error> {
        let directory = syscalls
            .open_at(parent, name, DIRECTORY_FLAGS, 0)
            .map_err(classify_path_error)?;
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
            .map_err(classify_path_error)?;
        validate_leaf_metadata(named, policy)?;
        let leaf = syscalls
            .open_at(parent, name, LOCK_FLAGS, 0)
            .map_err(classify_path_error)?;
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
        descriptor: &S::Handle,
        policy: DirectoryPolicy,
        expected: Option<DirectorySnapshot>,
    ) -> Result<DirectorySnapshot, Error> {
        let held = syscalls.fstat(descriptor).map_err(|_| Error::Unavailable)?;
        validate_directory_metadata(held, policy)?;
        check_acls(syscalls, descriptor)?;
        let named = syscalls
            .fstat_at(parent, name, NOFOLLOW)
            .map_err(classify_path_error)?;
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
        descriptor: &S::Handle,
        policy: LeafPolicy,
        expected: Option<LeafSnapshot>,
    ) -> Result<LeafSnapshot, Error> {
        let held = syscalls.fstat(descriptor).map_err(|_| Error::Unavailable)?;
        validate_leaf_metadata(held, policy)?;
        check_acls(syscalls, descriptor)?;
        let named = syscalls
            .fstat_at(parent, name, NOFOLLOW)
            .map_err(classify_path_error)?;
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
    pub(super) fn validate_leaf_metadata(
        metadata: Metadata,
        policy: LeafPolicy,
    ) -> Result<(), Error> {
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
            LeafSize::ExactlyZero if metadata.size != 0 => Err(Error::InvalidMetadata),
            LeafSize::PositiveAtMost(_) if metadata.size <= 0 => Err(Error::InvalidMetadata),
            LeafSize::PositiveAtMost(limit)
                if u64::try_from(metadata.size).map_or(true, |size| size > limit as u64) =>
            {
                Err(Error::SizeLimit)
            }
            _ => Ok(()),
        }
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

    fn verify_tmpfs<S: LinuxSyscalls>(
        syscalls: &mut S,
        records: &S::Handle,
        existing: bool,
    ) -> Result<(), Error> {
        let filesystem_type = syscalls
            .fstatfs_type(records)
            .map_err(|_| Error::Unavailable)?;
        if filesystem_type != TMPFS_TYPE {
            return Err(if existing {
                Error::Changed
            } else {
                Error::InvalidMetadata
            });
        }
        Ok(())
    }

    fn inventory<S: LinuxSyscalls>(
        syscalls: &mut S,
        directory: &S::Handle,
    ) -> Result<HashSet<Vec<u8>>, Error> {
        syscalls
            .seek_directory_start(directory)
            .map_err(|_| Error::Unavailable)?;
        let mut total = 0_usize;
        let mut names = HashSet::new();
        loop {
            let remaining = ENUMERATION_TOTAL_BYTES.saturating_sub(total);
            let request = if remaining == 0 {
                1
            } else {
                remaining.min(ENUMERATION_CALL_BYTES)
            };
            let mut buffer = vec![0_u8; request];
            let count = match syscalls.getdents64(directory, &mut buffer) {
                Ok(count) => count,
                Err(error) if error == libc::EINVAL && remaining < ENUMERATION_CALL_BYTES => {
                    return Err(Error::SizeLimit);
                }
                Err(_) => return Err(Error::Unavailable),
            };
            if count == 0 {
                return Ok(names);
            }
            if count > request || remaining == 0 || count > remaining {
                return Err(Error::SizeLimit);
            }
            parse_dirent64(&buffer[..count], &mut names)?;
            total = total.checked_add(count).ok_or(Error::SizeLimit)?;
        }
    }

    pub(super) fn parse_dirent64(bytes: &[u8], names: &mut HashSet<Vec<u8>>) -> Result<(), Error> {
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let header_end = offset.checked_add(19).ok_or(Error::InvalidMetadata)?;
            if header_end > bytes.len() {
                return Err(Error::InvalidMetadata);
            }
            let record_length =
                usize::from(u16::from_ne_bytes([bytes[offset + 16], bytes[offset + 17]]));
            if record_length < 20 {
                return Err(Error::InvalidMetadata);
            }
            let record_end = offset
                .checked_add(record_length)
                .ok_or(Error::InvalidMetadata)?;
            if record_end > bytes.len() {
                return Err(Error::InvalidMetadata);
            }
            let name_start = offset + 19;
            let terminator = bytes[name_start..record_end]
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(Error::InvalidMetadata)?;
            let name = &bytes[name_start..name_start + terminator];
            if name.is_empty() || name.contains(&b'/') {
                return Err(Error::InvalidMetadata);
            }
            if !names.insert(name.to_vec()) {
                return Err(Error::InvalidMetadata);
            }
            offset = record_end;
        }
        if offset != bytes.len() {
            return Err(Error::InvalidMetadata);
        }
        Ok(())
    }

    fn write_all<S: LinuxSyscalls>(
        syscalls: &mut S,
        descriptor: &S::Handle,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        let mut written = 0_usize;
        while written < bytes.len() {
            let count = syscalls
                .write(descriptor, &bytes[written..])
                .map_err(|_| Error::Unavailable)?;
            if count == 0 || count > bytes.len() - written {
                return Err(Error::Unavailable);
            }
            written = written.checked_add(count).ok_or(Error::SizeLimit)?;
            if written > RECORD_SIZE_LIMIT {
                return Err(Error::SizeLimit);
            }
        }
        Ok(written)
    }

    fn pread_bounded<S: LinuxSyscalls>(
        syscalls: &mut S,
        descriptor: &S::Handle,
    ) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(RECORD_READ_LIMIT)
            .map_err(|_| Error::SizeLimit)?;
        let mut offset = 0_usize;
        while offset < RECORD_READ_LIMIT {
            let request = (RECORD_READ_LIMIT - offset).min(ENUMERATION_CALL_BYTES);
            let mut buffer = vec![0_u8; request];
            let count = syscalls
                .pread(
                    descriptor,
                    i64::try_from(offset).map_err(|_| Error::SizeLimit)?,
                    &mut buffer,
                )
                .map_err(|_| Error::Unavailable)?;
            if count == 0 {
                break;
            }
            if count > request {
                return Err(Error::Unavailable);
            }
            bytes.extend_from_slice(&buffer[..count]);
            offset = offset.checked_add(count).ok_or(Error::SizeLimit)?;
        }
        if bytes.len() > RECORD_SIZE_LIMIT {
            return Err(Error::SizeLimit);
        }
        if bytes.is_empty() {
            return Err(Error::HealthRejected);
        }
        Ok(bytes)
    }

    pub(super) fn map_health_error(error: super::super::SimulationHealthError) -> Error {
        match error {
            super::super::SimulationHealthError::SizeLimitExceeded => Error::SizeLimit,
            super::super::SimulationHealthError::ClockInvalid => Error::ClockInvalid,
            _ => Error::HealthRejected,
        }
    }

    fn map_observation_error(error: ObservationError) -> Error {
        if error == ObservationError::ClockInvalid {
            Error::ClockInvalid
        } else {
            Error::GenerationInvalid
        }
    }

    fn classify_path_error(error: libc::c_int) -> Error {
        if matches!(error, libc::ELOOP | libc::ENOTDIR | libc::EISDIR) {
            Error::UnsafePath
        } else {
            Error::Unavailable
        }
    }

    impl LinuxSyscalls for RealLinuxSyscalls {
        type Handle = LinuxHandle;

        fn effective_ids(&mut self) -> (u32, u32) {
            unsafe { (libc::geteuid(), libc::getegid()) }
        }

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
            mode: libc::mode_t,
        ) -> Result<Self::Handle, libc::c_int> {
            let descriptor = unsafe {
                libc::openat(
                    parent.0.as_raw_fd(),
                    name.as_ptr(),
                    flags,
                    mode as libc::c_uint,
                )
            };
            own_opened_fd(descriptor)
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<Metadata, libc::c_int> {
            let mut value = MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(descriptor.0.as_raw_fd(), value.as_mut_ptr()) } != 0 {
                return Err(last_errno());
            }
            Ok(metadata(&unsafe { value.assume_init() }))
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
            Ok(metadata(&unsafe { value.assume_init() }))
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

        #[allow(clippy::unnecessary_cast)] // libc statfs type varies by target.
        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int> {
            let mut value = MaybeUninit::<libc::statfs>::uninit();
            if unsafe { libc::fstatfs(descriptor.0.as_raw_fd(), value.as_mut_ptr()) } != 0 {
                return Err(last_errno());
            }
            let value = unsafe { value.assume_init() };
            Ok(value.f_type as u64)
        }

        fn flock_exclusive_nonblocking(
            &mut self,
            descriptor: &Self::Handle,
        ) -> Result<(), libc::c_int> {
            if unsafe { libc::flock(descriptor.0.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
            {
                Err(last_errno())
            } else {
                Ok(())
            }
        }

        fn seek_directory_start(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int> {
            if unsafe { libc::lseek(descriptor.0.as_raw_fd(), 0, libc::SEEK_SET) } != 0 {
                Err(last_errno())
            } else {
                Ok(())
            }
        }

        fn getdents64(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let result = unsafe {
                libc::syscall(
                    libc::SYS_getdents64,
                    descriptor.0.as_raw_fd(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                )
            };
            if result < 0 {
                Err(last_errno())
            } else {
                usize::try_from(result).map_err(|_| libc::EIO)
            }
        }

        fn write(&mut self, descriptor: &Self::Handle, bytes: &[u8]) -> Result<usize, libc::c_int> {
            let result = unsafe {
                libc::write(descriptor.0.as_raw_fd(), bytes.as_ptr().cast(), bytes.len())
            };
            if result < 0 {
                Err(last_errno())
            } else {
                usize::try_from(result).map_err(|_| libc::EIO)
            }
        }

        fn pread(
            &mut self,
            descriptor: &Self::Handle,
            offset: i64,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let offset = libc::off_t::try_from(offset).map_err(|_| libc::EINVAL)?;
            let result = unsafe {
                libc::pread(
                    descriptor.0.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    offset,
                )
            };
            if result < 0 {
                Err(last_errno())
            } else {
                usize::try_from(result).map_err(|_| libc::EIO)
            }
        }

        fn fsync(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int> {
            if unsafe { libc::fsync(descriptor.0.as_raw_fd()) } != 0 {
                Err(last_errno())
            } else {
                Ok(())
            }
        }

        fn rename_at(
            &mut self,
            old_parent: &Self::Handle,
            old_name: &CStr,
            new_parent: &Self::Handle,
            new_name: &CStr,
            no_replace: bool,
        ) -> Result<(), libc::c_int> {
            let result = if no_replace {
                unsafe {
                    libc::syscall(
                        libc::SYS_renameat2,
                        old_parent.0.as_raw_fd(),
                        old_name.as_ptr(),
                        new_parent.0.as_raw_fd(),
                        new_name.as_ptr(),
                        RENAME_NOREPLACE,
                    )
                }
            } else {
                unsafe {
                    libc::renameat(
                        old_parent.0.as_raw_fd(),
                        old_name.as_ptr(),
                        new_parent.0.as_raw_fd(),
                        new_name.as_ptr(),
                    ) as libc::c_long
                }
            };
            if result != 0 {
                Err(last_errno())
            } else {
                Ok(())
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
