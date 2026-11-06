//! Retained, fixed-path reader for the first live maintenance action.
//!
//! The parsed records remain descriptions. Authority comes from keeping every
//! original directory/file descriptor and rechecking its fixed name and bytes.
use std::{
    collections::BTreeSet,
    ffi::CString,
    io::Read,
    os::{fd::AsRawFd, unix::fs::FileExt},
};

use serde_json::Value;

use super::bound::BoundOriginals;
use super::wire::{self, Intent, RootIntent};
use super::*;

const OPERATION_PATH: &str = "/run/owner-beta-paper-ops/operation";
const PAPER_STATE_PATH: &str = "/run/owner-beta-paper-ops/paper-state";
const LOCAL_INSTALLATION_ID_PATH: &str = "/run/owner-beta-paper-ops/paper-state/installation-id";
const LOCAL_EPOCH_FLOOR_PATH: &str = "/run/owner-beta-paper-ops/paper-state/epoch-floor";
const OPERATION_INTENT_PATH: &str = "/run/owner-beta-paper-ops/operation/intent.json";
const OPERATION_STAGE0_PATH: &str = "/run/owner-beta-paper-ops/operation/00000000.json";
const OPERATION_ACTIONS_PATH: &str = "/run/owner-beta-paper-ops/operation/actions";
const OPERATION_ACTION0_PATH: &str = "/run/owner-beta-paper-ops/operation/actions/00000000";
const OPERATION_BINDING_PATH: &str =
    "/run/owner-beta-paper-ops/operation/actions/00000000/binding.json";
const OPERATION_OUTPUT_PATH: &str = "/run/owner-beta-paper-ops/operation/actions/00000000/output";
const OPERATION_IMPORT_PATH: &str =
    "/run/owner-beta-paper-ops/operation/actions/00000000/import.json";

const STAGE_KEYS: &[&str] = &[
    "schema",
    "installation_id",
    "generation",
    "account_id",
    "sequence",
    "previous_sha256",
    "intent_sha256",
    "stage",
    "boot_id",
    "observed",
    "control_epoch",
    "artifact_sha256",
    "event_id",
    "error_code",
];

struct RetainedBytes {
    file: GuardedFile,
    bytes: Vec<u8>,
}

struct RetainedPaperState {
    directory: GuardedDirectory,
    installation_id: RetainedBytes,
    epoch_floor: RetainedBytes,
}

impl RetainedPaperState {
    fn open(
        directory_path: &'static str,
        installation_id_path: &'static str,
        epoch_floor_path: &'static str,
        expected_installation_id: uuid::Uuid,
        expected_epoch_floor: i64,
    ) -> Result<Self, ErrorCode> {
        let directory =
            GuardedDirectory::open(directory_path).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let (installation_id_file, installation_id_bytes) =
            read_private_state_file(installation_id_path, 64)?;
        let (epoch_floor_file, epoch_floor_bytes) = read_private_state_file(epoch_floor_path, 32)?;
        let mut state = Self {
            directory,
            installation_id: RetainedBytes {
                file: installation_id_file,
                bytes: installation_id_bytes,
            },
            epoch_floor: RetainedBytes {
                file: epoch_floor_file,
                bytes: epoch_floor_bytes,
            },
        };
        if parse_epoch_floor(&state.epoch_floor.bytes)? != expected_epoch_floor {
            return Err(ErrorCode::StaleEpoch);
        }
        state.revalidate(expected_installation_id, expected_epoch_floor)?;
        Ok(state)
    }

