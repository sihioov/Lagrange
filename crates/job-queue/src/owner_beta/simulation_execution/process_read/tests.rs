use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{SimulationProcessConfig, SimulationProcessReadError as Error};

const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const VALID_CONFIG: &str = r###"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"###;

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn checked_config() -> (Vec<u8>, String, SimulationProcessConfig) {
    let bytes = VALID_CONFIG.as_bytes().to_vec();
    let hash = digest(&bytes);
    let config = SimulationProcessConfig::from_json(&bytes, TEST_COMMIT, &hash)
        .expect("synthetic C3A config is valid");
    (bytes, hash, config)
}

#[cfg(target_os = "linux")]
mod linux_tests {
    use std::{
        cell::{Cell, RefCell},
        collections::{HashMap, HashSet},
        ffi::CStr,
        rc::Rc,
    };

    use super::super::linux::{
        self, DIRECTORY_FLAGS, LEAF_FLAGS, LinuxSyscalls, Metadata, NOFOLLOW,
        ProcessObservationFactory, ProcessObservationHandle,
    };
    use super::super::{CONFIG_SIZE_LIMIT, HEALTH_SIZE_LIMIT};
    use super::*;
    use crate::owner_beta::simulation_execution::{
        SimulationHealthEvent, SimulationHealthFailure, SimulationHealthTracker,
        SimulationProcessGeneration, SimulationProcessObservationError,
    };

    #[derive(Clone)]
    struct FakeNode {
        path: String,
        metadata: Metadata,
        bytes: Vec<u8>,
        read_offset: usize,
        max_read: usize,
        read_error: Option<libc::c_int>,
        acls: HashMap<String, Result<Option<usize>, libc::c_int>>,
        filesystem_type: u64,
    }

    impl FakeNode {
        fn new(path: &str, metadata: Metadata, bytes: Vec<u8>) -> Self {
            Self {
                path: path.to_owned(),
                metadata,
                bytes,
                read_offset: 0,
                max_read: usize::MAX,
                read_error: None,
                acls: HashMap::new(),
                filesystem_type: 0,
            }
        }
    }

    enum Mutation {
        Metadata { path: String, metadata: Metadata },
        Replace { path: String, node: FakeNode },
    }

    #[derive(Default)]
    struct FakeState {
        entries: HashMap<String, usize>,
        nodes: HashMap<usize, FakeNode>,
        next_node: usize,
        acquired: usize,
        dropped: usize,
        open_calls: Vec<(String, String, libc::c_int)>,
        fstatat_flags: Vec<libc::c_int>,
        operations: Vec<String>,
        read_calls: usize,
        read_requests: Vec<usize>,
        read_bytes: usize,
        open_errors: HashMap<String, libc::c_int>,
        mutate_after_read: Vec<Mutation>,
        mutate_on_observe: Vec<Mutation>,
    }

    impl FakeState {
        fn insert(&mut self, node: FakeNode) {
            let id = self.next_node;
            self.next_node += 1;
            self.entries.insert(node.path.clone(), id);
            self.nodes.insert(id, node);
        }

        fn replace(&mut self, path: &str, mut node: FakeNode) {
            node.path = path.to_owned();
            self.insert(node);
        }

        fn apply(&mut self, mutation: Mutation) {
            match mutation {
                Mutation::Metadata { path, metadata } => {
                    if let Some(id) = self.entries.get(&path).copied()
                        && let Some(node) = self.nodes.get_mut(&id)
                    {
                        node.metadata = metadata;
                    }
                }
                Mutation::Replace { path, node } => self.replace(&path, node),
            }
        }

        fn node_id(&self, path: &str) -> Option<usize> {
            self.entries.get(path).copied()
        }

        fn path_for(&self, id: usize) -> &str {
            &self.nodes.get(&id).expect("fake descriptor has node").path
        }
    }

    struct FakeHandle {
        node: usize,
        state: Rc<RefCell<FakeState>>,
    }

    impl Drop for FakeHandle {
        fn drop(&mut self) {
            self.state.borrow_mut().dropped += 1;
        }
    }

    struct FakeSyscalls {
        state: Rc<RefCell<FakeState>>,
    }

    impl FakeSyscalls {
        fn new() -> (Self, Rc<RefCell<FakeState>>) {
            let state = Rc::new(RefCell::new(FakeState::default()));
            (
                Self {
                    state: state.clone(),
                },
                state,
            )
        }

        fn handle(&self, node: usize) -> FakeHandle {
            self.state.borrow_mut().acquired += 1;
            FakeHandle {
                node,
                state: self.state.clone(),
            }
        }
    }

    impl LinuxSyscalls for FakeSyscalls {
        type Handle = FakeHandle;

