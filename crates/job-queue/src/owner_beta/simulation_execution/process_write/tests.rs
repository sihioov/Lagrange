// Test-only syscall casts and exact log scans preserve cross-platform fixtures.
#![allow(clippy::unnecessary_cast, clippy::double_ended_iterator_last)]

use super::{SimulationHealthWriteError as Error, SimulationProcessConfig};
use sha2::{Digest, Sha256};

const TEST_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const VALID_CONFIG: &str = r###"{"schema":"owner-beta-simulation-process-v1","code_commit":"0123456789abcdef0123456789abcdef01234567","manifest_format":"LAGRANGE_RELEASE_MANIFEST_V3","manifest_sha256":"sha256:1111111111111111111111111111111111111111111111111111111111111111","image_id":"sha256:2222222222222222222222222222222222222222222222222222222222222222","receipt_sha256":"sha256:3333333333333333333333333333333333333333333333333333333333333333","binary_sha256":"sha256:4444444444444444444444444444444444444444444444444444444444444444","source_input_sha256":"sha256:5555555555555555555555555555555555555555555555555555555555555555","recipe_sha256":"sha256:6666666666666666666666666666666666666666666666666666666666666666","build_origin":"official-release-v1","package":"job-queue","bin":"owner-beta-simulation-runner","target":"x86_64-unknown-linux-musl","profile":"release","toolchain":"1.97.1","owner":"11111111-2222-3333-4444-555555555555","entitlement_reference":"fixture:synthetic-owner-rights-v1","entitlement_sha256":"sha256:7777777777777777777777777777777777777777777777777777777777777777","historical_root":"/synthetic/history/δ","reader_uid":10001,"reader_gid":10001,"paths":{"config":"/run/owner-beta-simulation/config.json","artifact_root":"/data/owner-beta-artifacts","receipt":"/usr/share/lagrange/owner-beta-simulation/artifact.json","complete":"/usr/share/lagrange/owner-beta-simulation/COMPLETE","executable":"/usr/local/bin/owner-beta-simulation-runner","worker_id":"owner-beta-simulation-runner"},"limits":{"memory_bytes":1234567,"nano_cpus":2345678,"health_tmpfs_bytes":3456789,"pids_limit":42,"startup_timeout_seconds":123,"health_max_age_seconds":234,"stop_timeout_seconds":345}}"###;

fn checked_config() -> SimulationProcessConfig {
    let bytes = VALID_CONFIG.as_bytes();
    let digest = format!("sha256:{:x}", Sha256::digest(bytes));
    SimulationProcessConfig::from_json(bytes, TEST_COMMIT, &digest)
        .expect("the fixed C3A fixture is checked")
}

#[cfg(target_os = "linux")]
mod linux_tests {
    use std::{
        cell::RefCell,
        collections::{HashMap, HashSet, VecDeque},
        ffi::CStr,
        rc::Rc,
    };

    use super::super::linux::{
        self, LinuxSyscalls, Metadata, ObservationFactory, ObservationHandle, WriterPolicy,
    };
    use super::super::{SimulationHealthWriter, WriterSession};
    use super::*;
    use crate::owner_beta::simulation_execution::{
        SimulationHealthEvent, SimulationHealthFailure, SimulationHealthPhase,
        SimulationHealthRecord, SimulationProcessGeneration, SimulationProcessObservationError,
    };
    use uuid::Uuid;

    const PROCESS_PATH: &str = "/run/owner-beta-simulation-health";
    const RECORDS_PATH: &str = "/run/owner-beta-simulation-health/records";
    const CURRENT_PATH: &str = "/run/owner-beta-simulation-health/records/current.json";
    const PENDING_PATH: &str = "/run/owner-beta-simulation-health/records/.pending.json";

    #[derive(Clone)]
    enum FakeMutation {
        ReplacePath(String),
        SizeDelta { path: String, delta: i64 },
        BumpMtime(String),
        InsertRegular { path: String, bytes: Vec<u8> },
    }

    #[derive(Clone)]
    struct FakeNode {
        path: String,
        metadata: Metadata,
        bytes: Vec<u8>,
        filesystem_type: u64,
        directory_cursor: usize,
        acls: HashMap<String, Option<usize>>,
    }

    impl FakeNode {
        fn new(path: &str, metadata: Metadata, filesystem_type: u64) -> Self {
            Self {
                path: path.to_owned(),
                metadata,
                bytes: Vec::new(),
                filesystem_type,
                directory_cursor: 0,
                acls: HashMap::new(),
            }
        }
    }

    struct FakeState {
        entries: HashMap<String, usize>,
        nodes: HashMap<usize, FakeNode>,
        next_node: usize,
        acquired: usize,
        dropped: usize,
        operations: Vec<String>,
        open_calls: Vec<(String, libc::c_int, libc::mode_t)>,
        fstatat_flags: Vec<libc::c_int>,
        effective_uid: u32,
        effective_gid: u32,
        max_write: usize,
        max_read: usize,
        append_after_first_write: bool,
        create_mode_override: Option<u32>,
        create_metadata_override: Option<Metadata>,
        temp_acl_present: bool,
        failures: HashMap<String, Vec<(usize, libc::c_int)>>,
        call_counts: HashMap<String, usize>,
        mutations: Vec<(String, usize, FakeMutation)>,
        pread_mutations: HashMap<String, FakeMutation>,
        pread_size_override: HashMap<String, usize>,
        pread_requests: Vec<(String, i64, usize, usize)>,
        directory_override: Option<Vec<u8>>,
        override_cursor: usize,
    }

    impl FakeState {
        fn new() -> Self {
            let mut state = Self {
                entries: HashMap::new(),
                nodes: HashMap::new(),
                next_node: 1,
                acquired: 0,
                dropped: 0,
                operations: Vec::new(),
                open_calls: Vec::new(),
                fstatat_flags: Vec::new(),
                effective_uid: 10001,
                effective_gid: 10001,
                max_write: usize::MAX,
                max_read: usize::MAX,
                append_after_first_write: false,
                create_mode_override: None,
                create_metadata_override: None,
                temp_acl_present: false,
                failures: HashMap::new(),
                call_counts: HashMap::new(),
                mutations: Vec::new(),
                pread_mutations: HashMap::new(),
                pread_size_override: HashMap::new(),
                pread_requests: Vec::new(),
                directory_override: None,
                override_cursor: 0,
            };
            state.insert(FakeNode::new(
                "/",
                metadata(1, 1, libc::S_IFDIR as u32, 0, 0, 0o755, 0),
                0,
            ));
            state.insert(FakeNode::new(
                "/run",
                metadata(1, 2, libc::S_IFDIR as u32, 0, 0, 0o755, 0),
                0,
            ));
            state.insert(FakeNode::new(
                PROCESS_PATH,
                metadata(1, 3, libc::S_IFDIR as u32, 0, 10001, 0o750, 0),
                0,
            ));
            state.insert(FakeNode::new(
                "/run/owner-beta-simulation-health/writer.lock",
                metadata(1, 4, libc::S_IFREG as u32, 0, 10001, 0o440, 0),
                0,
            ));
            state.insert(FakeNode::new(
                RECORDS_PATH,
                metadata(2, 5, libc::S_IFDIR as u32, 10001, 10001, 0o700, 0),
                libc::TMPFS_MAGIC as u64,
            ));
            state
        }

        fn insert(&mut self, node: FakeNode) -> usize {
            let id = self.next_node;
            self.next_node += 1;
            self.entries.insert(node.path.clone(), id);
            self.nodes.insert(id, node);
            id
        }

        fn fail_on(&mut self, operation: &str, call: usize, errno: libc::c_int) {
            self.failures
                .entry(operation.to_owned())
                .or_default()
                .push((call, errno));
        }

        fn failure(&mut self, operation: &str) -> Option<libc::c_int> {
            let current = {
                let count = self.call_counts.entry(operation.to_owned()).or_default();
                *count += 1;
                *count
            };
            self.failures
                .get(operation)
                .and_then(|items| items.iter().find(|(call, _)| *call == current))
                .map(|(_, errno)| *errno)
        }

        fn mutate_on(&mut self, operation: &str, call: usize, mutation: FakeMutation) {
            self.mutations.push((operation.to_owned(), call, mutation));
        }

        fn apply_mutations(&mut self, operation: &str) {
            let call = self.call_counts.get(operation).copied().unwrap_or_default();
            let mut selected = Vec::new();
            self.mutations.retain(|(candidate, at, mutation)| {
                if candidate == operation && *at == call {
                    selected.push(mutation.clone());
                    false
                } else {
                    true
                }
            });
            for mutation in selected {
                self.apply_mutation(mutation);
            }
        }