    fn revalidate(
        &mut self,
        expected_installation_id: uuid::Uuid,
        expected_epoch_floor: i64,
    ) -> Result<(), ErrorCode> {
        self.directory
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.installation_id.revalidate()?;
        self.epoch_floor.revalidate()?;
        if !self
            .installation_id
            .file
            .parent_matches(&self.directory)
            .map_err(|_| ErrorCode::ReaderIncompatible)?
            || !self
                .epoch_floor
                .file
                .parent_matches(&self.directory)
                .map_err(|_| ErrorCode::ReaderIncompatible)?
        {
            return Err(ErrorCode::IdentityConflict);
        }
        validate_installation_id(&self.installation_id.bytes, expected_installation_id)?;
        if parse_epoch_floor(&self.epoch_floor.bytes)? != expected_epoch_floor {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
}

struct RetainedObject {
    file: std::fs::File,
    identity: Identity,
    directory: GuardedDirectory,
    leaf: String,
    bytes: Vec<u8>,
}

impl RetainedObject {
    fn revalidate(&self) -> Result<(), ErrorCode> {
        self.directory
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if check_file(&self.file).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity
            || read_file_at(&self.file, self.bytes.len())? != self.bytes
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let named = openat_file(
            self.directory.directory.as_raw_fd(),
            &self.leaf,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if check_file(&named).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
}

impl RetainedBytes {
    fn revalidate(&mut self) -> Result<(), ErrorCode> {
        self.file
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let current = self
            .file
            .read_bounded(self.bytes.len())
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if current != self.bytes {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
}

struct MaintenancePins {
    capability: RetainedBytes,
    api_elf: GuardedFile,
    worker_elf: GuardedFile,
    operations_schema: GuardedFile,
    archive_schema: GuardedFile,
    migration: GuardedFile,
    audit_sql: GuardedFile,
    executable: GuardedExecutable,
    provenance: super::ProducerProvenance,
}

impl MaintenancePins {
    fn open(intent: &Intent) -> Result<Self, ErrorCode> {
        let (mut capability, capability_bytes) = read_fixed(CAPABILITY_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let capability_value =
            wire::parse_capability(&capability_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        let mut api_elf =
            GuardedFile::open(API_IMAGE_ELF_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut worker_elf =
            GuardedFile::open(WORKER_IMAGE_ELF_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut operations_schema = GuardedFile::open(OPERATIONS_SCHEMA_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut archive_schema =
            GuardedFile::open(ARCHIVE_SCHEMA_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut migration =
            GuardedFile::open(MIGRATION_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mut audit_sql =
            GuardedFile::open(AUDIT_SQL_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let executable = GuardedExecutable::open().map_err(|_| ErrorCode::ReaderIncompatible)?;
        if !is_elf(&mut api_elf.file).map_err(|_| ErrorCode::ReaderIncompatible)?
            || !is_elf(&mut worker_elf.file).map_err(|_| ErrorCode::ReaderIncompatible)?
        {
            return Err(ErrorCode::ReaderIncompatible);
        }
        let source_matches = operations_schema
            .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
            .is_ok_and(|value| value == capability_value["operations_schema_sha256"])
            && archive_schema
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["archive_schema_sha256"])
            && migration
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["migration_0068_sha256"])
            && audit_sql
                .digest_bounded(MAX_SOURCE_FILE_BYTES as u64)
                .is_ok_and(|value| value == capability_value["audit_sql_sha256"]);
        let api_sha256 = api_elf
            .digest_bounded(MAX_ELF_BYTES)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let worker_sha256 = worker_elf
            .digest_bounded(MAX_ELF_BYTES)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let compiled_commit =
            option_env!("LAGRANGE_CODE_COMMIT").ok_or(ErrorCode::EvidenceUnavailable)?;
        if !source_matches
            || capability_value["release_manifest_sha256"] != intent.release_manifest_sha256
            || capability_value["code_commit"] != compiled_commit
            || capability_value["native_binary_sha256"] != executable.sha256
            || capability_value["api_binary_sha256"] != api_sha256
            || capability_value["worker_binary_sha256"] != worker_sha256
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let provenance = super::ProducerProvenance::open(
            compiled_commit,
            &api_sha256,
            &worker_sha256,
            &executable.sha256,
        )?;
        capability
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        Ok(Self {
            capability: RetainedBytes {
                file: capability,
                bytes: capability_bytes,
            },
            api_elf,
            worker_elf,
            operations_schema,
            archive_schema,
            migration,
            audit_sql,
            executable,
            provenance,
        })
    }

    fn revalidate(&mut self) -> Result<(), ErrorCode> {
        self.capability.revalidate()?;
        for file in [
            &self.api_elf,
            &self.worker_elf,
            &self.operations_schema,
            &self.archive_schema,
            &self.migration,
            &self.audit_sql,
        ] {
            file.revalidate()
                .map_err(|_| ErrorCode::ReaderIncompatible)?;
        }
        self.executable
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.provenance.revalidate()
    }
}

/// Retained source files for the sequence-zero filesystem chain. This does not
/// carry image/source pins and cannot reach BEGIN's SQL call.
pub(super) struct RetainedFilesystemChain {
    operation: GuardedDirectory,
    actions: GuardedDirectory,
    action_dir: GuardedDirectory,
    action_output: GuardedDirectory,
    paper_state: RetainedPaperState,
    mounted_output_identity: Identity,
    intent: RetainedBytes,
    stage: RetainedBytes,
    stage_object: RetainedObject,
    binding: RetainedBytes,
    result_object: Option<RetainedPublishedOutput>,
    imported_dispatch: Option<(RetainedBytes, RetainedBytes)>,
    root_sha256: String,
    action_sha256: String,
    stage_sha256: String,
}

impl RetainedFilesystemChain {
    pub(super) fn open(
        originals: &BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<Self, ErrorCode> {
        let root = &originals.root;
        let action = originals.action();
        validate_begin_pair(root, action)?;
        let paper_state = RetainedPaperState::open(
            PAPER_STATE_PATH,
            LOCAL_INSTALLATION_ID_PATH,
            LOCAL_EPOCH_FLOOR_PATH,
            root.intent.installation_id,
            root.intent.local_epoch_floor,
        )?;

        let operation =
            GuardedDirectory::open(OPERATION_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let (intent_file, intent_bytes) = read_fixed(OPERATION_INTENT_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if intent_bytes != root.canonical.as_bytes() {
            return Err(ErrorCode::IdentityConflict);
        }
        let root_sha256 = digest(&intent_bytes);
        let (stage_file, stage_bytes) = read_fixed(OPERATION_STAGE0_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let stage_value = parse_intent_stage(&stage_bytes, root, &root_sha256)?;
        let stage_sha256 = digest(&stage_bytes);
        if action.stage_sha256.as_deref() != Some(stage_sha256.as_str()) {
            return Err(ErrorCode::IdentityConflict);
        }
        require_current_boot(&stage_value)?;
        let stage_object_path = format!("{}.json", &stage_sha256[7..]);
        let stage_object = if stage_object_path.len() == 69 {
            // The current generation is mounted at this fixed directory. Its
            // only content object for sequence-zero BEGIN is the INTENT stage.
            open_content_object(OPERATION_PATH, &stage_object_path, &stage_bytes)?
        } else {
            return Err(ErrorCode::WireInvalid);
        };
        let operation_names = names(&operation)?;
        let expected_root_names = BTreeSet::from([
            "intent.json".to_owned(),
            "00000000.json".to_owned(),
            "actions".to_owned(),
            stage_object_path,
        ]);
        require_exact_names(operation_names, expected_root_names)?;

        let actions = GuardedDirectory::open(OPERATION_ACTIONS_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        require_exact_names(names(&actions)?, BTreeSet::from(["00000000".to_owned()]))?;
        let action_dir = GuardedDirectory::open(OPERATION_ACTION0_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let (binding_file, binding_bytes) =
            read_fixed(OPERATION_BINDING_PATH, wire::MAX_WIRE_BYTES)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if binding_bytes != originals.action_bytes() {
            return Err(ErrorCode::IdentityConflict);
        }
        let action_names = names(&action_dir)?;
        let expected_action_names = if entry_exists(&action_dir, "import.json")? {
            BTreeSet::from(["binding.json".into(), "import.json".into(), "output".into()])
        } else {
            BTreeSet::from(["binding.json".into(), "output".into()])
        };
        if entry_exists(&action_dir, "evidence.json")? || originals.artifact_bytes().is_some() {
            return Err(ErrorCode::IdentityConflict);
        }
        require_exact_names(action_names, expected_action_names)?;
        let action_output = GuardedDirectory::open(OPERATION_OUTPUT_PATH)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let mounted_output =
            GuardedDirectory::open(OUTPUT_PATH).map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if !same_object(action_output.identity, mounted_output.identity) {
            return Err(ErrorCode::IdentityConflict);
        }
        validate_output_tree(&action_output)?;
        let imported_dispatch = if entry_exists(&action_dir, "import.json")? {
            let (import, import_bytes) =
                read_fixed(OPERATION_IMPORT_PATH, super::bound::MAX_DISPATCH_BYTES)
                    .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            let (dispatch, dispatch_bytes) = read_fixed(
                "/run/owner-beta-paper-ops/operation/actions/00000000/output/dispatch.json",
                super::bound::MAX_DISPATCH_BYTES,
            )
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            if import_bytes != dispatch_bytes {
                return Err(ErrorCode::IdentityConflict);
            }
            Some((
                RetainedBytes {
                    file: import,
                    bytes: import_bytes,
                },
                RetainedBytes {
                    file: dispatch,
                    bytes: dispatch_bytes,
                },
            ))
        } else {
            None
        };

        let named_intent = RetainedBytes {
            file: intent_file,
            bytes: intent_bytes,
        };
        let stage = RetainedBytes {
            file: stage_file,
            bytes: stage_bytes,
        };
        let binding = RetainedBytes {
            file: binding_file,
            bytes: binding_bytes,
        };
        let mut live = Self {
            operation,
            actions,
            action_dir,
            action_output,
            paper_state,
            mounted_output_identity: mounted_output.identity,
            intent: named_intent,
            stage,
            stage_object,
            binding,
            result_object: None,
            imported_dispatch,
            root_sha256,
            action_sha256: action.sha256.clone(),
            stage_sha256,
        };
        live.revalidate(originals, inputs)?;
        Ok(live)
    }

    pub(super) fn revalidate(
        &mut self,
        originals: &BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<(), ErrorCode> {
        originals.revalidate_originals(inputs)?;
        self.operation
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.actions
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.action_dir
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        self.action_output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if self.action_output.identity != self.mounted_output_identity {
            return Err(ErrorCode::IdentityConflict);
        }
        self.intent.revalidate()?;
        self.stage.revalidate()?;
        self.stage_object.revalidate()?;
        self.binding.revalidate()?;
        match &mut self.imported_dispatch {
            Some((import, dispatch)) => {
                import.revalidate()?;
                dispatch.revalidate()?;
                if import.bytes != dispatch.bytes {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
            None if entry_exists(&self.action_dir, "import.json")? => {
                return Err(ErrorCode::IdentityConflict);
            }
            None => {}
        }
        let root = &originals.root;
        let action = originals.action();
        self.paper_state
            .revalidate(root.intent.installation_id, root.intent.local_epoch_floor)?;
        if root.intent.local_epoch_floor != root.intent.expected_epoch
            || digest(&self.intent.bytes) != self.root_sha256
            || digest(&self.stage.bytes) != self.stage_sha256
            || digest(&self.binding.bytes) != self.action_sha256
            || self.intent.bytes != root.canonical.as_bytes()
            || self.binding.bytes != originals.action_bytes()
            || action.stage_sha256.as_deref() != Some(self.stage_sha256.as_str())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        require_current_boot(&parse_intent_stage(
            &self.stage.bytes,
            root,
            &self.root_sha256,
        )?)?;
        validate_operation_names(&self.operation, &self.stage_sha256)?;
        validate_action_names(&self.actions, &self.action_dir, &self.action_output)?;
        validate_output_tree(&self.action_output)?;
        if let Some(result) = &self.result_object {
            result
                .revalidate(&self.action_output)
                .map_err(|_| ErrorCode::CommitUnknown)?;
        }
        Ok(())
    }

    pub(super) fn retain_result(
        &mut self,
        result: RetainedPublishedOutput,
    ) -> Result<(), ErrorCode> {
        if self.result_object.is_some() {
            return Err(ErrorCode::IdentityConflict);
        }
        self.result_object = Some(result);
        self.result_object
            .as_ref()
            .ok_or(ErrorCode::CommitUnknown)?
            .revalidate(&self.action_output)
            .map_err(|_| ErrorCode::CommitUnknown)?;
        Ok(())
    }
}

/// The only type that can authorize the maintenance BEGIN call. It combines
/// retained originals with the actual selected-image and producer pins.
pub(super) struct LiveBeginInputs {
    chain: RetainedFilesystemChain,
    pins: MaintenancePins,
}

impl LiveBeginInputs {
    pub(super) fn open(
        originals: &BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<Self, ErrorCode> {
        let chain = RetainedFilesystemChain::open(originals, inputs)?;
        let pins = MaintenancePins::open(&originals.root.intent)?;
        let mut live = Self { chain, pins };
        live.revalidate(originals, inputs)?;
        Ok(live)
    }

    pub(super) fn revalidate(
        &mut self,
        originals: &BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<(), ErrorCode> {
        self.chain.revalidate(originals, inputs)?;
        self.pins.revalidate()
    }

    pub(super) fn retain_result(
        &mut self,
        result: RetainedPublishedOutput,
    ) -> Result<(), ErrorCode> {
        self.chain.retain_result(result)
    }
}

fn validate_begin_pair(
    root: &RootIntent,
    action: &super::wire::ActionBinding,
) -> Result<(), ErrorCode> {
    if !matches!(
        (root.intent.purpose.as_str(), root.intent.scope.as_str()),
        ("STOP", "PAPER_STOP") | ("BACKUP", "COMBINED_BACKUP")
    ) || root.intent.account_id.is_none()
        || root.intent.expected_epoch != root.intent.local_epoch_floor
        || root.intent.request_key != format!("{}:begin", root.intent.generation)
        || action.verb != Verb::Begin
        || action.sequence != 0
        || action.request_key != root.intent.request_key
        || action.expected_epoch != root.intent.expected_epoch
        || action.artifact_sha256.is_some()
    {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    Ok(())
}

fn parse_intent_stage(
    bytes: &[u8],
    root: &RootIntent,
    root_sha256: &str,
) -> Result<Value, ErrorCode> {
    let value = wire::parse_canonical(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    if !super::exact_keys(&value, STAGE_KEYS)
        || value["schema"] != "owner-beta-paper-operation-stage-v1"
        || value["installation_id"] != root.intent.installation_id.to_string()
        || value["generation"] != root.intent.generation.to_string()
        || value["account_id"]
            != root
                .intent
                .account_id
                .map(|id| id.to_string())
                .unwrap_or_default()
        || value["sequence"] != 0
        || !value["previous_sha256"].is_null()
        || value["intent_sha256"] != root_sha256
        || value["stage"] != "INTENT"
        || value["control_epoch"] != root.intent.expected_epoch
        || !value["artifact_sha256"].is_null()
        || !value["event_id"].is_null()
        || value["error_code"] != "NONE"
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let boot = value["boot_id"].as_str().ok_or(ErrorCode::WireInvalid)?;
    let parsed_boot = uuid::Uuid::parse_str(boot).map_err(|_| ErrorCode::WireInvalid)?;
    if parsed_boot.is_nil() || parsed_boot.to_string() != boot {
        return Err(ErrorCode::WireInvalid);
    }
    if !super::exact_keys(
        &value["observed"],
        &["realtime_ns", "monotonic_ns", "boottime_ns"],
    ) || ["realtime_ns", "monotonic_ns", "boottime_ns"]
        .iter()
        .any(|key| !value["observed"][*key].as_i64().is_some_and(|n| n >= 0))
    {
        return Err(ErrorCode::WireInvalid);
    }
    Ok(value)
}

fn require_current_boot(stage: &Value) -> Result<(), ErrorCode> {
    let boot = stage["boot_id"].as_str().ok_or(ErrorCode::WireInvalid)?;
    let observed = uuid::Uuid::parse_str(boot).map_err(|_| ErrorCode::WireInvalid)?;
    if observed != local_boot()? {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

fn names(directory: &GuardedDirectory) -> Result<BTreeSet<String>, ErrorCode> {
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let entries = std::fs::read_dir(format!("/proc/self/fd/{}", directory.directory.as_raw_fd()))
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let mut names = BTreeSet::new();
    for entry in entries {
        let name = entry
            .map_err(|_| ErrorCode::EvidenceUnavailable)?
            .file_name()
            .into_string()
            .map_err(|_| ErrorCode::WireInvalid)?;
        if !names.insert(name) {
            return Err(ErrorCode::IdentityConflict);
        }
    }
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(names)
}

fn require_exact_names(
    actual: BTreeSet<String>,
    expected: BTreeSet<String>,
) -> Result<(), ErrorCode> {
    if actual == expected {
        Ok(())
    } else {
        Err(ErrorCode::IdentityConflict)
    }
}

fn entry_exists(directory: &GuardedDirectory, leaf: &str) -> Result<bool, ErrorCode> {
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let name = CString::new(leaf).map_err(|_| ErrorCode::WireInvalid)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.directory.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(true)
    } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        Ok(false)
    } else {
        Err(ErrorCode::ReaderIncompatible)
    }
}

fn open_content_object(
    directory_path: &'static str,
    leaf: &str,
    expected: &[u8],
) -> Result<RetainedObject, ErrorCode> {
    if leaf.len() != 69
        || !leaf.ends_with(".json")
        || !leaf[..64]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || digest(expected) != format!("sha256:{}", &leaf[..64])
    {
        return Err(ErrorCode::WireInvalid);
    }
    let dir = GuardedDirectory::open(directory_path).map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let file = openat_file(
        dir.directory.as_raw_fd(),
        leaf,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
    )
    .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let id = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
    if id.size != expected.len() as u64 || read_file_at(&file, expected.len())? != expected {
        return Err(ErrorCode::IdentityConflict);
    }
    dir.revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let named = openat_file(
        dir.directory.as_raw_fd(),
        leaf,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
    )
    .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    if check_file(&named).map_err(|_| ErrorCode::ReaderIncompatible)? != id {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(RetainedObject {
        file,
        identity: id,
        directory: dir,
        leaf: leaf.to_owned(),
        bytes: expected.to_vec(),
    })
}

fn read_file_at(file: &std::fs::File, len: usize) -> Result<Vec<u8>, ErrorCode> {
    let mut bytes = vec![0; len];
    file.read_exact_at(&mut bytes, 0)
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(bytes)
}

fn read_private_state_file(
    path: &'static str,
    limit: usize,
) -> Result<(GuardedFile, Vec<u8>), ErrorCode> {
    let mut file =
        GuardedFile::open_mode(path, 0o600).map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let bytes = file
        .read_bounded(limit)
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    file.revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok((file, bytes))
}

fn parse_installation_id(bytes: &[u8]) -> Result<uuid::Uuid, ErrorCode> {
    let text = std::str::from_utf8(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    let value = text.strip_suffix('\n').ok_or(ErrorCode::WireInvalid)?;
    let installation_id = uuid::Uuid::parse_str(value).map_err(|_| ErrorCode::WireInvalid)?;
    if installation_id.is_nil() || installation_id.to_string() != value {
        return Err(ErrorCode::WireInvalid);
    }
    Ok(installation_id)
}

fn validate_installation_id(bytes: &[u8], expected: uuid::Uuid) -> Result<(), ErrorCode> {
    if parse_installation_id(bytes)? == expected {
        Ok(())
    } else {
        Err(ErrorCode::IdentityConflict)
    }
}

fn parse_epoch_floor(bytes: &[u8]) -> Result<i64, ErrorCode> {
    let text = std::str::from_utf8(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    let digits = text.strip_suffix('\n').ok_or(ErrorCode::WireInvalid)?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ErrorCode::WireInvalid);
    }
    let floor = digits.parse::<i64>().map_err(|_| ErrorCode::WireInvalid)?;
    if floor < 0 || format!("{floor}\n") != text {
        return Err(ErrorCode::WireInvalid);
    }
    Ok(floor)
}

fn validate_operation_names(
    operation: &GuardedDirectory,
    stage_sha256: &str,
) -> Result<(), ErrorCode> {
    let names = names(operation)?;
    let object_name = format!("{}.json", &stage_sha256[7..]);
    require_exact_names(
        names,
        BTreeSet::from([
            "intent.json".to_owned(),
            "00000000.json".to_owned(),
            "actions".to_owned(),
            object_name,
        ]),
    )
}

fn validate_action_names(
    actions: &GuardedDirectory,
    action_dir: &GuardedDirectory,
    output: &GuardedDirectory,
) -> Result<(), ErrorCode> {
    require_exact_names(names(actions)?, BTreeSet::from(["00000000".to_owned()]))?;
    let expected = if entry_exists(action_dir, "import.json")? {
        BTreeSet::from(["binding.json".into(), "import.json".into(), "output".into()])
    } else {
        BTreeSet::from(["binding.json".into(), "output".into()])
    };
    if entry_exists(action_dir, "evidence.json")? {
        return Err(ErrorCode::IdentityConflict);
    }
    require_exact_names(names(action_dir)?, expected)?;
    names(output).map(|_| ())
}

fn validate_output_tree(output: &GuardedDirectory) -> Result<(), ErrorCode> {
    let names = names(output)?;
    for name in &names {
        if name == "dispatch.json" {
            let file = openat_file(
                output.directory.as_raw_fd(),
                name,
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
            check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
            continue;
        }
        if name.len() != 69
            || !name.ends_with(".json")
            || !name[..64]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut file = openat_file(
            output.directory.as_raw_fd(),
            name,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let id = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(super::bound::MAX_DISPATCH_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        if bytes.len() as u64 != id.size || digest(&bytes) != format!("sha256:{}", &name[..64]) {
            return Err(ErrorCode::IdentityConflict);
        }
    }
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)
}

#[cfg(test)]
mod tests;

fn local_boot() -> Result<uuid::Uuid, ErrorCode> {
    let text = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| ErrorCode::ClockInvalid)?;
    let text = text.trim_end_matches('\n');
    let boot = uuid::Uuid::parse_str(text).map_err(|_| ErrorCode::ClockInvalid)?;
    if boot.is_nil() || boot.to_string() != text {
        return Err(ErrorCode::ClockInvalid);
    }
    Ok(boot)
}