        fn open_path(
            &mut self,
            path: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let name = path.to_str().expect("fixed fake path").to_owned();
            let mut state = self.state.borrow_mut();
            state.open_calls.push((String::new(), name.clone(), flags));
            state.operations.push(format!("open:{name}"));
            if let Some(error) = state.open_errors.get(&name).copied() {
                return Err(error);
            }
            let node = state.node_id(&name).ok_or(libc::ENOENT)?;
            drop(state);
            Ok(self.handle(node))
        }

        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let name = name.to_str().expect("fixed fake name").to_owned();
            let mut state = self.state.borrow_mut();
            let parent_path = state.path_for(parent.node).to_owned();
            let path = if parent_path == "/" {
                format!("/{name}")
            } else {
                format!("{parent_path}/{name}")
            };
            state.open_calls.push((parent_path, name, flags));
            state.operations.push(format!("open:{path}"));
            if let Some(error) = state.open_errors.get(&path).copied() {
                return Err(error);
            }
            let node = state.node_id(&path).ok_or(libc::ENOENT)?;
            drop(state);
            Ok(self.handle(node))
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<Metadata, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.node).to_owned();
            state.operations.push(format!("fstat:{path}"));
            state
                .nodes
                .get(&descriptor.node)
                .map(|node| node.metadata)
                .ok_or(libc::EBADF)
        }

        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Metadata, libc::c_int> {
            let name = name.to_str().expect("fixed fake name").to_owned();
            let mut state = self.state.borrow_mut();
            let parent_path = state.path_for(parent.node).to_owned();
            state.fstatat_flags.push(flags);
            let path = if name == "/" {
                "/".to_owned()
            } else if parent_path == "/" {
                format!("/{name}")
            } else {
                format!("{parent_path}/{name}")
            };
            state.operations.push(format!("fstatat:{path}"));
            let node = state.node_id(&path).ok_or(libc::ENOENT)?;
            state
                .nodes
                .get(&node)
                .map(|node| node.metadata)
                .ok_or(libc::ENOENT)
        }

        fn fgetxattr_size(
            &mut self,
            descriptor: &Self::Handle,
            name: &CStr,
        ) -> Result<Option<usize>, libc::c_int> {
            let name = name.to_str().expect("fixed ACL name").to_owned();
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.node).to_owned();
            state.operations.push(format!("acl:{path}:{name}"));
            let node = state.nodes.get(&descriptor.node).ok_or(libc::EBADF)?;
            node.acls.get(&name).cloned().unwrap_or(Ok(None))
        }

        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.node).to_owned();
            state.operations.push(format!("statfs:{path}"));
            state
                .nodes
                .get(&descriptor.node)
                .map(|node| node.filesystem_type)
                .ok_or(libc::EBADF)
        }

        fn read(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let mut state = self.state.borrow_mut();
            state.read_calls += 1;
            state.read_requests.push(buffer.len());
            let path = state.path_for(descriptor.node).to_owned();
            state.operations.push(format!("read:{path}"));
            let result = {
                let node = state.nodes.get_mut(&descriptor.node).ok_or(libc::EBADF)?;
                if let Some(error) = node.read_error.take() {
                    Err(error)
                } else {
                    let remaining = node.bytes.len().saturating_sub(node.read_offset);
                    let count = remaining.min(buffer.len()).min(node.max_read);
                    let end = node.read_offset + count;
                    buffer[..count].copy_from_slice(&node.bytes[node.read_offset..end]);
                    node.read_offset = end;
                    Ok(count)
                }
            };
            if let Ok(count) = &result {
                state.read_bytes += *count;
            }
            let changes = std::mem::take(&mut state.mutate_after_read);
            for change in changes {
                state.apply(change);
            }
            result
        }
    }

    fn metadata(
        kind: libc::mode_t,
        permissions: u32,
        device: u64,
        inode: u64,
        uid: u32,
        gid: u32,
        links: u64,
        size: i64,
    ) -> Metadata {
        Metadata {
            device,
            inode,
            mode: kind as u32 | permissions,
            uid,
            gid,
            links,
            size,
            mtime_seconds: 10,
            mtime_nanoseconds: 11,
            ctime_seconds: 12,
            ctime_nanoseconds: 13,
        }
    }

    fn directory(path: &str, mode: u32, device: u64, inode: u64, uid: u32, gid: u32) -> FakeNode {
        FakeNode::new(
            path,
            metadata(libc::S_IFDIR, mode, device, inode, uid, gid, 2, 4096),
            Vec::new(),
        )
    }

    fn file(
        path: &str,
        mode: u32,
        device: u64,
        inode: u64,
        uid: u32,
        gid: u32,
        bytes: Vec<u8>,
    ) -> FakeNode {
        let length = bytes.len() as i64;
        FakeNode::new(
            path,
            metadata(libc::S_IFREG, mode, device, inode, uid, gid, 1, length),
            bytes,
        )
    }

    fn config_environment(bytes: &[u8]) -> (FakeSyscalls, Rc<RefCell<FakeState>>) {
        let (syscalls, state) = FakeSyscalls::new();
        let mut fake = state.borrow_mut();
        fake.insert(directory("/", 0o755, 1, 1, 0, 0));
        fake.insert(directory("/run", 0o755, 1, 2, 0, 0));
        fake.insert(directory(
            "/run/owner-beta-simulation",
            0o750,
            1,
            3,
            0,
            10001,
        ));
        fake.insert(file(
            "/run/owner-beta-simulation/config.json",
            0o440,
            1,
            4,
            0,
            10001,
            bytes.to_vec(),
        ));
        drop(fake);
        (syscalls, state)
    }

    fn health_environment(bytes: &[u8]) -> (FakeSyscalls, Rc<RefCell<FakeState>>) {
        let (syscalls, state) = FakeSyscalls::new();
        let mut fake = state.borrow_mut();
        fake.insert(directory("/", 0o755, 1, 1, 0, 0));
        fake.insert(directory("/run", 0o755, 1, 2, 0, 0));
        fake.insert(directory(
            "/run/owner-beta-simulation-health",
            0o750,
            1,
            3,
            0,
            10001,
        ));
        fake.insert(file(
            "/run/owner-beta-simulation-health/writer.lock",
            0o440,
            1,
            4,
            0,
            10001,
            Vec::new(),
        ));
        let mut records = directory(
            "/run/owner-beta-simulation-health/records",
            0o700,
            2,
            5,
            10001,
            10001,
        );
        records.filesystem_type = libc::TMPFS_MAGIC as u64;
        fake.insert(records);
        fake.insert(file(
            "/run/owner-beta-simulation-health/records/current.json",
            0o600,
            2,
            6,
            10001,
            10001,
            bytes.to_vec(),
        ));
        drop(fake);
        (syscalls, state)
    }

    fn generation() -> SimulationProcessGeneration {
        SimulationProcessGeneration::new(
            4242,
            55,
            Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap(),
        )
        .unwrap()
    }

    #[derive(Clone, Copy)]
    enum HealthState {
        Starting,
        Ready,
        Busy,
        Draining,
        Failed,
        ReadyWithRecentSample,
    }

    fn health_bytes(
        config: &SimulationProcessConfig,
        generation: SimulationProcessGeneration,
        state: HealthState,
    ) -> Vec<u8> {
        let mut tracker = SimulationHealthTracker::new(config, generation, 10_000);
        if !matches!(state, HealthState::Starting) {
            tracker
                .apply(SimulationHealthEvent::StartupVerified, 10_001)
                .unwrap();
        }
        match state {
            HealthState::Starting | HealthState::Ready => {}
            HealthState::Busy => tracker
                .apply(SimulationHealthEvent::ClaimVerified, 10_002)
                .unwrap(),
            HealthState::Draining => tracker
                .apply(SimulationHealthEvent::ShutdownRequested, 10_002)
                .unwrap(),
            HealthState::Failed => {
                tracker
                    .apply(
                        SimulationHealthEvent::Fatal(SimulationHealthFailure::RuntimeFailure),
                        10_002,
                    )
                    .unwrap();
                tracker
                    .apply(SimulationHealthEvent::Sample, 10_003)
                    .unwrap();
            }
            HealthState::ReadyWithRecentSample => tracker
                .apply(SimulationHealthEvent::Sample, 300_000)
                .unwrap(),
        }
        tracker.record().to_json().unwrap()
    }

    struct FakeObservationFactory {
        state: Rc<RefCell<FakeState>>,
        generation: SimulationProcessGeneration,
        final_boot_ms: u64,
        observed_pid: Rc<Cell<u32>>,
        revalidate_count: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
        observe_error: Option<SimulationProcessObservationError>,
        revalidate_error: Option<SimulationProcessObservationError>,
        mutate_on_observe: Vec<Mutation>,
    }

    impl FakeObservationFactory {
        fn new(state: &Rc<RefCell<FakeState>>, generation: SimulationProcessGeneration) -> Self {
            Self {
                state: state.clone(),
                generation,
                final_boot_ms: 10_100,
                observed_pid: Rc::new(Cell::new(0)),
                revalidate_count: Rc::new(Cell::new(0)),
                drops: Rc::new(Cell::new(0)),
                observe_error: None,
                revalidate_error: None,
                mutate_on_observe: Vec::new(),
            }
        }
    }

    struct FakeObservation {
        state: Rc<RefCell<FakeState>>,
        generation: SimulationProcessGeneration,
        final_boot_ms: u64,
        revalidate_count: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
        revalidate_error: Option<SimulationProcessObservationError>,
    }

    impl Drop for FakeObservation {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    impl ProcessObservationHandle for FakeObservation {
        fn generation(&self) -> SimulationProcessGeneration {
            self.generation
        }

        fn revalidate(&mut self) -> Result<u64, SimulationProcessObservationError> {
            self.state
                .borrow_mut()
                .operations
                .push("observer:revalidate".to_owned());
            self.revalidate_count.set(self.revalidate_count.get() + 1);
            if let Some(error) = self.revalidate_error {
                return Err(error);
            }
            Ok(self.final_boot_ms)
        }
    }

    impl ProcessObservationFactory for FakeObservationFactory {
        type Observation = FakeObservation;

        fn observe(
            &mut self,
            pid: u32,
        ) -> Result<Self::Observation, SimulationProcessObservationError> {
            let mut state = self.state.borrow_mut();
            state.operations.push(format!("observer:observe:{pid}"));
            for mutation in std::mem::take(&mut self.mutate_on_observe) {
                state.apply(mutation);
            }
            drop(state);
            self.observed_pid.set(pid);
            if let Some(error) = self.observe_error {
                return Err(error);
            }
            Ok(FakeObservation {
                state: self.state.clone(),
                generation: self.generation,
                final_boot_ms: self.final_boot_ms,
                revalidate_count: self.revalidate_count.clone(),
                drops: self.drops.clone(),
                revalidate_error: self.revalidate_error,
            })
        }
    }

    fn mutate_node_metadata(
        state: &Rc<RefCell<FakeState>>,
        path: &str,
        f: impl FnOnce(&mut Metadata),
    ) {
        let mut fake = state.borrow_mut();
        let node_id = fake.node_id(path).unwrap();
        f(&mut fake.nodes.get_mut(&node_id).unwrap().metadata);
    }

    fn set_acl(
        state: &Rc<RefCell<FakeState>>,
        path: &str,
        name: &str,
        value: Result<Option<usize>, libc::c_int>,
    ) {
        let mut fake = state.borrow_mut();
        let node_id = fake.node_id(path).unwrap();
        fake.nodes
            .get_mut(&node_id)
            .unwrap()
            .acls
            .insert(name.to_owned(), value);
    }

    fn fake_node_metadata(state: &Rc<RefCell<FakeState>>, path: &str) -> Metadata {
        let fake = state.borrow();
        let node_id = fake.node_id(path).unwrap();
        fake.nodes.get(&node_id).unwrap().metadata
    }

    fn assert_all_handles_dropped(state: &Rc<RefCell<FakeState>>) {
        let fake = state.borrow();
        assert_eq!(fake.acquired, fake.dropped);
    }

    fn assert_no_reopened_paths(state: &Rc<RefCell<FakeState>>) {
        let fake = state.borrow();
        let mut opened = HashSet::new();
        for (parent, name, _) in &fake.open_calls {
            let path = if parent.is_empty() {
                name.clone()
            } else if parent == "/" {
                format!("/{name}")
            } else {
                format!("{parent}/{name}")
            };
            assert!(opened.insert(path.clone()), "path reopened: {path}");
        }
    }

    fn append_unreported_byte(state: &Rc<RefCell<FakeState>>, path: &str, expected_size: usize) {
        let mut fake = state.borrow_mut();
        let node_id = fake.node_id(path).unwrap();
        let node = fake.nodes.get_mut(&node_id).unwrap();
        assert_eq!(node.metadata.size, expected_size as i64);
        assert_eq!(node.bytes.len(), expected_size);
        node.bytes.push(b' ');
        node.max_read = expected_size + 1;
    }

    #[test]
    fn config_reader_uses_read_only_no_follow_handles_and_exact_c3a_bytes() {
        let (bytes, expected_hash, expected_config) = checked_config();
        let (syscalls, state) = config_environment(&bytes);
        let config = linux::read_config_with(syscalls, TEST_COMMIT, &expected_hash).unwrap();
        assert_eq!(config, expected_config);
        assert_all_handles_dropped(&state);

        let fake = state.borrow();
        assert_eq!(
            fake.read_calls > 1,
            true,
            "short positive reads were exercised"
        );
        assert!(
            fake.open_calls
                .iter()
                .all(|(_, _, flags)| flags & (libc::O_CREAT | libc::O_WRONLY | libc::O_RDWR) == 0)
        );
        assert_eq!(fake.open_calls[0].2, DIRECTORY_FLAGS);
        assert!(
            fake.open_calls
                .iter()
                .skip(1)
                .take(2)
                .all(|(_, _, flags)| *flags == DIRECTORY_FLAGS)
        );
        assert_eq!(fake.open_calls.last().unwrap().2, LEAF_FLAGS);
        assert!(!fake.open_calls.iter().any(|(_, name, _)| name == ".."));
        assert!(fake.fstatat_flags.iter().all(|flags| *flags == NOFOLLOW));
    }

    #[test]
    fn invalid_pins_are_rejected_before_io_and_c3a_rejects_bad_raw_config() {
        let (bytes, expected_hash, _) = checked_config();
        let (syscalls, state) = config_environment(&bytes);
        assert_eq!(
            linux::read_config_with(syscalls, &"0".repeat(40), &expected_hash),
            Err(Error::ConfigRejected)
        );
        assert!(state.borrow().operations.is_empty());
        assert_eq!(state.borrow().acquired, 0);

        let (syscalls, state) = config_environment(&bytes);
        let different_commit = "1123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            linux::read_config_with(syscalls, different_commit, &expected_hash),
            Err(Error::ConfigRejected)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let zero_hash = format!("sha256:{}", "0".repeat(64));
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &zero_hash),
            Err(Error::ConfigRejected)
        );
        assert_all_handles_dropped(&state);

        for invalid in [
            format!(
                "{}{}",
                VALID_CONFIG.trim_end_matches('}'),
                ",\"schema\":\"other\"}"
            ),
            VALID_CONFIG.replace(
                "\"schema\":\"owner-beta-simulation-process-v1\"",
                "\"schema\":7",
            ),
            VALID_CONFIG.replace(
                "\"schema\":\"owner-beta-simulation-process-v1\"",
                "\"schema\":\"owner-beta-simulation-process-v1\",\"unknown\":true",
            ),
        ] {
            let invalid_bytes = invalid.into_bytes();
            let invalid_hash = digest(&invalid_bytes);
            let (syscalls, state) = config_environment(&invalid_bytes);
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &invalid_hash),
                Err(Error::ConfigRejected)
            );
            assert_all_handles_dropped(&state);
        }
    }

    #[test]
    fn config_metadata_acl_type_and_hardlink_policies_fail_closed() {
        let (bytes, hash, _) = checked_config();

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation/config.json",
            |metadata| {
                metadata.uid = 10001;
            },
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation/config.json",
            |metadata| {
                metadata.links = 2;
            },
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation/config.json",
            |metadata| {
                metadata.mode |= 0o4000;
            },
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        for kind in [
            libc::S_IFIFO,
            libc::S_IFSOCK,
            libc::S_IFCHR,
            libc::S_IFDIR,
            libc::S_IFLNK,
        ] {
            let (syscalls, state) = config_environment(&bytes);
            mutate_node_metadata(
                &state,
                "/run/owner-beta-simulation/config.json",
                |metadata| {
                    metadata.mode = kind as u32 | 0o440;
                },
            );
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &hash),
                Err(Error::UnsafePath)
            );
            assert_all_handles_dropped(&state);
        }

        let (syscalls, state) = config_environment(&bytes);
        set_acl(
            &state,
            "/run/owner-beta-simulation/config.json",
            "system.posix_acl_access",
            Ok(Some(0)),
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        set_acl(
            &state,
            "/run/owner-beta-simulation/config.json",
            "system.posix_acl_default",
            Err(libc::EIO),
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::Unavailable)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        set_acl(
            &state,
            "/run/owner-beta-simulation/config.json",
            "system.posix_acl_access",
            Err(libc::ENOTSUP),
        );
        assert!(linux::read_config_with(syscalls, TEST_COMMIT, &hash).is_ok());
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn config_bounded_reads_cover_empty_size_limit_short_eof_and_eintr() {
        let (bytes, hash, _) = checked_config();
        for invalid_size in [0, -1] {
            let (syscalls, state) = config_environment(&bytes);
            mutate_node_metadata(
                &state,
                "/run/owner-beta-simulation/config.json",
                |metadata| {
                    metadata.size = invalid_size;
                },
            );
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &hash),
                Err(Error::InvalidMetadata)
            );
            assert_all_handles_dropped(&state);
        }

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation/config.json",
            |metadata| {
                metadata.size = (16 * 1024 + 1) as i64;
            },
        );
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::SizeLimit)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation/config.json")
            .unwrap();
        state.borrow_mut().nodes.get_mut(&node_id).unwrap().max_read = 19;
        assert!(linux::read_config_with(syscalls, TEST_COMMIT, &hash).is_ok());
        assert!(state.borrow().read_calls > 1);
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation/config.json")
            .unwrap();
        state
            .borrow_mut()
            .nodes
            .get_mut(&node_id)
            .unwrap()
            .bytes
            .pop();
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::Changed)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation/config.json")
            .unwrap();
        state
            .borrow_mut()
            .nodes
            .get_mut(&node_id)
            .unwrap()
            .read_error = Some(libc::EINTR);
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::Unavailable)
        );
        assert_eq!(state.borrow().read_calls, 1, "EINTR is not retried");
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn exact_config_and_health_caps_are_accepted_with_bounded_reads() {
        let (mut config_bytes, _, _) = checked_config();
        config_bytes.resize(16 * 1024, b' ');
        let config_hash = digest(&config_bytes);
        let (syscalls, state) = config_environment(&config_bytes);
        assert!(linux::read_config_with(syscalls, TEST_COMMIT, &config_hash).is_ok());
        assert_all_handles_dropped(&state);

        let (_, _, config) = checked_config();
        let process_generation = generation();
        let mut health = health_bytes(&config, process_generation, HealthState::Ready);
        health.resize(8 * 1024, b' ');
        let (syscalls, state) = health_environment(&health);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Ok(())
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn config_read_time_cap_plus_one_is_bounded_and_rejected() {
        let mut bytes = VALID_CONFIG.as_bytes().to_vec();
        bytes.resize(CONFIG_SIZE_LIMIT, b' ');
        let hash = digest(&bytes);
        let (syscalls, state) = config_environment(&bytes);
        append_unreported_byte(
            &state,
            "/run/owner-beta-simulation/config.json",
            CONFIG_SIZE_LIMIT,
        );

        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::SizeLimit)
        );
        {
            let fake = state.borrow();
            assert_eq!(fake.read_calls, 1);
            assert_eq!(fake.read_requests.as_slice(), &[CONFIG_SIZE_LIMIT + 1]);
            assert_eq!(fake.read_bytes, CONFIG_SIZE_LIMIT + 1);
            assert_eq!(fake.open_calls.len(), 4);
        }
        assert_no_reopened_paths(&state);
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn health_read_time_cap_plus_one_is_bounded_and_precedes_observation() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let mut bytes = health_bytes(&config, process_generation, HealthState::Ready);
        bytes.resize(HEALTH_SIZE_LIMIT, b' ');
        let (syscalls, state) = health_environment(&bytes);
        append_unreported_byte(
            &state,
            "/run/owner-beta-simulation-health/records/current.json",
            HEALTH_SIZE_LIMIT,
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);

        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::SizeLimit)
        );
        assert_eq!(observer.observed_pid.get(), 0);
        {
            let fake = state.borrow();
            assert_eq!(fake.read_calls, 1);
            assert_eq!(fake.read_requests.as_slice(), &[HEALTH_SIZE_LIMIT + 1]);
            assert_eq!(fake.read_bytes, HEALTH_SIZE_LIMIT + 1);
            assert_eq!(fake.open_calls.len(), 6);
        }
        assert_no_reopened_paths(&state);
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn config_leaf_mutation_fails_but_directory_timestamp_change_is_allowed() {
        let (bytes, hash, _) = checked_config();
        let (syscalls, state) = config_environment(&bytes);
        let mut changed = fake_node_metadata(&state, "/run/owner-beta-simulation/config.json");
        changed.ctime_nanoseconds += 1;
        state
            .borrow_mut()
            .mutate_after_read
            .push(Mutation::Metadata {
                path: "/run/owner-beta-simulation/config.json".to_owned(),
                metadata: changed,
            });
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::Changed)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let mut changed = fake_node_metadata(&state, "/run");
        changed.mtime_seconds += 1;
        state
            .borrow_mut()
            .mutate_after_read
            .push(Mutation::Metadata {
                path: "/run".to_owned(),
                metadata: changed,
            });
        assert!(linux::read_config_with(syscalls, TEST_COMMIT, &hash).is_ok());
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        let mut changed = fake_node_metadata(&state, "/run");
        changed.inode += 100;
        state
            .borrow_mut()
            .mutate_after_read
            .push(Mutation::Metadata {
                path: "/run".to_owned(),
                metadata: changed,
            });
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::Changed)
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn current_name_or_metadata_change_after_read_is_rejected_before_observation() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);

        for path in [
            "/",
            "/run/owner-beta-simulation-health",
            "/run/owner-beta-simulation-health/writer.lock",
            "/run/owner-beta-simulation-health/records",
            "/run/owner-beta-simulation-health/records/current.json",
        ] {
            let (syscalls, state) = health_environment(&bytes);
            let mut changed = fake_node_metadata(&state, path);
            if path.ends_with(".json") || path.ends_with("writer.lock") {
                changed.ctime_nanoseconds += 1;
            } else {
                changed.inode += 1000;
            }
            state
                .borrow_mut()
                .mutate_after_read
                .push(Mutation::Metadata {
                    path: path.to_owned(),
                    metadata: changed,
                });
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::Changed)
            );
            assert_eq!(observer.observed_pid.get(), 0);
            assert_all_handles_dropped(&state);
        }

        let (syscalls, state) = health_environment(&bytes);
        let original = {
            let fake = state.borrow();
            let id = fake
                .node_id("/run/owner-beta-simulation-health/records/current.json")
                .unwrap();
            fake.nodes.get(&id).unwrap().clone()
        };
        let mut replacement = original;
        replacement.metadata.inode += 1000;
        state
            .borrow_mut()
            .mutate_after_read
            .push(Mutation::Replace {
                path: "/run/owner-beta-simulation-health/records/current.json".to_owned(),
                node: replacement,
            });
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::Changed)
        );
        assert_eq!(observer.observed_pid.get(), 0);
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn config_root_and_parent_metadata_or_symlink_fail_closed() {
        let (bytes, hash, _) = checked_config();
        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(&state, "/", |metadata| {
            metadata.mode = libc::S_IFLNK as u32 | 0o777
        });
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::UnsafePath)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(&state, "/run", |metadata| metadata.mode |= 0o020);
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = config_environment(&bytes);
        mutate_node_metadata(&state, "/run/owner-beta-simulation", |metadata| {
            metadata.mode = libc::S_IFLNK as u32 | 0o777;
        });
        assert_eq!(
            linux::read_config_with(syscalls, TEST_COMMIT, &hash),
            Err(Error::UnsafePath)
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn health_ready_and_busy_records_follow_c3b_and_observer_order() {
        let (_, _, config) = checked_config();
        for health_state in [HealthState::Ready, HealthState::Busy] {
            let process_generation = generation();
            let bytes = health_bytes(&config, process_generation, health_state);
            let (syscalls, state) = health_environment(&bytes);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            let result = linux::check_health_with(syscalls, &mut observer, &config);
            assert_eq!(result, Ok(()));
            assert_eq!(observer.observed_pid.get(), process_generation.pid());
            assert_eq!(observer.revalidate_count.get(), 1);
            assert_eq!(observer.drops.get(), 1);
            assert_all_handles_dropped(&state);

            let fake = state.borrow();
            let operations = &fake.operations;
            let observe = operations
                .iter()
                .position(|value| value == "observer:observe:4242")
                .unwrap();
            let revalidate = operations
                .iter()
                .position(|value| value == "observer:revalidate")
                .unwrap();
            let post_observer_file_check = operations
                .iter()
                .rposition(|value| {
                    value == "fstatat:/run/owner-beta-simulation-health/records/current.json"
                })
                .unwrap();
            let last_read = operations
                .iter()
                .position(|value| {
                    value == "read:/run/owner-beta-simulation-health/records/current.json"
                })
                .unwrap();
            assert!(last_read < observe && observe < post_observer_file_check);
            assert!(post_observer_file_check < revalidate);
            let fake = state.borrow();
            assert!(
                fake.open_calls.iter().all(|(_, _, flags)| flags
                    & (libc::O_CREAT | libc::O_WRONLY | libc::O_RDWR)
                    == 0)
            );
            assert!(fake.fstatat_flags.iter().all(|flags| *flags == NOFOLLOW));
            assert!(
                fake.open_calls
                    .iter()
                    .filter(|(_, name, _)| name == "writer.lock" || name == "current.json")
                    .all(|(_, _, flags)| *flags == LEAF_FLAGS)
            );
        }
    }

    #[test]
    fn malformed_duplicate_and_unbound_health_never_reach_process_observer() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let valid = health_bytes(&config, process_generation, HealthState::Ready);
        let duplicate = format!(
            "{}{}",
            String::from_utf8(valid.clone())
                .unwrap()
                .trim_end_matches('}'),
            ",\"phase\":\"Failed\"}"
        )
        .into_bytes();
        let unbound = String::from_utf8(valid)
            .unwrap()
            .replace(
                config.config_sha256(),
                "sha256:9999999999999999999999999999999999999999999999999999999999999999",
            )
            .into_bytes();

        for bytes in [duplicate, unbound] {
            let (syscalls, state) = health_environment(&bytes);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::HealthRejected)
            );
            assert_eq!(observer.observed_pid.get(), 0);
            assert_eq!(observer.drops.get(), 0);
            assert_all_handles_dropped(&state);
        }

        let raw_diagnostic = format!("{:?} {}", Error::HealthRejected, Error::HealthRejected);
        assert!(!raw_diagnostic.contains("/run"));
        assert!(!raw_diagnostic.contains("4242"));
    }

    #[test]
    fn generation_binding_and_final_observer_clock_control_readiness() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);

        let (syscalls, state) = health_environment(&bytes);
        let mut observer = FakeObservationFactory::new(&state, generation_with_pid(4243));
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::GenerationInvalid)
        );
        assert_eq!(observer.revalidate_count.get(), 0);
        assert_eq!(observer.drops.get(), 1);
        let fake = state.borrow();
        let observe = fake
            .operations
            .iter()
            .position(|value| value.as_str() == "observer:observe:4242")
            .unwrap();
        let post_observer_file_check = fake
            .operations
            .iter()
            .rposition(|value| {
                value.as_str() == "fstatat:/run/owner-beta-simulation-health/records/current.json"
            })
            .unwrap();
        assert!(observe < post_observer_file_check);
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.revalidate_error = Some(SimulationProcessObservationError::ClockInvalid);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::ClockInvalid)
        );
        assert_eq!(observer.revalidate_count.get(), 1);
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.final_boot_ms = 400_000;
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::HealthRejected)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.final_boot_ms = 10_001 + 234_000;
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Ok(()),
            "the C3B freshness boundary is inclusive"
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn observer_unavailability_is_typed_and_closes_the_read_set() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);
        let (syscalls, state) = health_environment(&bytes);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.observe_error = Some(SimulationProcessObservationError::Unavailable);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::Unavailable)
        );
        assert_eq!(observer.revalidate_count.get(), 0);
        assert_eq!(observer.drops.get(), 0);
        assert_all_handles_dropped(&state);
    }

    fn generation_with_pid(pid: u32) -> SimulationProcessGeneration {
        SimulationProcessGeneration::new(
            pid,
            55,
            Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn c3b_phase_staleness_future_clock_and_sticky_failure_remain_authoritative() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        for state_value in [
            HealthState::Starting,
            HealthState::Draining,
            HealthState::Failed,
        ] {
            let bytes = health_bytes(&config, process_generation, state_value);
            let (syscalls, state) = health_environment(&bytes);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::HealthRejected)
            );
            assert_eq!(observer.revalidate_count.get(), 1);
            assert_all_handles_dropped(&state);
        }

        let stale_progress = health_bytes(
            &config,
            process_generation,
            HealthState::ReadyWithRecentSample,
        );
        let (syscalls, state) = health_environment(&stale_progress);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.final_boot_ms = 400_000;
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::HealthRejected),
            "a recent sample does not refresh old progress"
        );
        assert_all_handles_dropped(&state);

        let ready = health_bytes(&config, process_generation, HealthState::Ready);
        let (syscalls, state) = health_environment(&ready);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        observer.final_boot_ms = 10_000;
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::ClockInvalid)
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn health_storage_metadata_acl_tmpfs_and_device_relationships_fail_closed() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);

        let cases: [(&str, fn(&mut Metadata)); 3] = [
            (
                "/run/owner-beta-simulation-health/records",
                |metadata: &mut Metadata| metadata.uid = 0,
            ),
            (
                "/run/owner-beta-simulation-health/records/current.json",
                |metadata: &mut Metadata| metadata.uid = 0,
            ),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                |metadata: &mut Metadata| metadata.gid = 0,
            ),
        ];
        for (path, change) in cases {
            let (syscalls, state) = health_environment(&bytes);
            mutate_node_metadata(&state, path, change);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::InvalidMetadata)
            );
            assert_eq!(observer.observed_pid.get(), 0);
            assert_all_handles_dropped(&state);
        }

        let (syscalls, state) = health_environment(&bytes);
        let records_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation-health/records")
            .unwrap();
        state
            .borrow_mut()
            .nodes
            .get_mut(&records_id)
            .unwrap()
            .filesystem_type = 0;
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation-health/records",
            |metadata| metadata.device = 1,
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation-health/records/current.json",
            |metadata| metadata.device = 3,
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation-health/writer.lock",
            |metadata| metadata.device = 4,
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        set_acl(
            &state,
            "/run/owner-beta-simulation-health/records/current.json",
            "system.posix_acl_access",
            Ok(Some(0)),
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::InvalidMetadata)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        set_acl(
            &state,
            "/run/owner-beta-simulation-health/records/current.json",
            "system.posix_acl_default",
            Err(libc::EIO),
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::Unavailable)
        );
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn health_read_size_eof_and_interruption_are_bounded_and_close_descriptors() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);

        for invalid_size in [0, -1] {
            let (syscalls, state) = health_environment(&bytes);
            mutate_node_metadata(
                &state,
                "/run/owner-beta-simulation-health/records/current.json",
                |metadata| metadata.size = invalid_size,
            );
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::InvalidMetadata)
            );
            assert_eq!(observer.observed_pid.get(), 0);
            assert_all_handles_dropped(&state);
        }

        let (syscalls, state) = health_environment(&bytes);
        mutate_node_metadata(
            &state,
            "/run/owner-beta-simulation-health/records/current.json",
            |metadata| metadata.size = 8193,
        );
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::SizeLimit)
        );
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation-health/records/current.json")
            .unwrap();
        state.borrow_mut().nodes.get_mut(&node_id).unwrap().max_read = 17;
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Ok(())
        );
        assert!(state.borrow().read_calls > 1);
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation-health/records/current.json")
            .unwrap();
        state
            .borrow_mut()
            .nodes
            .get_mut(&node_id)
            .unwrap()
            .bytes
            .pop();
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::Changed)
        );
        assert_eq!(observer.observed_pid.get(), 0);
        assert_all_handles_dropped(&state);

        let (syscalls, state) = health_environment(&bytes);
        let node_id = state
            .borrow()
            .node_id("/run/owner-beta-simulation-health/records/current.json")
            .unwrap();
        state
            .borrow_mut()
            .nodes
            .get_mut(&node_id)
            .unwrap()
            .read_error = Some(libc::EINTR);
        let mut observer = FakeObservationFactory::new(&state, process_generation);
        assert_eq!(
            linux::check_health_with(syscalls, &mut observer, &config),
            Err(Error::Unavailable)
        );
        assert_eq!(state.borrow().read_calls, 1);
        assert_eq!(observer.observed_pid.get(), 0);
        assert_all_handles_dropped(&state);
    }

    #[test]
    fn retained_file_and_lock_replacements_during_observation_are_rejected() {
        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);

        for path in [
            "/",
            "/run/owner-beta-simulation-health",
            "/run/owner-beta-simulation-health/writer.lock",
            "/run/owner-beta-simulation-health/records",
            "/run/owner-beta-simulation-health/records/current.json",
        ] {
            let (syscalls, state) = health_environment(&bytes);
            let mut replacement = {
                let fake = state.borrow();
                let id = fake.node_id(path).unwrap();
                fake.nodes.get(&id).unwrap().clone()
            };
            replacement.metadata.inode += 1000;
            observer_replace_on_call(&state, path, replacement);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            observer.mutate_on_observe = std::mem::take(&mut state.borrow_mut().mutate_on_observe);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::Changed)
            );
            assert_eq!(observer.revalidate_count.get(), 0);
            assert_eq!(observer.drops.get(), 1);
            assert_all_handles_dropped(&state);
        }
    }

    fn observer_replace_on_call(state: &Rc<RefCell<FakeState>>, path: &str, node: FakeNode) {
        state
            .borrow_mut()
            .mutate_on_observe
            .push(Mutation::Replace {
                path: path.to_owned(),
                node,
            });
    }

    #[test]
    fn partial_acquisition_closes_every_descriptor_and_static_errors_do_not_echo_paths() {
        let (config_bytes, hash, _) = checked_config();
        for path in [
            "/run",
            "/run/owner-beta-simulation",
            "/run/owner-beta-simulation/config.json",
        ] {
            let (syscalls, state) = config_environment(&config_bytes);
            state
                .borrow_mut()
                .open_errors
                .insert(path.to_owned(), libc::ENOENT);
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &hash),
                Err(Error::Unavailable)
            );
            assert_all_handles_dropped(&state);
        }

        let (_, _, config) = checked_config();
        let process_generation = generation();
        let bytes = health_bytes(&config, process_generation, HealthState::Ready);
        for path in [
            "/run/owner-beta-simulation-health",
            "/run/owner-beta-simulation-health/writer.lock",
            "/run/owner-beta-simulation-health/records",
            "/run/owner-beta-simulation-health/records/current.json",
        ] {
            let (syscalls, state) = health_environment(&bytes);
            state
                .borrow_mut()
                .open_errors
                .insert(path.to_owned(), libc::ENOENT);
            let mut observer = FakeObservationFactory::new(&state, process_generation);
            assert_eq!(
                linux::check_health_with(syscalls, &mut observer, &config),
                Err(Error::Unavailable)
            );
            assert_eq!(observer.observed_pid.get(), 0);
            assert_all_handles_dropped(&state);
        }
        assert!(!format!("{:?}", Error::UnsafePath).contains("config.json"));
    }

    #[test]
    fn root_and_child_open_errors_share_typed_classification_without_retry() {
        let (config_bytes, hash, _) = checked_config();
        let errors = [
            (libc::ELOOP, Error::UnsafePath),
            (libc::ENOTDIR, Error::UnsafePath),
            (libc::EISDIR, Error::UnsafePath),
            (libc::ENOENT, Error::Unavailable),
            (libc::EACCES, Error::Unavailable),
            (libc::EIO, Error::Unavailable),
            (libc::EINTR, Error::Unavailable),
        ];

        for (errno, expected) in errors {
            let (syscalls, state) = config_environment(&config_bytes);
            state.borrow_mut().open_errors.insert("/".to_owned(), errno);
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &hash),
                Err(expected)
            );
            {
                let fake = state.borrow();
                assert_eq!(fake.open_calls.len(), 1);
                assert_eq!(fake.read_calls, 0);
                assert_eq!(fake.acquired, 0);
            }
            assert_all_handles_dropped(&state);

            let (syscalls, state) = config_environment(&config_bytes);
            state
                .borrow_mut()
                .open_errors
                .insert("/run".to_owned(), errno);
            assert_eq!(
                linux::read_config_with(syscalls, TEST_COMMIT, &hash),
                Err(expected)
            );
            {
                let fake = state.borrow();
                assert_eq!(fake.open_calls.len(), 2);
                assert_eq!(fake.read_calls, 0);
                assert_eq!(fake.acquired, 1);
                assert_eq!(fake.dropped, 1);
            }
            assert_all_handles_dropped(&state);
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_public_factories_fail_before_io_or_config_inspection() {
    let (bytes, hash, config) = checked_config();
    assert!(!bytes.is_empty());
    assert!(!hash.is_empty());
    assert_eq!(
        super::read_simulation_process_config("invalid", "invalid"),
        Err(Error::UnsupportedPlatform)
    );
    assert_eq!(
        super::check_simulation_process_health(&config),
        Err(Error::UnsupportedPlatform)
    );
}