        fn apply_mutation(&mut self, mutation: FakeMutation) {
            match mutation {
                FakeMutation::ReplacePath(path) => {
                    let id = self.id_for(&path).expect("hook path must exist");
                    let mut replacement = self.nodes.get(&id).unwrap().clone();
                    replacement.metadata.inode = replacement.metadata.inode.saturating_add(1_000);
                    self.insert(replacement);
                }
                FakeMutation::SizeDelta { path, delta } => {
                    let id = self.id_for(&path).expect("hook path must exist");
                    let node = self.nodes.get_mut(&id).unwrap();
                    node.metadata.size = node
                        .metadata
                        .size
                        .checked_add(delta)
                        .expect("test size mutation stays in range");
                }
                FakeMutation::BumpMtime(path) => {
                    let id = self.id_for(&path).expect("hook path must exist");
                    let node = self.nodes.get_mut(&id).unwrap();
                    node.metadata.mtime_seconds = node.metadata.mtime_seconds.saturating_add(1);
                }
                FakeMutation::InsertRegular { path, bytes } => self.add_regular(&path, &bytes),
            }
        }

        fn add_regular(&mut self, path: &str, bytes: &[u8]) {
            let mut node = FakeNode::new(
                path,
                metadata(
                    2,
                    90 + self.next_node as u64,
                    libc::S_IFREG as u32,
                    10001,
                    10001,
                    0o600,
                    bytes.len() as i64,
                ),
                0,
            );
            node.bytes = bytes.to_vec();
            self.insert(node);
        }

        fn id_for(&self, path: &str) -> Option<usize> {
            self.entries.get(path).copied()
        }

        fn path_for(&self, id: usize) -> &str {
            &self.nodes.get(&id).expect("fake handle owns a node").path
        }
    }

    fn metadata(
        device: u64,
        inode: u64,
        kind: u32,
        uid: u32,
        gid: u32,
        permissions: u32,
        size: i64,
    ) -> Metadata {
        Metadata {
            device,
            inode,
            mode: kind | permissions,
            uid,
            gid,
            links: 1,
            size,
            mtime_seconds: 10,
            mtime_nanoseconds: 1,
            ctime_seconds: 10,
            ctime_nanoseconds: 1,
        }
    }

    fn path_at(parent: &str, name: &CStr) -> String {
        let name = name.to_str().expect("the policy uses fixed ASCII names");
        if name.starts_with('/') {
            name.to_owned()
        } else if parent == "/" {
            format!("/{name}")
        } else {
            format!("{parent}/{name}")
        }
    }

    struct FakeHandle {
        id: usize,
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
            let state = Rc::new(RefCell::new(FakeState::new()));
            (
                Self {
                    state: state.clone(),
                },
                state,
            )
        }

        fn handle(&self, id: usize) -> FakeHandle {
            self.state.borrow_mut().acquired += 1;
            FakeHandle {
                id,
                state: self.state.clone(),
            }
        }

        fn note(state: &mut FakeState, operation: impl Into<String>) {
            state.operations.push(operation.into());
        }
    }

    impl LinuxSyscalls for FakeSyscalls {
        type Handle = FakeHandle;

        fn effective_ids(&mut self) -> (u32, u32) {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "ids");
            (state.effective_uid, state.effective_gid)
        }

        fn open_path(
            &mut self,
            path: &CStr,
            flags: libc::c_int,
        ) -> Result<Self::Handle, libc::c_int> {
            let path = path.to_str().expect("fixed fake path").to_owned();
            let id = {
                let mut state = self.state.borrow_mut();
                Self::note(&mut state, format!("open:{path}"));
                state.open_calls.push((path.clone(), flags, 0));
                if let Some(error) = state.failure("open_path") {
                    return Err(error);
                }
                state.id_for(&path).ok_or(libc::ENOENT)?
            };
            Ok(self.handle(id))
        }

        fn open_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
            mode: libc::mode_t,
        ) -> Result<Self::Handle, libc::c_int> {
            let id = {
                let mut state = self.state.borrow_mut();
                let parent_path = state.path_for(parent.id).to_owned();
                let path = path_at(&parent_path, name);
                let creating = flags & libc::O_CREAT != 0;
                Self::note(
                    &mut state,
                    if creating {
                        format!("create:{path}")
                    } else {
                        format!("openat:{path}")
                    },
                );
                state.open_calls.push((path.clone(), flags, mode));
                let key = if creating { "create" } else { "open_at" };
                if let Some(error) = state.failure(key) {
                    return Err(error);
                }
                state.apply_mutations(key);
                if creating {
                    if state.entries.contains_key(&path) {
                        return Err(libc::EEXIST);
                    }
                    let inode = 100 + state.next_node as u64;
                    let uid = state.effective_uid;
                    let gid = state.effective_gid;
                    let mut file_metadata = metadata(
                        2,
                        inode,
                        libc::S_IFREG as u32,
                        uid,
                        gid,
                        state.create_mode_override.unwrap_or(mode as u32),
                        0,
                    );
                    if let Some(override_metadata) = state.create_metadata_override {
                        file_metadata = override_metadata;
                    }
                    let mut node = FakeNode::new(&path, file_metadata, 0);
                    if state.temp_acl_present {
                        node.acls
                            .insert("system.posix_acl_access".to_owned(), Some(0));
                    }
                    state.insert(node)
                } else {
                    state.id_for(&path).ok_or(libc::ENOENT)?
                }
            };
            Ok(self.handle(id))
        }

        fn fstat(&mut self, descriptor: &Self::Handle) -> Result<Metadata, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.id).to_owned();
            Self::note(&mut state, format!("fstat:{path}"));
            state
                .nodes
                .get(&descriptor.id)
                .map(|node| node.metadata)
                .ok_or(libc::EBADF)
        }

        fn fstat_at(
            &mut self,
            parent: &Self::Handle,
            name: &CStr,
            flags: libc::c_int,
        ) -> Result<Metadata, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let parent_path = state.path_for(parent.id).to_owned();
            let path = path_at(&parent_path, name);
            Self::note(&mut state, format!("fstatat:{path}"));
            state.fstatat_flags.push(flags);
            if let Some(error) = state.failure("fstat_at") {
                return Err(error);
            }
            let id = state.id_for(&path).ok_or(libc::ENOENT)?;
            state
                .nodes
                .get(&id)
                .map(|node| node.metadata)
                .ok_or(libc::ENOENT)
        }

        fn fgetxattr_size(
            &mut self,
            descriptor: &Self::Handle,
            name: &CStr,
        ) -> Result<Option<usize>, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let name = name.to_str().expect("fixed ACL name").to_owned();
            let path = state.path_for(descriptor.id).to_owned();
            Self::note(&mut state, format!("acl:{path}:{name}"));
            if let Some(error) = state.failure("fgetxattr") {
                return Err(error);
            }
            let node = state.nodes.get(&descriptor.id).ok_or(libc::EBADF)?;
            Ok(node.acls.get(&name).copied().unwrap_or(None))
        }

        fn fstatfs_type(&mut self, descriptor: &Self::Handle) -> Result<u64, libc::c_int> {
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.id).to_owned();
            Self::note(&mut state, format!("statfs:{path}"));
            state
                .nodes
                .get(&descriptor.id)
                .map(|node| node.filesystem_type)
                .ok_or(libc::EBADF)
        }

        fn flock_exclusive_nonblocking(
            &mut self,
            descriptor: &Self::Handle,
        ) -> Result<(), libc::c_int> {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "flock:LOCK_EX|LOCK_NB");
            let path = state.path_for(descriptor.id).to_owned();
            if path != "/run/owner-beta-simulation-health/writer.lock" {
                return Err(libc::EBADF);
            }
            state.failure("flock").map_or(Ok(()), Err)
        }

        fn seek_directory_start(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int> {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "lseek:records:0");
            if let Some(error) = state.failure("lseek") {
                return Err(error);
            }
            if let Some(node) = state.nodes.get_mut(&descriptor.id) {
                node.directory_cursor = 0;
                state.override_cursor = 0;
                Ok(())
            } else {
                Err(libc::EBADF)
            }
        }

        fn getdents64(
            &mut self,
            descriptor: &Self::Handle,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "getdents64");
            if let Some(error) = state.failure("getdents") {
                return Err(error);
            }
            if let Some(raw) = state.directory_override.clone() {
                let start = state.override_cursor;
                let end = raw.len().min(start.saturating_add(buffer.len()));
                if start == end {
                    return Ok(0);
                }
                let count = end - start;
                buffer[..count].copy_from_slice(&raw[start..end]);
                state.override_cursor = end;
                return Ok(count);
            }

            let directory = state.path_for(descriptor.id).to_owned();
            let mut names = vec![b".".to_vec(), b"..".to_vec()];
            for path in state.entries.keys() {
                let parent = path
                    .rsplit_once('/')
                    .map(|(parent, _)| if parent.is_empty() { "/" } else { parent });
                if parent == Some(directory.as_str()) {
                    names.push(
                        path.rsplit('/')
                            .next()
                            .unwrap_or_default()
                            .as_bytes()
                            .to_vec(),
                    );
                }
            }
            names.sort();
            let cursor = state
                .nodes
                .get(&descriptor.id)
                .ok_or(libc::EBADF)?
                .directory_cursor;
            let mut output = Vec::new();
            let mut next = cursor;
            while next < names.len() {
                let record = dirent64(&names[next]);
                if output.len() + record.len() > buffer.len() {
                    if output.is_empty() {
                        return Err(libc::EINVAL);
                    }
                    break;
                }
                output.extend_from_slice(&record);
                next += 1;
            }
            if let Some(node) = state.nodes.get_mut(&descriptor.id) {
                node.directory_cursor = next;
            }
            let count = output.len();
            buffer[..count].copy_from_slice(&output);
            Ok(count)
        }

        fn write(&mut self, descriptor: &Self::Handle, bytes: &[u8]) -> Result<usize, libc::c_int> {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "write");
            if let Some(error) = state.failure("write") {
                return Err(error);
            }
            let count = bytes.len().min(state.max_write);
            if count == 0 {
                return Ok(0);
            }
            let append_extra = state.append_after_first_write;
            state.append_after_first_write = false;
            let tick = state.next_node as i64;
            let node = state.nodes.get_mut(&descriptor.id).ok_or(libc::EBADF)?;
            node.bytes.extend_from_slice(&bytes[..count]);
            if append_extra {
                node.bytes.push(b'x');
            }
            node.metadata.size = node.bytes.len() as i64;
            node.metadata.mtime_seconds = tick;
            node.metadata.mtime_nanoseconds += 1;
            node.metadata.ctime_seconds = tick;
            node.metadata.ctime_nanoseconds += 1;
            Ok(count)
        }

        fn pread(
            &mut self,
            descriptor: &Self::Handle,
            offset: i64,
            buffer: &mut [u8],
        ) -> Result<usize, libc::c_int> {
            let mut state = self.state.borrow_mut();
            Self::note(&mut state, "pread");
            if let Some(error) = state.failure("pread") {
                return Err(error);
            }
            let path = state.path_for(descriptor.id).to_owned();
            if let Some(mutation) = state.pread_mutations.remove(&path) {
                state.apply_mutation(mutation);
            }
            let max_read = state.max_read;
            let mut contents = state
                .nodes
                .get(&descriptor.id)
                .ok_or(libc::EBADF)?
                .bytes
                .clone();
            if let Some(size) = state.pread_size_override.get(&path) {
                contents.resize(*size, b' ');
            }
            let start = usize::try_from(offset).map_err(|_| libc::EINVAL)?;
            if start >= contents.len() {
                state.pread_requests.push((path, offset, buffer.len(), 0));
                return Ok(0);
            }
            let count = buffer.len().min(max_read).min(contents.len() - start);
            buffer[..count].copy_from_slice(&contents[start..start + count]);
            state
                .pread_requests
                .push((path, offset, buffer.len(), count));
            Ok(count)
        }

        fn fsync(&mut self, descriptor: &Self::Handle) -> Result<(), libc::c_int> {
            let mut state = self.state.borrow_mut();
            let path = state.path_for(descriptor.id).to_owned();
            let key = if path == RECORDS_PATH {
                "fsync_dir"
            } else {
                "fsync_file"
            };
            Self::note(&mut state, key);
            if let Some(error) = state.failure(key) {
                return Err(error);
            }
            state.apply_mutations(key);
            Ok(())
        }

        fn rename_at(
            &mut self,
            old_parent: &Self::Handle,
            old_name: &CStr,
            new_parent: &Self::Handle,
            new_name: &CStr,
            no_replace: bool,
        ) -> Result<(), libc::c_int> {
            let mut state = self.state.borrow_mut();
            let old_parent_path = state.path_for(old_parent.id).to_owned();
            let new_parent_path = state.path_for(new_parent.id).to_owned();
            let old_path = path_at(&old_parent_path, old_name);
            let new_path = path_at(&new_parent_path, new_name);
            Self::note(
                &mut state,
                if no_replace {
                    "rename:noreplace"
                } else {
                    "rename:replace"
                },
            );
            if let Some(error) = state.failure("rename") {
                return Err(error);
            }
            let source = state.id_for(&old_path).ok_or(libc::ENOENT)?;
            if no_replace && state.entries.contains_key(&new_path) {
                return Err(libc::EEXIST);
            }
            if !no_replace {
                state.entries.remove(&new_path);
            }
            state.entries.remove(&old_path);
            state.entries.insert(new_path.clone(), source);
            if let Some(node) = state.nodes.get_mut(&source) {
                node.path = new_path;
            }
            state.apply_mutations("rename");
            Ok(())
        }
    }

    fn dirent64(name: &[u8]) -> Vec<u8> {
        let raw_length = 19 + name.len() + 1;
        let record_length = (raw_length + 7) & !7;
        let mut record = vec![0_u8; record_length];
        record[16..18].copy_from_slice(&(record_length as u16).to_ne_bytes());
        record[19..19 + name.len()].copy_from_slice(name);
        record
    }

    #[derive(Clone)]
    struct FakeObservationPlan {
        generation: SimulationProcessGeneration,
        initial_boot_ms: u64,
        revalidations: VecDeque<Result<u64, SimulationProcessObservationError>>,
    }

    struct FakeObservation {
        plan: FakeObservationPlan,
        state: Rc<RefCell<FakeState>>,
        last_boot_ms: u64,
    }

    impl ObservationHandle for FakeObservation {
        fn generation(&self) -> SimulationProcessGeneration {
            self.plan.generation
        }

        fn initial_boot_ms(&self) -> u64 {
            self.plan.initial_boot_ms
        }

        fn revalidate(&mut self) -> Result<u64, SimulationProcessObservationError> {
            let mut state = self.state.borrow_mut();
            state.operations.push("observer_revalidate".to_owned());
            drop(state);
            match self.plan.revalidations.pop_front() {
                Some(Ok(value)) => {
                    self.last_boot_ms = value;
                    Ok(value)
                }
                Some(Err(error)) => Err(error),
                None => {
                    self.last_boot_ms = self.last_boot_ms.saturating_add(1);
                    Ok(self.last_boot_ms)
                }
            }
        }
    }

    struct FakeObservationFactory {
        state: Rc<RefCell<FakeState>>,
        plan: FakeObservationPlan,
        observe_error: Option<SimulationProcessObservationError>,
    }

    impl FakeObservationFactory {
        fn new(state: Rc<RefCell<FakeState>>) -> Self {
            Self {
                state,
                plan: FakeObservationPlan {
                    generation: generation(),
                    initial_boot_ms: 1_000,
                    revalidations: (1_001..1_100).map(Ok).collect(),
                },
                observe_error: None,
            }
        }
    }

    impl ObservationFactory for FakeObservationFactory {
        type Observation = FakeObservation;

        fn observe_current(
            &mut self,
        ) -> Result<Self::Observation, SimulationProcessObservationError> {
            self.state
                .borrow_mut()
                .operations
                .push("observe_current".to_owned());
            if let Some(error) = self.observe_error {
                return Err(error);
            }
            Ok(FakeObservation {
                plan: self.plan.clone(),
                state: self.state.clone(),
                last_boot_ms: self.plan.initial_boot_ms,
            })
        }
    }

    fn generation() -> SimulationProcessGeneration {
        SimulationProcessGeneration::new(
            42,
            9_876,
            Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap(),
        )
        .unwrap()
    }

    fn open_fixture() -> (
        WriterPolicy<FakeSyscalls, FakeObservation>,
        Rc<RefCell<FakeState>>,
        SimulationProcessConfig,
    ) {
        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        let writer = linux::open_with(syscalls, factory, &config)
            .expect("synthetic policy inputs publish Starting");
        (writer, state, config)
    }

    fn record_from_state(
        state: &Rc<RefCell<FakeState>>,
        config: &SimulationProcessConfig,
    ) -> SimulationHealthRecord {
        let state = state.borrow();
        let id = state
            .id_for(&format!("{RECORDS_PATH}/current.json"))
            .unwrap();
        let bytes = &state.nodes.get(&id).unwrap().bytes;
        SimulationHealthRecord::from_json(bytes, config, &generation())
            .expect("writer output uses the checked C3B wire contract")
    }

    fn bytes_at(state: &Rc<RefCell<FakeState>>, path: &str) -> Vec<u8> {
        let state = state.borrow();
        let id = state.id_for(path).expect("fixture path exists");
        state.nodes.get(&id).unwrap().bytes.clone()
    }

    #[test]
    fn checked_startup_and_event_lifecycle_publish_c3b_records() {
        let (mut writer, state, config) = open_fixture();
        let initial = record_from_state(&state, &config);
        assert_eq!(initial.phase(), SimulationHealthPhase::Starting);
        assert_eq!(initial.sequence(), 0);
        assert_eq!(initial.started_boot_ms(), 1_000);
        assert_eq!(initial.generation(), &generation());
        assert_eq!(initial.config_sha256(), config.config_sha256());
        assert_eq!(initial.receipt_sha256(), config.receipt_sha256());

        let events = [
            (
                SimulationHealthEvent::StartupVerified,
                SimulationHealthPhase::Ready,
            ),
            (
                SimulationHealthEvent::ClaimVerified,
                SimulationHealthPhase::Busy,
            ),
            (
                SimulationHealthEvent::LeaseVerified,
                SimulationHealthPhase::Busy,
            ),
            (
                SimulationHealthEvent::CycleVerified,
                SimulationHealthPhase::Ready,
            ),
            (SimulationHealthEvent::Sample, SimulationHealthPhase::Ready),
            (
                SimulationHealthEvent::ShutdownRequested,
                SimulationHealthPhase::Draining,
            ),
            (
                SimulationHealthEvent::Fatal(SimulationHealthFailure::RuntimeFailure),
                SimulationHealthPhase::Failed,
            ),
            (SimulationHealthEvent::Sample, SimulationHealthPhase::Failed),
        ];
        let mut previous_progress = None;
        for (index, (event, expected_phase)) in events.into_iter().enumerate() {
            writer.apply(event).expect("checked event publishes");
            let record = record_from_state(&state, &config);
            assert_eq!(record.phase(), expected_phase);
            assert_eq!(record.sequence(), index as u64 + 1);
            if event == SimulationHealthEvent::Sample {
                assert_eq!(record.progress_boot_ms(), previous_progress);
            }
            previous_progress = record.progress_boot_ms();
        }

        let state_ref = state.borrow();
        assert!(
            !state_ref
                .entries
                .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
        );
        assert!(
            state_ref
                .operations
                .iter()
                .any(|op| op == "rename:noreplace")
        );
        assert!(state_ref.operations.iter().any(|op| op == "rename:replace"));
        let lock = state_ref
            .operations
            .iter()
            .position(|op| op == "flock:LOCK_EX|LOCK_NB")
            .unwrap();
        let observed = state_ref
            .operations
            .iter()
            .position(|op| op == "observe_current")
            .unwrap();
        let created = state_ref
            .operations
            .iter()
            .position(|op| op.starts_with("create:"))
            .unwrap();
        let first_rename = state_ref
            .operations
            .iter()
            .position(|op| op == "rename:noreplace")
            .unwrap();
        let directory_sync = state_ref
            .operations
            .iter()
            .position(|op| op == "fsync_dir")
            .unwrap();
        assert!(lock < observed && observed < created && created < first_rename);
        assert!(first_rename < directory_sync);
        let first_write = state_ref
            .operations
            .iter()
            .position(|op| op == "write")
            .unwrap();
        let file_sync = state_ref
            .operations
            .iter()
            .position(|op| op == "fsync_file")
            .unwrap();
        let read_before_rename = state_ref
            .operations
            .iter()
            .enumerate()
            .filter(|(index, operation)| *index < first_rename && operation.as_str() == "pread")
            .map(|(index, _)| index)
            .last()
            .unwrap();
        let observer_before_rename = state_ref
            .operations
            .iter()
            .enumerate()
            .filter(|(index, operation)| {
                *index < first_rename && operation.as_str() == "observer_revalidate"
            })
            .map(|(index, _)| index)
            .last()
            .unwrap();
        let read_after_rename = state_ref
            .operations
            .iter()
            .enumerate()
            .find(|(index, operation)| *index > first_rename && operation.as_str() == "pread")
            .map(|(index, _)| index)
            .unwrap();
        let observer_after_rename = state_ref
            .operations
            .iter()
            .enumerate()
            .rfind(|(index, operation)| {
                *index > first_rename && operation.as_str() == "observer_revalidate"
            })
            .map(|(index, _)| index)
            .unwrap();
        assert!(first_write < file_sync && file_sync < read_before_rename);
        assert!(
            read_before_rename < observer_before_rename && observer_before_rename < first_rename
        );
        assert!(directory_sync < read_after_rename && read_after_rename < observer_after_rename);
        assert!(state_ref.open_calls.iter().any(|(path, flags, _)| {
            path.ends_with("/writer.lock")
                && *flags
                    == (libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        }));
        assert!(state_ref.open_calls.iter().any(|(_, flags, mode)| *flags
            == (libc::O_RDWR
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC)
            && *mode == 0o600));
        assert!(
            state_ref
                .fstatat_flags
                .iter()
                .all(|flags| *flags == libc::AT_SYMLINK_NOFOLLOW)
        );
    }

    #[test]
    fn initial_existing_current_pending_or_unknown_entry_is_dirty_without_mutation() {
        for name in ["current.json", ".pending.json", "foreign"] {
            let (syscalls, state) = FakeSyscalls::new();
            state
                .borrow_mut()
                .add_regular(&format!("{RECORDS_PATH}/{name}"), b"opaque");
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            assert!(matches!(
                linux::open_with(syscalls, factory, &config),
                Err(Error::DirtyRecords)
            ));
            let state = state.borrow();
            assert!(!state.operations.iter().any(|op| {
                op.starts_with("create:")
                    || op == "write"
                    || op.starts_with("fsync_")
                    || op.starts_with("rename:")
                    || op == "observe_current"
            }));
        }
    }

    #[test]
    fn nonblocking_lock_contender_never_observes_or_mutates_records() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().fail_on("flock", 1, libc::EAGAIN);
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::LockUnavailable)
        ));
        let state = state.borrow();
        assert!(
            state
                .operations
                .iter()
                .any(|op| op == "flock:LOCK_EX|LOCK_NB")
        );
        assert!(!state.operations.iter().any(|op| op == "observe_current"));
        assert!(!state.operations.iter().any(|op| op.starts_with("create:")));
        assert_eq!(state.acquired, state.dropped);
    }

    #[test]
    fn observation_failures_are_typed_and_do_not_cross_visibility() {
        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let mut factory = FakeObservationFactory::new(state.clone());
        factory.observe_error = Some(SimulationProcessObservationError::Changed);
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::GenerationInvalid)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op.starts_with("create:"))
        );

        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let mut factory = FakeObservationFactory::new(state.clone());
        factory.plan.revalidations =
            VecDeque::from([Err(SimulationProcessObservationError::ClockInvalid)]);
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::ClockInvalid)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op.starts_with("rename:"))
        );
        assert!(
            state
                .borrow()
                .entries
                .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
        );
    }

    #[test]
    fn changed_generation_before_and_after_rename_has_boundary_specific_outcome() {
        for (revalidations, expected, current_visible, rename_count) in [
            (
                VecDeque::from([Err(SimulationProcessObservationError::Changed)]),
                Error::GenerationInvalid,
                false,
                0,
            ),
            (
                VecDeque::from([Ok(1_001), Err(SimulationProcessObservationError::Changed)]),
                Error::PublicationUnconfirmed,
                true,
                1,
            ),
        ] {
            let (syscalls, state) = FakeSyscalls::new();
            let config = checked_config();
            let mut factory = FakeObservationFactory::new(state.clone());
            factory.plan.revalidations = revalidations;
            assert!(matches!(
                linux::open_with(syscalls, factory, &config),
                Err(error) if error == expected
            ));
            let state_ref = state.borrow();
            assert_eq!(
                state_ref.entries.contains_key(CURRENT_PATH),
                current_visible
            );
            assert_eq!(
                state_ref.entries.contains_key(PENDING_PATH),
                !current_visible
            );
            assert_eq!(
                state_ref
                    .operations
                    .iter()
                    .filter(|operation| operation.as_str() == "rename:noreplace")
                    .count(),
                rename_count
            );
            assert_eq!(
                state_ref
                    .operations
                    .iter()
                    .filter(|operation| operation.as_str() == "observer_revalidate")
                    .count(),
                if current_visible { 2 } else { 1 }
            );
            assert_eq!(state_ref.acquired, state_ref.dropped);
        }
    }

    #[test]
    fn short_writes_and_reads_complete_with_exact_same_descriptor_readback() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().max_write = 5;
        state.borrow_mut().max_read = 7;
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        let writer = linux::open_with(syscalls, factory, &config).unwrap();
        let state_ref = state.borrow();
        assert!(
            state_ref
                .operations
                .iter()
                .filter(|op| op.as_str() == "write")
                .count()
                > 1
        );
        assert!(
            state_ref
                .operations
                .iter()
                .filter(|op| op.as_str() == "pread")
                .count()
                > 1
        );
        assert!(
            state_ref
                .entries
                .contains_key(&format!("{RECORDS_PATH}/current.json"))
        );
        drop(state_ref);
        drop(writer);
        let state_ref = state.borrow();
        assert_eq!(state_ref.acquired, state_ref.dropped);
    }

    #[test]
    fn every_apply_failure_poison_prevents_all_later_io_and_keeps_lock_until_drop() {
        let (mut writer, state, _) = open_fixture();
        assert_eq!(
            writer.apply(SimulationHealthEvent::LeaseVerified),
            Err(Error::HealthRejected)
        );
        assert!(writer.is_invalidated());
        let operations_at_failure = state.borrow().operations.len();
        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        assert!(
            state
                .borrow()
                .operations
                .iter()
                .any(|op| op == "flock:LOCK_EX|LOCK_NB")
        );
        assert!(state.borrow().dropped < state.borrow().acquired);
        drop(writer);
        assert_eq!(state.borrow().dropped, state.borrow().acquired);
    }

    #[test]
    fn syscall_failures_are_bounded_and_visibility_depends_on_rename_boundary() {
        let cases = [
            ("create", 1, libc::EINTR, Error::Unavailable, false),
            ("write", 1, libc::EINTR, Error::Unavailable, false),
            ("fsync_file", 1, libc::EIO, Error::Unavailable, false),
            ("pread", 1, libc::EINTR, Error::Unavailable, false),
            ("rename", 1, libc::EIO, Error::PublicationUnconfirmed, false),
            (
                "fsync_dir",
                1,
                libc::EIO,
                Error::PublicationUnconfirmed,
                true,
            ),
        ];
        for (operation, call, errno, expected, current_visible) in cases {
            let (syscalls, state) = FakeSyscalls::new();
            state.borrow_mut().fail_on(operation, call, errno);
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            let result = linux::open_with(syscalls, factory, &config);
            assert!(
                matches!(result, Err(error) if error == expected),
                "{operation}"
            );
            let state_ref = state.borrow();
            assert_eq!(
                state_ref
                    .entries
                    .contains_key(&format!("{RECORDS_PATH}/current.json")),
                current_visible,
                "{operation}"
            );
            assert_eq!(
                state_ref
                    .entries
                    .contains_key(&format!("{RECORDS_PATH}/.pending.json")),
                operation != "create" && !current_visible,
                "{operation}"
            );
            assert_eq!(state_ref.acquired, state_ref.dropped, "{operation}");
            assert_eq!(
                state_ref.call_counts.get(operation).copied().unwrap_or(0),
                1,
                "failed operation is never retried: {operation}"
            );
        }
    }

    #[test]
    fn post_rename_observation_failure_is_unconfirmed_and_never_restored() {
        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let mut factory = FakeObservationFactory::new(state.clone());
        factory.plan.revalidations = VecDeque::from([
            Ok(1_001),
            Err(SimulationProcessObservationError::ClockInvalid),
        ]);
        let result = linux::open_with(syscalls, factory, &config);
        assert!(matches!(result, Err(Error::PublicationUnconfirmed)));
        let state_ref = state.borrow();
        assert!(
            state_ref
                .entries
                .contains_key(&format!("{RECORDS_PATH}/current.json"))
        );
        assert!(
            !state_ref
                .entries
                .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
        );
        assert!(
            state_ref
                .operations
                .iter()
                .any(|op| op == "rename:noreplace")
        );
        assert_eq!(state_ref.acquired, state_ref.dropped);
    }

    #[test]
    fn post_rename_apply_failure_keeps_new_record_and_poisoned_lock() {
        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let mut factory = FakeObservationFactory::new(state.clone());
        factory.plan.revalidations = VecDeque::from([
            Ok(1_001),
            Ok(1_001),
            Ok(1_002),
            Ok(1_003),
            Err(SimulationProcessObservationError::ClockInvalid),
        ]);
        let mut writer = linux::open_with(syscalls, factory, &config)
            .expect("synthetic policy inputs publish Starting");

        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::PublicationUnconfirmed)
        );
        assert!(writer.is_invalidated());
        let published = record_from_state(&state, &config);
        assert_eq!(published.phase(), SimulationHealthPhase::Ready);
        assert_eq!(published.sequence(), 1);
        let state_ref = state.borrow();
        assert!(
            !state_ref
                .entries
                .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
        );
        assert!(state_ref.operations.iter().any(|op| op == "rename:replace"));
        assert!(state_ref.dropped < state_ref.acquired);
        let operations_at_failure = state_ref.operations.len();
        drop(state_ref);

        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        drop(writer);
        assert_eq!(state.borrow().acquired, state.borrow().dropped);
    }

    #[test]
    fn stable_in_range_size_disagreement_fails_before_rename_and_preserves_prior_current() {
        let (mut writer, state, config) = open_fixture();
        let prior_current = bytes_at(&state, CURRENT_PATH);
        state.borrow_mut().mutate_on(
            "fsync_file",
            2,
            FakeMutation::SizeDelta {
                path: PENDING_PATH.to_owned(),
                delta: -1,
            },
        );
        let before = state.borrow().operations.len();

        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::Changed)
        );
        assert!(writer.is_invalidated());
        assert_eq!(bytes_at(&state, CURRENT_PATH), prior_current);
        assert!(state.borrow().entries.contains_key(PENDING_PATH));
        let state_ref = state.borrow();
        let pending_id = state_ref.id_for(PENDING_PATH).unwrap();
        let pending = state_ref.nodes.get(&pending_id).unwrap();
        assert!(pending.metadata.size > 0 && pending.metadata.size <= 8_192);
        assert_ne!(pending.metadata.size as usize, pending.bytes.len());
        assert!(SimulationHealthRecord::from_json(&pending.bytes, &config, &generation()).is_ok());
        assert!(
            !state_ref.operations[before..]
                .iter()
                .any(|operation| operation.starts_with("rename:"))
        );
        assert!(
            !state_ref
                .operations
                .iter()
                .any(|operation| operation.starts_with("unlink"))
        );
        assert!(state_ref.dropped < state_ref.acquired);
        let operations_at_failure = state_ref.operations.len();
        drop(state_ref);
        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        drop(writer);
        assert_eq!(state.borrow().acquired, state.borrow().dropped);
    }

    #[test]
    fn stable_in_range_size_disagreement_after_rename_is_unconfirmed_and_poisoned() {
        let (mut writer, state, config) = open_fixture();
        state.borrow_mut().mutate_on(
            "rename",
            2,
            FakeMutation::SizeDelta {
                path: CURRENT_PATH.to_owned(),
                delta: -1,
            },
        );

        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::PublicationUnconfirmed)
        );
        assert!(writer.is_invalidated());
        assert_eq!(record_from_state(&state, &config).sequence(), 1);
        assert!(!state.borrow().entries.contains_key(PENDING_PATH));
        let current_id = state.borrow().id_for(CURRENT_PATH).unwrap();
        let current = state.borrow();
        let published = current.nodes.get(&current_id).unwrap();
        assert!(published.metadata.size > 0 && published.metadata.size <= 8_192);
        assert_ne!(published.metadata.size as usize, published.bytes.len());
        assert_eq!(current.call_counts.get("rename"), Some(&2));
        assert!(current.dropped < current.acquired);
        let operations_at_failure = current.operations.len();
        drop(current);

        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        drop(writer);
        assert_eq!(state.borrow().acquired, state.borrow().dropped);
    }

    #[test]
    fn read_time_cap_plus_one_with_in_range_metadata_is_bounded_and_not_published() {
        let (mut writer, state, _) = open_fixture();
        let pread_requests_before_apply = state.borrow().pread_requests.len();
        let prior_current = bytes_at(&state, CURRENT_PATH);
        state
            .borrow_mut()
            .pread_size_override
            .insert(PENDING_PATH.to_owned(), 8_193);
        let before = state.borrow().operations.len();

        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::SizeLimit)
        );
        assert!(writer.is_invalidated());
        assert_eq!(bytes_at(&state, CURRENT_PATH), prior_current);
        assert!(state.borrow().entries.contains_key(PENDING_PATH));
        let state_ref = state.borrow();
        let pending_id = state_ref.id_for(PENDING_PATH).unwrap();
        let metadata = state_ref.nodes.get(&pending_id).unwrap().metadata;
        assert!(metadata.size > 0 && metadata.size <= 8_192);
        let requests = state_ref.pread_requests[pread_requests_before_apply..]
            .iter()
            .filter(|(path, _, _, _)| path == PENDING_PATH)
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 9);
        assert_eq!(
            requests.iter().map(|request| request.3).sum::<usize>(),
            8_193
        );
        assert!(
            requests
                .iter()
                .all(|request| request.2 <= 1_024 && request.3 > 0)
        );
        for pair in requests.windows(2) {
            assert_eq!(pair[1].1, pair[0].1 + pair[0].3 as i64);
        }
        assert!(
            !state_ref.operations[before..]
                .iter()
                .any(|operation| operation.starts_with("rename:"))
        );
        assert!(
            !state_ref
                .operations
                .iter()
                .any(|operation| operation.starts_with("unlink"))
        );
        assert!(state_ref.dropped < state_ref.acquired);
        let operations_at_failure = state_ref.operations.len();
        drop(state_ref);
        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        drop(writer);
        assert_eq!(state.borrow().acquired, state.borrow().dropped);
    }

    #[test]
    fn size_or_time_change_during_readback_fails_before_visibility() {
        for mutation in [
            FakeMutation::SizeDelta {
                path: PENDING_PATH.to_owned(),
                delta: 1,
            },
            FakeMutation::BumpMtime(PENDING_PATH.to_owned()),
        ] {
            let (mut writer, state, _) = open_fixture();
            let prior_current = bytes_at(&state, CURRENT_PATH);
            state
                .borrow_mut()
                .pread_mutations
                .insert(PENDING_PATH.to_owned(), mutation);
            let before = state.borrow().operations.len();

            assert_eq!(
                writer.apply(SimulationHealthEvent::StartupVerified),
                Err(Error::Changed)
            );
            assert!(writer.is_invalidated());
            assert_eq!(bytes_at(&state, CURRENT_PATH), prior_current);
            assert!(state.borrow().entries.contains_key(PENDING_PATH));
            let state_ref = state.borrow();
            assert!(
                !state_ref.operations[before..]
                    .iter()
                    .any(|operation| operation.starts_with("rename:"))
            );
            assert!(
                !state_ref
                    .operations
                    .iter()
                    .any(|operation| operation.starts_with("unlink"))
            );
            assert!(state_ref.dropped < state_ref.acquired);
            let operations_at_failure = state_ref.operations.len();
            drop(state_ref);
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(Error::Invalidated)
            );
            assert_eq!(state.borrow().operations.len(), operations_at_failure);
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }
    }

    #[test]
    fn create_collision_preserves_pending_and_prior_current_without_retry() {
        let (mut writer, state, _) = open_fixture();
        let prior_current = bytes_at(&state, CURRENT_PATH);
        let collision = b"foreign pending evidence".to_vec();
        state.borrow_mut().mutate_on(
            "create",
            2,
            FakeMutation::InsertRegular {
                path: PENDING_PATH.to_owned(),
                bytes: collision.clone(),
            },
        );
        let before = state.borrow().operations.len();

        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::Unavailable)
        );
        assert!(writer.is_invalidated());
        assert_eq!(bytes_at(&state, CURRENT_PATH), prior_current);
        assert_eq!(bytes_at(&state, PENDING_PATH), collision);
        let state_ref = state.borrow();
        assert_eq!(state_ref.call_counts.get("create"), Some(&2));
        assert!(
            !state_ref.operations[before..]
                .iter()
                .any(|operation| operation == "write" || operation.starts_with("rename:"))
        );
        assert!(
            !state_ref
                .operations
                .iter()
                .any(|operation| operation.starts_with("unlink"))
        );
        assert!(state_ref.dropped < state_ref.acquired);
        let operations_at_failure = state_ref.operations.len();
        drop(state_ref);
        assert_eq!(
            writer.apply(SimulationHealthEvent::Sample),
            Err(Error::Invalidated)
        );
        assert_eq!(state.borrow().operations.len(), operations_at_failure);
        drop(writer);
        assert_eq!(state.borrow().acquired, state.borrow().dropped);
    }

    #[test]
    fn pre_rename_ancestor_or_current_replacement_keeps_old_publication() {
        for path in ["/run", CURRENT_PATH] {
            let (mut writer, state, _) = open_fixture();
            let prior_current = bytes_at(&state, CURRENT_PATH);
            state.borrow_mut().mutate_on(
                "fsync_file",
                2,
                FakeMutation::ReplacePath(path.to_owned()),
            );
            let before = state.borrow().operations.len();

            assert_eq!(
                writer.apply(SimulationHealthEvent::StartupVerified),
                Err(Error::Changed),
                "{path}"
            );
            assert!(writer.is_invalidated());
            assert_eq!(bytes_at(&state, CURRENT_PATH), prior_current);
            assert!(state.borrow().entries.contains_key(PENDING_PATH));
            let state_ref = state.borrow();
            assert!(
                !state_ref.operations[before..]
                    .iter()
                    .any(|operation| operation.starts_with("rename:"))
            );
            assert!(
                !state_ref
                    .operations
                    .iter()
                    .any(|operation| operation.starts_with("unlink"))
            );
            assert!(state_ref.dropped < state_ref.acquired);
            let operations_at_failure = state_ref.operations.len();
            drop(state_ref);
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(Error::Invalidated)
            );
            assert_eq!(state.borrow().operations.len(), operations_at_failure);
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }
    }

    #[test]
    fn post_rename_ancestor_or_current_replacement_is_unconfirmed_and_not_restored() {
        for path in ["/run", CURRENT_PATH] {
            let (mut writer, state, config) = open_fixture();
            let target_inode_before = {
                let state_ref = state.borrow();
                let id = state_ref.id_for(path).unwrap();
                state_ref.nodes.get(&id).unwrap().metadata.inode
            };
            state
                .borrow_mut()
                .mutate_on("rename", 2, FakeMutation::ReplacePath(path.to_owned()));
            let before = state.borrow().operations.len();

            assert_eq!(
                writer.apply(SimulationHealthEvent::StartupVerified),
                Err(Error::PublicationUnconfirmed),
                "{path}"
            );
            assert!(writer.is_invalidated());
            assert_eq!(record_from_state(&state, &config).sequence(), 1);
            assert!(!state.borrow().entries.contains_key(PENDING_PATH));
            let state_ref = state.borrow();
            let target_id = state_ref.id_for(path).unwrap();
            assert_ne!(
                state_ref.nodes.get(&target_id).unwrap().metadata.inode,
                target_inode_before
            );
            assert!(
                state_ref.operations[before..]
                    .iter()
                    .any(|operation| operation == "rename:replace")
            );
            assert_eq!(state_ref.call_counts.get("rename"), Some(&2));
            assert!(
                !state_ref
                    .operations
                    .iter()
                    .any(|operation| operation.starts_with("unlink"))
            );
            assert!(state_ref.dropped < state_ref.acquired);
            let operations_at_failure = state_ref.operations.len();
            drop(state_ref);
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(Error::Invalidated)
            );
            assert_eq!(state.borrow().operations.len(), operations_at_failure);
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }
    }

    #[test]
    fn altered_readback_and_backward_clock_fail_closed_without_retry() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().append_after_first_write = true;
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        let result = linux::open_with(syscalls, factory, &config);
        assert!(matches!(
            result,
            Err(Error::Changed) | Err(Error::HealthRejected)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op.starts_with("rename:"))
        );

        let (syscalls, state) = FakeSyscalls::new();
        let config = checked_config();
        let mut factory = FakeObservationFactory::new(state.clone());
        factory.plan.revalidations = VecDeque::from([Ok(1_001), Ok(1_002), Ok(999)]);
        let mut writer = linux::open_with(syscalls, factory, &config).unwrap();
        let operations_before = state.borrow().operations.len();
        assert_eq!(
            writer.apply(SimulationHealthEvent::StartupVerified),
            Err(Error::ClockInvalid)
        );
        assert!(writer.is_invalidated());
        assert_eq!(record_from_state(&state, &config).sequence(), 0);
        assert!(
            !state.borrow().operations[operations_before..]
                .iter()
                .any(|operation| operation.starts_with("create:")
                    || operation.starts_with("rename:"))
        );
    }

    #[test]
    fn zero_write_and_zero_length_read_are_terminal_failures() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().max_write = 0;
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::Unavailable)
        ));
        assert_eq!(state.borrow().call_counts.get("write"), Some(&1));
        assert!(
            state
                .borrow()
                .entries
                .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
        );

        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().max_read = 0;
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::HealthRejected)
        ));
        assert_eq!(state.borrow().call_counts.get("pread"), Some(&1));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op.starts_with("rename:"))
        );
    }

    #[test]
    fn temporary_acl_and_creation_mode_faults_leave_the_pending_evidence() {
        for (mode, acl) in [(Some(0o640), false), (None, true)] {
            let (syscalls, state) = FakeSyscalls::new();
            {
                let mut state_ref = state.borrow_mut();
                state_ref.create_mode_override = mode;
                state_ref.temp_acl_present = acl;
            }
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            assert!(matches!(
                linux::open_with(syscalls, factory, &config),
                Err(Error::InvalidMetadata)
            ));
            let state_ref = state.borrow();
            assert!(
                state_ref
                    .entries
                    .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
            );
            assert!(!state_ref.operations.iter().any(|op| {
                op == "write" || op.starts_with("rename:") || op.starts_with("unlink")
            }));
            assert_eq!(state_ref.acquired, state_ref.dropped);
        }
    }

    #[test]
    fn temporary_type_owner_group_link_device_and_size_faults_fail_before_write() {
        for fault in ["type", "owner", "group", "mode", "links", "device", "size"] {
            let (syscalls, state) = FakeSyscalls::new();
            let mut bad = metadata(2, 777, libc::S_IFREG as u32, 10001, 10001, 0o600, 0);
            match fault {
                "type" => bad.mode = libc::S_IFLNK as u32 | 0o777,
                "owner" => bad.uid = 1,
                "group" => bad.gid = 2,
                "mode" => bad.mode = libc::S_IFREG as u32 | 0o640,
                "links" => bad.links = 2,
                "device" => bad.device = 1,
                "size" => bad.size = 8_193,
                _ => unreachable!(),
            }
            state.borrow_mut().create_metadata_override = Some(bad);
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            let expected = if fault == "type" {
                Error::UnsafePath
            } else {
                Error::InvalidMetadata
            };
            assert!(
                matches!(
                    linux::open_with(syscalls, factory, &config),
                    Err(error) if error == expected
                ),
                "{fault}"
            );
            let state_ref = state.borrow();
            assert!(!state_ref.operations.iter().any(|op| op == "write"));
            assert!(
                state_ref
                    .entries
                    .contains_key(&format!("{RECORDS_PATH}/.pending.json"))
            );
            assert_eq!(state_ref.acquired, state_ref.dropped);
        }
    }

    #[test]
    fn effective_identity_and_acl_metadata_fail_before_observation() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().effective_uid = 1000;
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::InvalidIdentity)
        ));
        assert_eq!(state.borrow().operations.len(), 1);
        assert_eq!(state.borrow().operations[0], "ids");

        let (syscalls, state) = FakeSyscalls::new();
        {
            let mut state_ref = state.borrow_mut();
            let id = state_ref.id_for(RECORDS_PATH).unwrap();
            state_ref
                .nodes
                .get_mut(&id)
                .unwrap()
                .acls
                .insert("system.posix_acl_default".to_owned(), Some(0));
        }
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::InvalidMetadata)
        ));
        let oversized = metadata(2, 999, libc::S_IFREG as u32, 10001, 10001, 0o600, 8_193);
        assert_eq!(
            linux::validate_leaf_metadata(oversized, linux::RECORD_POLICY),
            Err(Error::SizeLimit)
        );
        let empty = metadata(2, 999, libc::S_IFREG as u32, 10001, 10001, 0o600, 0);
        assert_eq!(
            linux::validate_leaf_metadata(empty, linux::RECORD_POLICY),
            Err(Error::InvalidMetadata)
        );
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op == "observe_current")
        );
    }

    #[test]
    fn apply_rechecks_uid_and_gid_before_filesystem_or_observation_work() {
        for (uid, gid) in [(1000, 10001), (10001, 1000)] {
            let (mut writer, state, _) = open_fixture();
            state.borrow_mut().effective_uid = uid;
            state.borrow_mut().effective_gid = gid;
            let before = state.borrow().operations.len();
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(Error::InvalidIdentity)
            );
            assert!(writer.is_invalidated());
            assert_eq!(&state.borrow().operations[before..], &["ids".to_owned()]);
        }
    }

    #[test]
    fn fixed_tree_metadata_modes_types_devices_and_tmpfs_are_checked() {
        let cases = [
            ("/", "owner", Error::InvalidMetadata),
            ("/", "special", Error::InvalidMetadata),
            ("/", "type", Error::UnsafePath),
            ("/run", "owner", Error::InvalidMetadata),
            ("/run", "group_write", Error::InvalidMetadata),
            (PROCESS_PATH, "mode", Error::InvalidMetadata),
            (PROCESS_PATH, "gid", Error::InvalidMetadata),
            (PROCESS_PATH, "type", Error::UnsafePath),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "links",
                Error::InvalidMetadata,
            ),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "lock_mode",
                Error::InvalidMetadata,
            ),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "owner",
                Error::InvalidMetadata,
            ),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "gid",
                Error::InvalidMetadata,
            ),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "type",
                Error::UnsafePath,
            ),
            (RECORDS_PATH, "mode", Error::InvalidMetadata),
            (RECORDS_PATH, "owner", Error::InvalidMetadata),
            (RECORDS_PATH, "same_device", Error::InvalidMetadata),
            (RECORDS_PATH, "type", Error::UnsafePath),
            (
                "/run/owner-beta-simulation-health/writer.lock",
                "group",
                Error::InvalidMetadata,
            ),
            (RECORDS_PATH, "gid", Error::InvalidMetadata),
        ];
        for (path, fault, expected) in cases {
            let (syscalls, state) = FakeSyscalls::new();
            {
                let mut state_ref = state.borrow_mut();
                let id = state_ref.id_for(path).unwrap();
                let node = state_ref.nodes.get_mut(&id).unwrap();
                match fault {
                    "owner" => node.metadata.uid = 1,
                    "mode" => node.metadata.mode = libc::S_IFDIR as u32 | 0o770,
                    "special" => node.metadata.mode |= 0o4000,
                    "group_write" => node.metadata.mode |= 0o020,
                    "gid" => node.metadata.gid = 2,
                    "links" => node.metadata.links = 2,
                    "type" => node.metadata.mode = libc::S_IFLNK as u32 | 0o777,
                    "lock_mode" => node.metadata.mode = libc::S_IFREG as u32 | 0o400,
                    "same_device" => node.metadata.device = 1,
                    "group" => node.metadata.gid = 10002,
                    _ => unreachable!(),
                }
            }
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            assert!(
                matches!(
                    linux::open_with(syscalls, factory, &config),
                    Err(error) if error == expected
                ),
                "{path} {fault}"
            );
            assert!(
                !state
                    .borrow()
                    .operations
                    .iter()
                    .any(|op| op == "observe_current")
            );
        }

        let (syscalls, state) = FakeSyscalls::new();
        {
            let mut state_ref = state.borrow_mut();
            let id = state_ref.id_for(RECORDS_PATH).unwrap();
            state_ref.nodes.get_mut(&id).unwrap().filesystem_type = 0;
        }
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::InvalidMetadata)
        ));
    }

    #[test]
    fn access_and_default_acls_are_rejected_on_every_retained_tree_object() {
        for path in [
            "/",
            "/run",
            PROCESS_PATH,
            "/run/owner-beta-simulation-health/writer.lock",
            RECORDS_PATH,
        ] {
            for acl in ["system.posix_acl_access", "system.posix_acl_default"] {
                let (syscalls, state) = FakeSyscalls::new();
                {
                    let mut state_ref = state.borrow_mut();
                    let id = state_ref.id_for(path).unwrap();
                    state_ref
                        .nodes
                        .get_mut(&id)
                        .unwrap()
                        .acls
                        .insert(acl.to_owned(), Some(0));
                }
                let config = checked_config();
                let factory = FakeObservationFactory::new(state.clone());
                assert!(
                    matches!(
                        linux::open_with(syscalls, factory, &config),
                        Err(Error::InvalidMetadata)
                    ),
                    "{path} {acl}"
                );
                assert!(
                    !state
                        .borrow()
                        .operations
                        .iter()
                        .any(|operation| operation == "observe_current")
                );
            }
        }
    }

    #[test]
    fn acl_absence_errno_is_accepted_but_other_xattr_errors_fail_once() {
        for errno in [libc::ENODATA, libc::ENOTSUP, libc::EOPNOTSUPP] {
            let (syscalls, state) = FakeSyscalls::new();
            state.borrow_mut().fail_on("fgetxattr", 1, errno);
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            let writer = linux::open_with(syscalls, factory, &config)
                .expect("documented ACL-absence errors mean no ACL");
            assert!(
                state
                    .borrow()
                    .operations
                    .iter()
                    .any(|op| op == "observe_current")
            );
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }

        for errno in [libc::EIO, libc::EINTR] {
            let (syscalls, state) = FakeSyscalls::new();
            state.borrow_mut().fail_on("fgetxattr", 1, errno);
            let config = checked_config();
            let factory = FakeObservationFactory::new(state.clone());
            assert!(matches!(
                linux::open_with(syscalls, factory, &config),
                Err(Error::Unavailable)
            ));
            let state_ref = state.borrow();
            assert_eq!(state_ref.call_counts.get("fgetxattr"), Some(&1));
            assert!(
                !state_ref
                    .operations
                    .iter()
                    .any(|op| op == "observe_current")
            );
            assert!(
                !state_ref
                    .operations
                    .iter()
                    .any(|op| op.starts_with("create:"))
            );
            assert_eq!(state_ref.acquired, state_ref.dropped);
        }
    }

    #[test]
    fn current_record_type_owner_group_mode_link_acl_device_and_size_faults_fail_closed() {
        for fault in [
            "type",
            "owner",
            "group",
            "mode",
            "links",
            "access_acl",
            "default_acl",
            "device",
            "size",
        ] {
            let (mut writer, state, _) = open_fixture();
            {
                let mut state_ref = state.borrow_mut();
                let id = state_ref
                    .id_for(&format!("{RECORDS_PATH}/current.json"))
                    .unwrap();
                let node = state_ref.nodes.get_mut(&id).unwrap();
                match fault {
                    "type" => node.metadata.mode = libc::S_IFLNK as u32 | 0o777,
                    "owner" => node.metadata.uid = 1,
                    "group" => node.metadata.gid = 2,
                    "mode" => node.metadata.mode = libc::S_IFREG as u32 | 0o640,
                    "links" => node.metadata.links = 2,
                    "access_acl" => {
                        node.acls
                            .insert("system.posix_acl_access".to_owned(), Some(0));
                    }
                    "default_acl" => {
                        node.acls
                            .insert("system.posix_acl_default".to_owned(), Some(0));
                    }
                    "device" => node.metadata.device = 1,
                    "size" => node.metadata.size = 8_193,
                    _ => unreachable!(),
                }
            }
            let before = state.borrow().operations.len();
            let expected = match fault {
                "type" => Error::UnsafePath,
                "device" => Error::Changed,
                "size" => Error::SizeLimit,
                _ => Error::InvalidMetadata,
            };
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(expected),
                "{fault}"
            );
            assert!(writer.is_invalidated());
            assert!(
                !state.borrow().operations[before..]
                    .iter()
                    .any(|operation| operation == "observer_revalidate"
                        || operation.starts_with("create:")),
                "{fault}"
            );
            assert!(state.borrow().dropped < state.borrow().acquired);
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }
    }

    #[test]
    fn descriptor_errno_classification_never_retries_interrupts() {
        for (errno, expected) in [
            (libc::ELOOP, Error::UnsafePath),
            (libc::ENOTDIR, Error::UnsafePath),
            (libc::EISDIR, Error::UnsafePath),
            (libc::EINTR, Error::Unavailable),
        ] {
            for operation in ["open_path", "open_at"] {
                let (syscalls, state) = FakeSyscalls::new();
                state.borrow_mut().fail_on(operation, 1, errno);
                let config = checked_config();
                let factory = FakeObservationFactory::new(state.clone());
                assert!(
                    matches!(
                        linux::open_with(syscalls, factory, &config),
                        Err(error) if error == expected
                    ),
                    "{operation} {errno}"
                );
                assert_eq!(state.borrow().call_counts.get(operation), Some(&1));
                assert_eq!(state.borrow().acquired, state.borrow().dropped);
            }
        }
    }

    #[test]
    fn raw_dirent_parser_rejects_truncation_duplicates_missing_names_and_slashes() {
        let parse = |bytes: &[u8]| {
            let mut names = HashSet::new();
            linux::parse_dirent64(bytes, &mut names)
        };
        let dot = dirent64(b".");
        let dotdot = dirent64(b"..");
        let mut valid = dot.clone();
        valid.extend_from_slice(&dotdot);
        assert_eq!(parse(&valid), Ok(()));
        let mut nonzero_padding = dot.clone();
        *nonzero_padding.last_mut().unwrap() = 0xa5;
        assert_eq!(parse(&nonzero_padding), Ok(()));
        assert_eq!(parse(&dot[..5]), Err(Error::InvalidMetadata));
        let mut duplicate = dot.clone();
        duplicate.extend_from_slice(&dot);
        assert_eq!(parse(&duplicate), Err(Error::InvalidMetadata));
        assert_eq!(parse(&dirent64(b"bad/name")), Err(Error::InvalidMetadata));
        let mut unterminated = dirent64(b"entry");
        unterminated[19..].fill(b'x');
        assert_eq!(parse(&unterminated), Err(Error::InvalidMetadata));
    }

    #[test]
    fn malformed_inventory_stops_before_observation_and_temp_creation() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().directory_override = Some(vec![0; 5]);
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::InvalidMetadata)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| { op == "observe_current" || op.starts_with("create:") })
        );
    }

    #[test]
    fn interrupted_inventory_read_is_not_retried() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().fail_on("getdents", 1, libc::EINTR);
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::Unavailable)
        ));
        assert_eq!(state.borrow().call_counts.get("getdents"), Some(&1));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op == "observe_current")
        );
    }

    #[test]
    fn missing_dot_entry_and_over_limit_inventory_are_rejected() {
        let (syscalls, state) = FakeSyscalls::new();
        state.borrow_mut().directory_override = Some(dirent64(b".."));
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::DirtyRecords)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op == "observe_current")
        );

        let (syscalls, state) = FakeSyscalls::new();
        {
            let mut state_ref = state.borrow_mut();
            for index in 0..50 {
                let name = format!("entry-{index:02}-{}", "x".repeat(180));
                state_ref.add_regular(&format!("{RECORDS_PATH}/{name}"), b"");
            }
        }
        let config = checked_config();
        let factory = FakeObservationFactory::new(state.clone());
        assert!(matches!(
            linux::open_with(syscalls, factory, &config),
            Err(Error::SizeLimit)
        ));
        assert!(
            !state
                .borrow()
                .operations
                .iter()
                .any(|op| op == "observe_current")
        );
    }

    #[test]
    fn replacing_retained_ancestor_lock_or_published_inode_before_apply_is_detected() {
        for path in [
            "/",
            "/run",
            PROCESS_PATH,
            "/run/owner-beta-simulation-health/writer.lock",
            CURRENT_PATH,
        ] {
            let (mut writer, state, _) = open_fixture();
            {
                let mut state_ref = state.borrow_mut();
                let id = state_ref.id_for(path).unwrap();
                let mut replacement = state_ref.nodes.get(&id).unwrap().clone();
                replacement.metadata.inode += 100;
                state_ref.insert(replacement);
            }
            let before = state.borrow().operations.len();
            assert_eq!(
                writer.apply(SimulationHealthEvent::Sample),
                Err(Error::Changed),
                "{path}"
            );
            assert!(writer.is_invalidated());
            assert!(
                !state.borrow().operations[before..]
                    .iter()
                    .any(|op| op == "observer_revalidate" || op.starts_with("create:"))
            );
            assert!(state.borrow().dropped < state.borrow().acquired);
            drop(writer);
            assert_eq!(state.borrow().acquired, state.borrow().dropped);
        }
    }

    #[test]
    fn health_sequence_overflow_maps_to_a_value_free_rejection() {
        assert_eq!(
            linux::map_health_error(
                crate::owner_beta::simulation_execution::SimulationHealthError::SequenceExhausted
            ),
            Error::HealthRejected
        );
    }

    #[test]
    fn writer_and_errors_have_redacted_debug_output() {
        let (writer, _, _) = open_fixture();
        let writer = SimulationHealthWriter {
            session: Box::new(writer),
        };
        assert_eq!(format!("{writer:?}"), "SimulationHealthWriter");
        assert_eq!(format!("{:?}", Error::Unavailable), "Unavailable");
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_factory_fails_before_config_inspection_or_observation() {
    assert!(matches!(
        super::open_simulation_health_writer(&checked_config()),
        Err(Error::UnsupportedPlatform)
    ));
}
