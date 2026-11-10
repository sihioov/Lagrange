//! Retained, fixed-path readers for live maintenance actions.
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
        let (capability, capability_bytes) = read_fixed(CAPABILITY_PATH, wire::MAX_WIRE_BYTES)
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
#[derive(Clone, Copy)]
enum ChainUse { Maintenance, ArchiveReadOnly }

pub(super) struct RetainedFilesystemChain {
    use_kind: ChainUse,
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
        validate_begin_pair(&originals.root, originals.action())?;
        Self::open_validated(originals, inputs, ChainUse::Maintenance)
    }

    fn open_archive(originals: &BoundOriginals, inputs: &ProtectedInputs) -> Result<Self, ErrorCode> {
        validate_archive_begin_pair(&originals.root, originals.action())?;
        Self::open_validated(originals, inputs, ChainUse::ArchiveReadOnly)
    }

    fn open_validated(originals: &BoundOriginals, inputs: &ProtectedInputs, use_kind: ChainUse)
        -> Result<Self, ErrorCode> {
        let root = &originals.root;
        let action = originals.action();
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
            use_kind,
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
        match self.use_kind {
            ChainUse::Maintenance => validate_begin_pair(root, action)?,
            ChainUse::ArchiveReadOnly => validate_archive_begin_pair(root, action)?,
        }
        self.paper_state
            .revalidate(root.intent.installation_id, root.intent.local_epoch_floor)?;
        // RESTORE's adopted root may name an archived epoch below the retained
        // destination floor. Actual local floor custody above remains required;
        // ordinary maintenance still requires equality. This grants no fence.
        if (matches!(self.use_kind, ChainUse::Maintenance)
            && root.intent.local_epoch_floor != root.intent.expected_epoch)
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

/// This holder only permits non-serving reads before any destination SQL
/// BEGIN/fence. It cannot be passed to begin_bound or maintenance_bound.
pub(super) struct ArchiveIntakeInputs {
    chain: RetainedFilesystemChain,
    pins: MaintenancePins,
}
impl ArchiveIntakeInputs {
    pub(super) fn open(originals: &BoundOriginals, inputs: &ProtectedInputs) -> Result<Self, ErrorCode> {
        let chain = RetainedFilesystemChain::open_archive(originals, inputs)?;
        let pins = MaintenancePins::open(&originals.root.intent)?;
        let mut owner = Self { chain, pins };
        owner.revalidate(originals, inputs)?;
        Ok(owner)
    }
    pub(super) fn revalidate(&mut self, originals: &BoundOriginals, inputs: &ProtectedInputs)
        -> Result<(), ErrorCode> {
        let chain = self.chain.revalidate(originals, inputs);
        let pins = self.pins.revalidate();
        chain.and(pins)
    }
}
fn validate_archive_begin_pair(root: &RootIntent, action: &super::wire::ActionBinding) -> Result<(), ErrorCode> {
    let r = &root.intent;
    if r.purpose != "RESTORE" || r.scope != "COMBINED_BACKUP" || r.account_id.is_none()
        || r.artifact_sha256.is_none() || r.value["predecessor_generation"].is_null()
        || r.request_key != format!("{}:begin", r.generation)
        || action.verb != Verb::Begin || action.sequence != 0
        || action.previous_binding_sha256.is_some() || action.artifact_sha256.is_some()
        || action.request_key != r.request_key || action.expected_epoch != r.expected_epoch {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    Ok(())
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

pub(super) fn archive_package_names(directory: &GuardedDirectory) -> Result<(), ErrorCode> {
    directory.revalidate().map_err(|_| ErrorCode::ReaderIncompatible)?;
    let entries = std::fs::read_dir(format!("/proc/self/fd/{}", directory.directory.as_raw_fd()))
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let mut found = BTreeSet::new();
    for entry in entries {
        let name = entry.map_err(|_| ErrorCode::EvidenceUnavailable)?.file_name()
            .into_string().map_err(|_| ErrorCode::WireInvalid)?;
        if !matches!(name.as_str(), "files.bundle" | "audit.stream" | "files" | "coordination")
            || !found.insert(name) || found.len() > 4 {
            return Err(ErrorCode::IdentityConflict);
        }
    }
    directory.revalidate().map_err(|_| ErrorCode::ReaderIncompatible)?;
    require_exact_names(found, BTreeSet::from([
        "files.bundle".into(), "audit.stream".into(), "files".into(), "coordination".into(),
    ]))
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

// Relative names below are generated from canonical counters/digests after the
// fixed operation mount has been opened. No wire string is an arbitrary path.
struct RelativeOriginal {
    components: Vec<String>,
    parents: Vec<(File, Identity)>,
    file: File,
    identity: Identity,
    bytes: Vec<u8>,
}

fn open_relative_parent(
    root: &GuardedDirectory,
    components: &[String],
) -> Result<Vec<(File, Identity)>, ErrorCode> {
    root.revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let first = root
        .directory
        .try_clone()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let mut parents = vec![(first, root.identity)];
    for component in components {
        if component.is_empty()
            || component.contains('/')
            || matches!(component.as_str(), "." | "..")
        {
            return Err(ErrorCode::WireInvalid);
        }
        let parent = parents.last().ok_or(ErrorCode::InternalFailure)?;
        let file = openat_file(
            parent.0.as_raw_fd(),
            component,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let id = check_dir(&file, true).map_err(|_| ErrorCode::ReaderIncompatible)?;
        parents.push((file, id));
    }
    Ok(parents)
}

impl RelativeOriginal {
    fn open(
        root: &GuardedDirectory,
        components: Vec<String>,
        limit: usize,
    ) -> Result<Self, ErrorCode> {
        let (leaf, path) = components.split_last().ok_or(ErrorCode::WireInvalid)?;
        let parents = open_relative_parent(root, path)?;
        let parent = parents.last().ok_or(ErrorCode::InternalFailure)?;
        let file = openat_file(
            parent.0.as_raw_fd(),
            leaf,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let id = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
        let size = usize::try_from(id.size).map_err(|_| ErrorCode::WireInvalid)?;
        if size > limit {
            return Err(ErrorCode::WireInvalid);
        }
        let bytes = read_file_at(&file, size)?;
        let original = Self {
            components,
            parents,
            file,
            identity: id,
            bytes,
        };
        original.revalidate(root)?;
        Ok(original)
    }

    fn revalidate(&self, root: &GuardedDirectory) -> Result<(), ErrorCode> {
        let (leaf, path) = self.components.split_last().ok_or(ErrorCode::WireInvalid)?;
        let now = open_relative_parent(root, path)?;
        if now.len() != self.parents.len() {
            return Err(ErrorCode::IdentityConflict);
        }
        for ((held, id), (named, named_id)) in self.parents.iter().zip(&now) {
            if !same_object(
                check_dir(held, true).map_err(|_| ErrorCode::ReaderIncompatible)?,
                *id,
            ) || !same_object(*named_id, *id)
                || !same_object(
                    check_dir(named, true).map_err(|_| ErrorCode::ReaderIncompatible)?,
                    *id,
                )
            {
                return Err(ErrorCode::IdentityConflict);
            }
        }
        let named = openat_file(
            now.last().ok_or(ErrorCode::InternalFailure)?.0.as_raw_fd(),
            leaf,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        if check_file(&self.file).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity
            || check_file(&named).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity
            || read_file_at(&self.file, self.bytes.len())? != self.bytes
        {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }
}

struct MaintenanceTree {
    root: GuardedDirectory,
    files: std::collections::BTreeMap<Vec<String>, RelativeOriginal>,
    directories: std::collections::BTreeMap<Vec<String>, BTreeSet<String>>,
}

impl MaintenanceTree {
    fn open() -> Result<Self, ErrorCode> {
        Ok(Self {
            root: GuardedDirectory::open(OPERATION_PATH)
                .map_err(|_| ErrorCode::EvidenceUnavailable)?,
            files: std::collections::BTreeMap::new(),
            directories: std::collections::BTreeMap::new(),
        })
    }

    fn read(&mut self, path: Vec<String>, limit: usize) -> Result<Vec<u8>, ErrorCode> {
        if !self.files.contains_key(&path) {
            let original = RelativeOriginal::open(&self.root, path.clone(), limit)?;
            self.files.insert(path.clone(), original);
        }
        let original = self
            .files
            .get(&path)
            .ok_or(ErrorCode::EvidenceUnavailable)?;
        original.revalidate(&self.root)?;
        if original.bytes.len() > limit {
            return Err(ErrorCode::WireInvalid);
        }
        Ok(original.bytes.clone())
    }

    fn names(&mut self, path: Vec<String>) -> Result<BTreeSet<String>, ErrorCode> {
        let parents = open_relative_parent(&self.root, &path)?;
        let directory = &parents.last().ok_or(ErrorCode::InternalFailure)?.0;
        let result = relative_names(directory)?;
        if let Some(before) = self.directories.get(&path) {
            require_exact_names(result.clone(), before.clone())?;
        } else {
            self.directories.insert(path, result.clone());
        }
        Ok(result)
    }

    fn object(&mut self, hash: &str) -> Result<Value, ErrorCode> {
        let leaf = digest_leaf(hash)?;
        let bytes = self.read(vec![leaf], bound::MAX_DISPATCH_BYTES)?;
        if digest(&bytes) != hash {
            return Err(ErrorCode::IdentityConflict);
        }
        wire::parse_canonical_limit(&bytes, bound::MAX_DISPATCH_BYTES)
            .map_err(|_| ErrorCode::WireInvalid)
    }

    fn output_original(&mut self, sequence: i64, expected: &[u8]) -> Result<(), ErrorCode> {
        let bytes = self.read(
            vec![
                "actions".into(),
                format!("{sequence:08}"),
                "output".into(),
                digest_leaf(&digest(expected))?,
            ],
            bound::MAX_DISPATCH_BYTES,
        )?;
        if bytes != expected {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(())
    }

    fn revalidate(&mut self) -> Result<(), ErrorCode> {
        self.root
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        for file in self.files.values() {
            file.revalidate(&self.root)?;
        }
        let paths: Vec<_> = self.directories.keys().cloned().collect();
        for path in paths {
            self.names(path)?;
        }
        Ok(())
    }
}

fn relative_names(directory: &File) -> Result<BTreeSet<String>, ErrorCode> {
    check_dir(directory, true).map_err(|_| ErrorCode::ReaderIncompatible)?;
    let mut result = BTreeSet::new();
    for entry in std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))
        .map_err(|_| ErrorCode::EvidenceUnavailable)?
    {
        let name = entry
            .map_err(|_| ErrorCode::EvidenceUnavailable)?
            .file_name()
            .into_string()
            .map_err(|_| ErrorCode::WireInvalid)?;
        if !result.insert(name) {
            return Err(ErrorCode::IdentityConflict);
        }
    }
    Ok(result)
}

fn digest_leaf(hash: &str) -> Result<String, ErrorCode> {
    if hash.len() != 71
        || !hash.starts_with("sha256:")
        || !hash.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(ErrorCode::WireInvalid);
    }
    Ok(format!("{}.json", &hash[7..]))
}

fn field_text<'a>(value: &'a Value, key: &str) -> Result<&'a str, ErrorCode> {
    value[key].as_str().ok_or(ErrorCode::WireInvalid)
}

fn stage_after(
    bytes: &[u8],
    root: &RootIntent,
    sequence: i64,
    previous: &Value,
    previous_hash: &str,
) -> Result<Value, ErrorCode> {
    let value = wire::parse_canonical(bytes).map_err(|_| ErrorCode::WireInvalid)?;
    let fence = root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?;
    let name = match sequence {
        1 => "FENCED",
        2 => "STOPPED",
        _ if sequence >= 3 => "RECONCILED",
        _ => return Err(ErrorCode::WireInvalid),
    };
    if !exact_keys(&value, STAGE_KEYS)
        || value["schema"] != "owner-beta-paper-operation-stage-v1"
        || value["installation_id"] != root.intent.installation_id.to_string()
        || value["generation"] != root.intent.generation.to_string()
        || value["account_id"] != root.intent.value["account_id"]
        || value["sequence"] != sequence
        || value["previous_sha256"] != previous_hash
        || value["intent_sha256"] != root.sha256
        || value["stage"] != name
        || value["control_epoch"] != fence
        || value["error_code"] != "NONE"
        || value["boot_id"] != previous["boot_id"]
        || !canonical_uuid(&value["event_id"])
        || !sha256_wire(&value["artifact_sha256"])
        || (sequence == 2 && value["event_id"] != previous["event_id"])
        || !exact_keys(
            &value["observed"],
            &["realtime_ns", "monotonic_ns", "boottime_ns"],
        )
        || ["realtime_ns", "monotonic_ns", "boottime_ns"]
            .iter()
            .any(|key| {
                match (
                    value["observed"][*key].as_i64(),
                    previous["observed"][*key].as_i64(),
                ) {
                    (Some(now), Some(before)) => now < before || now < 0,
                    _ => true,
                }
            })
    {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(value)
}

fn maintenance_pair(
    root: &RootIntent,
    action: &wire::ActionBinding,
    last: &Value,
    last_hash: &str,
) -> Result<(), ErrorCode> {
    if !matches!(
        (root.intent.purpose.as_str(), root.intent.scope.as_str()),
        ("STOP", "PAPER_STOP") | ("BACKUP", "COMBINED_BACKUP")
    ) || !matches!(action.verb, Verb::Reconcile | Verb::Off)
        || action.sequence < 1
        || action.expected_epoch != root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?
        || action.stage_sha256.as_deref() != Some(last_hash)
        || last["sequence"].as_i64() != action.sequence.checked_add(1)
        || !matches!(last["stage"].as_str(), Some("STOPPED" | "RECONCILED"))
        || (action.verb == Verb::Off && last["stage"] != "RECONCILED")
    {
        return Err(ErrorCode::EvidenceUnavailable);
    }
    Ok(())
}

pub(super) struct PriorMaintenanceResult {
    pub(super) request: ExactRequest,
    pub(super) canonical: String,
    pub(super) event_id: String,
    pub(super) epoch: i64,
}

impl PriorMaintenanceResult {
    pub(super) fn verify_readback(
        &self,
        readback: &ExactReadback,
        fence: i64,
    ) -> Result<(), ErrorCode> {
        let value = &readback.value;
        if value["status"] != "COMMITTED"
            || value["current_state"] != "DRAINING"
            || value["current_epoch"] != fence
            || value["event_epoch"] != self.epoch
            || value["event_id"] != self.event_id
            || value["result_canonical"] != self.canonical
            || value["result_sha256"] != digest(self.canonical.as_bytes())
        {
            return Err(ErrorCode::CommitUnknown);
        }
        Ok(())
    }
}

/// No public constructor/deserializer; only protected original files plus the
/// actual helper ELF/provenance can instantiate this consuming authority.
pub(super) struct LiveMaintenanceInputs {
    tree: MaintenanceTree,
    state: RetainedPaperState,
    pins: MaintenancePins,
    output: GuardedDirectory,
    output_path: Vec<String>,
    results: Vec<RetainedPublishedOutput>,
    pub(super) prior: Vec<PriorMaintenanceResult>,
    pub(super) captured_pids: Vec<i32>,
    pub(super) fence: i64,
    inventory: Value,
    evidence: Value,
}

impl LiveMaintenanceInputs {
    pub(super) fn open(
        originals: &BoundOriginals,
        inputs: &ProtectedInputs,
    ) -> Result<Self, ErrorCode> {
        let root = &originals.root;
        let action = originals.action();
        if !matches!(action.verb, Verb::Reconcile | Verb::Off)
            || !matches!(root.intent.purpose.as_str(), "STOP" | "BACKUP")
        {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        let fence = root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?;
        if action.verb == Verb::Off {
            fence.checked_add(1).ok_or(ErrorCode::StaleEpoch)?;
        }
        let state = RetainedPaperState::open(
            PAPER_STATE_PATH,
            LOCAL_INSTALLATION_ID_PATH,
            LOCAL_EPOCH_FLOOR_PATH,
            root.intent.installation_id,
            fence,
        )?;
        let mut tree = MaintenanceTree::open()?;
        let root_bytes = tree.read(vec!["intent.json".into()], wire::MAX_WIRE_BYTES)?;
        if root_bytes != root.canonical.as_bytes() {
            return Err(ErrorCode::IdentityConflict);
        }
        let top = tree.names(vec![])?;
        let mut stages: Vec<Value> = Vec::new();
        let mut stage_hashes: Vec<String> = Vec::new();
        let last_sequence = action
            .sequence
            .checked_add(1)
            .ok_or(ErrorCode::WireInvalid)?;
        // Count before iterating an 18-digit ordinal; a missing member never
        // causes an unbounded search or selection by mtime.
        let stage_names: BTreeSet<_> = top
            .iter()
            .filter(|name| {
                name.len() >= 13
                    && name.ends_with(".json")
                    && name[..name.len() - 5].bytes().all(|b| b.is_ascii_digit())
                    && name.len() != 69
            })
            .cloned()
            .collect();
        if usize::try_from(last_sequence.checked_add(1).ok_or(ErrorCode::WireInvalid)?).ok()
            != Some(stage_names.len())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        for sequence in 0..=last_sequence {
            let name = format!("{sequence:08}.json");
            if !stage_names.contains(&name) {
                return Err(ErrorCode::IdentityConflict);
            }
            let bytes = tree.read(vec![name], wire::MAX_WIRE_BYTES)?;
            let hash = digest(&bytes);
            let value = if sequence == 0 {
                parse_intent_stage(&bytes, root, &root.sha256)?
            } else {
                stage_after(
                    &bytes,
                    root,
                    sequence,
                    stages.last().ok_or(ErrorCode::WireInvalid)?,
                    stage_hashes.last().ok_or(ErrorCode::WireInvalid)?,
                )?
            };
            require_current_boot(&value)?;
            if tree.object(&hash)? != value {
                return Err(ErrorCode::IdentityConflict);
            }
            stages.push(value);
            stage_hashes.push(hash);
        }
        for name in &top {
            if name == "intent.json" || name == "actions" || stage_names.contains(name) {
                continue;
            }
            if name.len() != 69 || !name.ends_with(".json") {
                return Err(ErrorCode::IdentityConflict);
            }
            tree.object(&format!("sha256:{}", &name[..64]))?;
        }
        let last = stages.last().ok_or(ErrorCode::WireInvalid)?;
        maintenance_pair(
            root,
            action,
            last,
            stage_hashes.last().ok_or(ErrorCode::WireInvalid)?,
        )?;
        let action_names = tree.names(vec!["actions".into()])?;
        if usize::try_from(
            action
                .sequence
                .checked_add(1)
                .ok_or(ErrorCode::WireInvalid)?,
        )
        .ok()
            != Some(action_names.len())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut previous_binding = None;
        let mut prior = Vec::new();
        let mut selected_output = None;
        for sequence in 0..=action.sequence {
            let name = format!("{sequence:08}");
            if !action_names.contains(&name) {
                return Err(ErrorCode::IdentityConflict);
            }
            let base = vec!["actions".into(), name];
            let mut binding_path = base.clone();
            binding_path.push("binding.json".into());
            let binding_bytes = tree.read(binding_path, wire::MAX_WIRE_BYTES)?;
            let binding = wire::parse_action(&binding_bytes, root)
                .map_err(|_| ErrorCode::IdentityConflict)?;
            let expected_stage = if sequence == 0 {
                0
            } else {
                sequence.checked_add(1).ok_or(ErrorCode::WireInvalid)?
            };
            let stage_index =
                usize::try_from(expected_stage).map_err(|_| ErrorCode::WireInvalid)?;
            if binding.sequence != sequence
                || binding.previous_binding_sha256 != previous_binding
                || binding.stage_sha256.as_deref()
                    != stage_hashes.get(stage_index).map(String::as_str)
                || (sequence < action.sequence
                    && binding.verb
                        != if sequence == 0 {
                            Verb::Begin
                        } else {
                            Verb::Reconcile
                        })
            {
                return Err(ErrorCode::IdentityConflict);
            }
            previous_binding = Some(binding.sha256.clone());
            let names = tree.names(base.clone())?;
            let mut expected = BTreeSet::from(["binding.json".into(), "output".into()]);
            let evidence_bytes = if binding.artifact_sha256.is_some() {
                expected.insert("evidence.json".into());
                let mut path = base.clone();
                path.push("evidence.json".into());
                Some(tree.read(path, wire::MAX_WIRE_BYTES)?)
            } else {
                None
            };
            let artifact = evidence_bytes
                .as_ref()
                .map(|bytes| {
                    wire::parse_bound_artifact(bytes, root, &binding)
                        .map_err(|_| ErrorCode::WireInvalid)
                })
                .transpose()?;
            if binding.verb == Verb::Reconcile {
                let inventory = artifact.as_ref().ok_or(ErrorCode::EvidenceUnavailable)?;
                if inventory.value["fence_event_id"] != stages[1]["event_id"]
                    || inventory.value["boot_id"] != stages[1]["boot_id"]
                    || (sequence == 1 && stages[2]["artifact_sha256"] != inventory.sha256)
                {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
            let imported = names.contains("import.json");
            if imported {
                expected.insert("import.json".into());
            }
            require_exact_names(names, expected)?;
            let mut output_path = base.clone();
            output_path.push("output".into());
            let output_parents = open_relative_parent(&tree.root, &output_path)?;
            if sequence == action.sequence {
                if binding_bytes != originals.action_bytes()
                    || evidence_bytes.as_deref() != originals.artifact_bytes()
                {
                    return Err(ErrorCode::IdentityConflict);
                }
                let output = GuardedDirectory::open(OUTPUT_PATH)
                    .map_err(|_| ErrorCode::EvidenceUnavailable)?;
                if !same_object(
                    output.identity,
                    output_parents.last().ok_or(ErrorCode::WireInvalid)?.1,
                ) {
                    return Err(ErrorCode::IdentityConflict);
                }
                validate_output_tree(&output)?;
                if imported {
                    let mut path = base.clone();
                    path.push("import.json".into());
                    let imported_bytes = tree.read(path, bound::MAX_DISPATCH_BYTES)?;
                    let mut path = output_path.clone();
                    path.push("dispatch.json".into());
                    if tree.read(path, bound::MAX_DISPATCH_BYTES)? != imported_bytes {
                        return Err(ErrorCode::IdentityConflict);
                    }
                    let dispatch = wire::BoundDispatch::parse(&imported_bytes)
                        .map_err(|_| ErrorCode::IdentityConflict)?;
                    if dispatch.action.canonical != binding.canonical
                        || dispatch.root.canonical != root.canonical
                    {
                        return Err(ErrorCode::IdentityConflict);
                    }
                }
                selected_output = Some(output);
                continue;
            }
            if !imported {
                return Err(ErrorCode::CommitUnknown);
            }
            let mut path = base;
            path.push("import.json".into());
            let imported_bytes = tree.read(path, bound::MAX_DISPATCH_BYTES)?;
            let mut path = output_path.clone();
            path.push("dispatch.json".into());
            if tree.read(path, bound::MAX_DISPATCH_BYTES)? != imported_bytes {
                return Err(ErrorCode::IdentityConflict);
            }
            let dispatch = wire::BoundDispatch::parse(&imported_bytes)
                .map_err(|_| ErrorCode::IdentityConflict)?;
            if dispatch.root.canonical != root.canonical
                || dispatch.action.canonical != binding.canonical
                || dispatch.artifact.as_ref().map(|a| &a.sha256)
                    != artifact.as_ref().map(|a| &a.sha256)
            {
                return Err(ErrorCode::IdentityConflict);
            }
            for bytes in [
                root.canonical.as_bytes(),
                binding.canonical.as_bytes(),
                dispatch.request.canonical.as_bytes(),
                imported_bytes.as_slice(),
            ] {
                tree.output_original(sequence, bytes)?;
            }
            if let Some(bytes) = evidence_bytes {
                tree.output_original(sequence, &bytes)?;
            }
            let following_sequence = if sequence == 0 {
                1
            } else {
                sequence.checked_add(2).ok_or(ErrorCode::WireInvalid)?
            };
            let following = stages
                .get(usize::try_from(following_sequence).map_err(|_| ErrorCode::WireInvalid)?)
                .ok_or(ErrorCode::IdentityConflict)?;
            let artifact_hash = field_text(following, "artifact_sha256")?;
            let selected = tree.object(artifact_hash)?;
            let result = if sequence == 0 {
                let (_, event, epoch) = begin_outcome(&selected, &root.intent)?;
                if selected["outcome"] != "OK" || epoch != fence || following["event_id"] != event {
                    return Err(ErrorCode::IdentityConflict);
                }
                selected
            } else {
                validate_summary_binding(&selected, root, &binding, following)?;
                json!({"schema":"owner-beta-paper-action-v1", "action":"RECONCILE_OPERATION",
                    "outcome":"OK", "control_epoch":fence,"account_id":root.intent.account_id,
                    "data":{"observation_id":selected["observation_id"],"summary":selected,
                        "summary_sha256":artifact_hash}})
            };
            let canonical = canonical_text(&result)?;
            // Derived expected bytes only locate/compare the actual protected
            // successful result. They cannot replace a missing original.
            tree.output_original(sequence, canonical.as_bytes())?;
            prior.push(PriorMaintenanceResult {
                request: dispatch.request,
                canonical,
                event_id: field_text(following, "event_id")?.to_owned(),
                epoch: fence,
            });
            let output_names = tree.names(output_path.clone())?;
            for leaf in output_names {
                if leaf == "dispatch.json" {
                    continue;
                }
                if leaf.len() != 69 || !leaf.ends_with(".json") {
                    return Err(ErrorCode::IdentityConflict);
                }
                let mut path = output_path.clone();
                path.push(leaf.clone());
                let bytes = tree.read(path, bound::MAX_DISPATCH_BYTES)?;
                if digest(&bytes) != format!("sha256:{}", &leaf[..64]) {
                    return Err(ErrorCode::IdentityConflict);
                }
            }
        }
        let evidence = wire::parse_bound_artifact(
            originals
                .artifact_bytes()
                .ok_or(ErrorCode::EvidenceUnavailable)?,
            root,
            action,
        )
        .map_err(|_| ErrorCode::WireInvalid)?
        .value;
        let inventory = if action.verb == Verb::Reconcile {
            if last["stage"] == "STOPPED"
                && last["artifact_sha256"] != action.artifact_sha256.as_deref().unwrap_or_default()
            {
                return Err(ErrorCode::IdentityConflict);
            }
            evidence.clone()
        } else {
            let summary = tree.object(field_text(last, "artifact_sha256")?)?;
            if evidence["reconciliation_sha256"] != last["artifact_sha256"]
                || evidence["observation_id"] != summary["observation_id"]
                || evidence["inventory_sha256"] != summary["inventory_sha256"]
                || summary["unresolved_commits"] != 0
                || summary["live_claims"] != 0
            {
                return Err(ErrorCode::WriterUnknown);
            }
            tree.object(field_text(&evidence, "inventory_sha256")?)?
        };
        let captured_pids = validate_writer_originals(&mut tree, root, &inventory, &stages[1])?;
        let pins = MaintenancePins::open(&root.intent)?;
        let mut live = Self {
            tree,
            state,
            pins,
            output: selected_output.ok_or(ErrorCode::EvidenceUnavailable)?,
            output_path: vec![
                "actions".into(),
                format!("{:08}", action.sequence),
                "output".into(),
            ],
            results: Vec::new(),
            prior,
            captured_pids,
            fence,
            inventory,
            evidence,
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
        self.tree.revalidate()?;
        self.state
            .revalidate(originals.root.intent.installation_id, self.fence)?;
        self.pins.revalidate()?;
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let named = open_relative_parent(&self.tree.root, &self.output_path)?;
        if !same_object(
            self.output.identity,
            named.last().ok_or(ErrorCode::IdentityConflict)?.1,
        ) {
            return Err(ErrorCode::IdentityConflict);
        }
        validate_output_tree(&self.output)?;
        for result in &self.results {
            result
                .revalidate(&self.output)
                .map_err(|_| ErrorCode::CommitUnknown)?;
        }
        check_clock(
            &self.inventory["observed"],
            field_text(&self.inventory, "boot_id")?,
        )?;
        check_clock(
            &self.evidence["observed"],
            field_text(&self.evidence, "boot_id")?,
        )?;
        Ok(())
    }

    pub(super) fn retain_result(
        &mut self,
        result: RetainedPublishedOutput,
    ) -> Result<(), ErrorCode> {
        result
            .revalidate(&self.output)
            .map_err(|_| ErrorCode::CommitUnknown)?;
        self.results.push(result);
        Ok(())
    }
}

fn validate_summary_binding(
    summary: &Value,
    root: &RootIntent,
    action: &wire::ActionBinding,
    stage: &Value,
) -> Result<(), ErrorCode> {
    wire::validate_summary(summary).map_err(|_| ErrorCode::WireInvalid)?;
    if summary["installation_id"] != root.intent.installation_id.to_string()
        || summary["generation"] != root.intent.generation.to_string()
        || summary["account_id"] != root.intent.value["account_id"]
        || summary["control_epoch"] != action.expected_epoch
        || summary["inventory_sha256"].as_str() != action.artifact_sha256.as_deref()
        || summary["observation_id"] != stage["event_id"]
    {
        return Err(ErrorCode::IdentityConflict);
    }
    Ok(())
}

fn collect_observation_links(value: &Value, links: &mut BTreeSet<String>) -> Result<(), ErrorCode> {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                if matches!(
                    key.as_str(),
                    "identity_sha256"
                        | "observation_sha256"
                        | "backend_set_sha256"
                        | "launch_fence_sha256"
                        | "backend_observation_sha256"
                        | "capture_sha256"
                        | "initial_sha256"
                ) && !child.is_null()
                {
                    let hash = child.as_str().ok_or(ErrorCode::WireInvalid)?;
                    digest_leaf(hash)?;
                    links.insert(hash.to_owned());
                }
                if matches!(key.as_str(), "containers" | "units")
                    && let Some(values) = child.as_object()
                {
                    for hash in values.values().filter_map(Value::as_str) {
                        digest_leaf(hash)?;
                        links.insert(hash.into());
                    }
                }
                if key == "processes"
                    && let Some(values) = child.as_array()
                {
                    for hash in values.iter().filter_map(Value::as_str) {
                        digest_leaf(hash)?;
                        links.insert(hash.into());
                    }
                }
                collect_observation_links(child, links)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_observation_links(child, links)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_writer_originals(
    tree: &mut MaintenanceTree,
    root: &RootIntent,
    inventory: &Value,
    fence_stage: &Value,
) -> Result<Vec<i32>, ErrorCode> {
    wire::validate_inventory_bound(inventory, root).map_err(|_| ErrorCode::WireInvalid)?;
    if inventory["fence_event_id"] != fence_stage["event_id"]
        || inventory["boot_id"] != fence_stage["boot_id"]
    {
        return Err(ErrorCode::IdentityConflict);
    }
    let required: &[&str] = if root.intent.scope == "COMBINED_BACKUP" {
        &[
            "API",
            "PAPER_RUNNER",
            "PAPER_DELIVERY",
            "SOURCE_PUBLISHER",
            "SOURCE_CHILD",
            "DAILY",
            "ADMITTER",
            "RECOMMENDATION",
            "SIMULATION",
            "RAW_CURATED",
            "OTHER_DB_WRITER",
        ]
    } else {
        &["API", "PAPER_RUNNER", "PAPER_DELIVERY"]
    };
    let rows = inventory["writers"]
        .as_array()
        .ok_or(ErrorCode::WireInvalid)?;
    if required
        .iter()
        .any(|class| !rows.iter().any(|row| row["writer_class"] == *class))
    {
        return Err(ErrorCode::WriterUnknown);
    }
    let backend_hash = field_text(inventory, "backend_observation_sha256")?;
    let backend = tree.object(backend_hash)?;
    let captured = validate_backend_original(&backend, root, fence_stage)?;
    let launch_hash = field_text(inventory, "launch_fence_sha256")?;
    let launch = tree.object(launch_hash)?;
    validate_launch_original(tree, &launch)?;
    let mut assigned_backends = BTreeSet::new();
    let mut expected_resources = BTreeSet::new();
    for row in rows {
        let identity = tree.object(field_text(row, "identity_sha256")?)?;
        if identity.get("writer_class").is_none() {
            expected_resources.insert((
                field_text(&identity, "kind")?.to_owned(),
                digest(canonical_text(&identity["generation"])?.as_bytes()),
            ));
        }
    }
    for row in rows {
        let identity_hash = field_text(row, "identity_sha256")?;
        let identity = tree.object(identity_hash)?;
        let observation = tree.object(field_text(row, "observation_sha256")?)?;
        let set_hash = field_text(row, "backend_set_sha256")?;
        let backend_set = tree.object(set_hash)?;
        if observation["identity_sha256"] != identity_hash
            || observation["first"].is_null()
            || observation["second"].is_null()
            || observation["second"]["launches"] != launch
        {
            return Err(ErrorCode::WriterUnknown);
        }
        for key in ["first", "second"] {
            let resources = observation[key]["resources"]
                .as_array()
                .ok_or(ErrorCode::WriterUnknown)?;
            let observed_resources: BTreeSet<_> = resources
                .iter()
                .map(|item| {
                    Ok((
                        field_text(item, "kind")?.to_owned(),
                        field_text(item, "identity_sha256")?.to_owned(),
                    ))
                })
                .collect::<Result<_, ErrorCode>>()?;
            if observed_resources != expected_resources
                || observed_resources.len() != resources.len()
            {
                return Err(ErrorCode::WriterUnknown);
            }
        }
        if identity.get("writer_class").is_some() {
            if row["terminal"] != "ABSENT_AT_FENCE"
                || identity["writer_class"] != row["writer_class"]
                || identity["installation_id"] != root.intent.installation_id.to_string()
                || identity["generation"] != root.intent.generation.to_string()
                || identity["boot_id"] != inventory["boot_id"]
                || identity["manifest"] != root.intent.release_manifest_sha256
                || set_hash != backend_hash
            {
                return Err(ErrorCode::WriterUnknown);
            }
            let capture = tree.object(field_text(&identity, "capture_sha256")?)?;
            if capture["boot_id"] != inventory["boot_id"]
                || capture["manifest"] != root.intent.release_manifest_sha256
                || capture["backend_observation"] != backend["initial"]
            {
                return Err(ErrorCode::WriterUnknown);
            }
            for key in ["first", "second"] {
                validate_launch_original(tree, &observation[key]["launches"])?;
                if !observation[key]["resources"].is_array() {
                    return Err(ErrorCode::WriterUnknown);
                }
            }
            if observation["launch_fence_sha256"] != launch_hash
                || observation["backend_observation_sha256"] != backend_hash
            {
                return Err(ErrorCode::WriterUnknown);
            }
            for key in ["containers", "units"] {
                let members = capture[key].as_object().ok_or(ErrorCode::WriterUnknown)?;
                for hash in members.values() {
                    let member = tree.object(hash.as_str().ok_or(ErrorCode::WriterUnknown)?)?;
                    if member["classes"]
                        .as_array()
                        .is_none_or(|classes| classes.contains(&row["writer_class"]))
                    {
                        return Err(ErrorCode::WriterUnknown);
                    }
                }
            }
        } else {
            if identity["kind"] != row["kind"]
                || identity["boot_id"] != inventory["boot_id"]
                || identity["manifest"] != root.intent.release_manifest_sha256
                || !identity["generation"]["classes"]
                    .as_array()
                    .is_some_and(|classes| classes.contains(&row["writer_class"]))
                || backend_set["identity_sha256"] != identity_hash
                || backend_set["backend_observation_sha256"] != backend_hash
            {
                return Err(ErrorCode::WriterUnknown);
            }
            let classes = identity["generation"]["classes"]
                .as_array()
                .ok_or(ErrorCode::WriterUnknown)?;
            if classes.is_empty()
                || classes.iter().any(|class| {
                    !rows.iter().any(|candidate| {
                        candidate["identity_sha256"] == identity_hash
                            && candidate["writer_class"] == *class
                    })
                })
            {
                return Err(ErrorCode::WriterUnknown);
            }
            let pid = identity["generation"]["pid"]
                .as_i64()
                .ok_or(ErrorCode::WriterUnknown)?;
            if (row["terminal"] == "EXITED_REAPED") != (pid > 1) {
                return Err(ErrorCode::WriterUnknown);
            }
            let members = backend_set["backends"]
                .as_array()
                .ok_or(ErrorCode::WriterUnknown)?;
            let all = backend["captured"]
                .as_array()
                .ok_or(ErrorCode::WriterUnknown)?;
            let resource_name = if row["kind"] == "CONTAINER" {
                &identity["generation"]["generation"]["Id"]
            } else {
                &identity["generation"]["name"]
            };
            if members.iter().any(|member| {
                !all.contains(member)
                    || member["process"]["owner"] != json!([row["kind"], resource_name])
            }) {
                return Err(ErrorCode::WriterUnknown);
            }
            for member in members {
                assigned_backends.insert(member["pid"].as_i64().ok_or(ErrorCode::WriterUnknown)?);
            }
            for key in ["first", "second"] {
                validate_terminal_original(tree, &observation[key], &identity)?;
                validate_launch_binding(tree, &observation[key]["launches"], &identity)?;
            }
            let resource_hash = digest(canonical_text(&identity["generation"])?.as_bytes());
            let group = |key: &str| {
                observation[key]["resources"]
                    .as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|item| item["identity_sha256"] == resource_hash)
                    })
                    .map(|item| item["cgroup"]["identity"].clone())
            };
            if group("first") != group("second") {
                return Err(ErrorCode::WriterUnknown);
            }
        }
    }
    if assigned_backends
        != captured
            .iter()
            .map(|pid| i64::from(*pid))
            .collect::<BTreeSet<_>>()
    {
        return Err(ErrorCode::WriterUnknown);
    }
    // Retain every transitive observation original, while distinguishing actual
    // graph links from descriptive ELF/image/release digests in root observations.
    let mut pending = BTreeSet::new();
    collect_observation_links(inventory, &mut pending)?;
    let mut visited = BTreeSet::new();
    while let Some(hash) = pending.pop_first() {
        if visited.insert(hash.clone()) {
            let object = tree.object(&hash)?;
            collect_observation_links(&object, &mut pending)?;
        }
    }
    Ok(captured)
}

fn validate_backend_original(
    value: &Value,
    root: &RootIntent,
    fence_stage: &Value,
) -> Result<Vec<i32>, ErrorCode> {
    if !exact_keys(
        value,
        &["captured", "initial", "final", "database_generation"],
    ) || value["final"]["prepared"] != 0
        || value["final"]["unexpected"] != 0
        || value["final"]["backends"] != json!([])
        || value["initial"]["prepared"] != 0
        || value["initial"]["unexpected"] != 0
        || value["final"]["fence"] != value["initial"]["fence"]
        || value["final"]["fence"]["generation"] != root.intent.generation.to_string()
        || value["final"]["fence"]["epoch"].as_i64()
            != Some(root.fence_epoch().map_err(|_| ErrorCode::StaleEpoch)?)
        || value["final"]["fence"]["state"] != "DRAINING"
        || value["final"]["fence"]["event"] != fence_stage["event_id"]
    {
        return Err(ErrorCode::WriterUnknown);
    }
    let generation = value["database_generation"]
        .as_array()
        .ok_or(ErrorCode::WriterUnknown)?;
    if generation.len() != 5
        || !generation[0].as_str().is_some_and(|id| {
            id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        || generation[1]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_none()
        || generation[2]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_none()
        || !generation[3].as_i64().is_some_and(|n| n >= 0)
        || !sha256_wire(&generation[4])
    {
        return Err(ErrorCode::WriterUnknown);
    }
    for key in ["initial", "final"] {
        let observation = &value[key];
        let observer = &observation["observer"];
        if !exact_keys(
            observation,
            &["observer", "prepared", "unexpected", "backends", "fence"],
        ) || !exact_keys(
            observer,
            &["pid", "user", "session", "read_only", "role", "superuser"],
        ) || !observer["pid"].as_i64().is_some_and(|pid| pid > 1)
            || !observer["user"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
            || observer["user"] != observer["session"]
            || observer["read_only"] != "on"
            || observer["role"] != "none"
            || observer["superuser"] != true
        {
            return Err(ErrorCode::WriterUnknown);
        }
    }
    let captured = value["captured"]
        .as_array()
        .ok_or(ErrorCode::WriterUnknown)?;
    let initial = value["initial"]["backends"]
        .as_array()
        .ok_or(ErrorCode::WriterUnknown)?;
    if captured.len() > 64 || captured.len() != initial.len() {
        return Err(ErrorCode::WriterUnknown);
    }
    let mut pids = BTreeSet::new();
    for row in captured {
        let pid = row["pid"]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| *n > 1)
            .ok_or(ErrorCode::WriterUnknown)?;
        if !pids.insert(pid)
            || !row["start_us"].as_i64().is_some_and(|n| n > 0)
            || !row["process"]["pid"].as_i64().is_some_and(|n| n > 1)
            || !row["process"]["start_ticks"]
                .as_i64()
                .is_some_and(|n| n > 0)
            || !row["socket"]["socket_inode"]
                .as_i64()
                .is_some_and(|n| n > 0)
            || row["socket"]["pid"] != row["process"]["pid"]
            || row["socket"]["start_ticks"] != row["process"]["start_ticks"]
            || row["socket"]["local"] != json!([row["addr"], row["port"]])
            || !row["socket"]["peer"]
                .as_array()
                .is_some_and(|peer| peer.len() == 2 && peer[0].is_string() && peer[1] == 5432)
            || ["initial", "final"]
                .iter()
                .any(|key| value[*key]["observer"]["pid"] == row["pid"])
            || !initial.iter().any(|before| {
                ["pid", "start_us", "role", "addr", "port"]
                    .iter()
                    .all(|key| before[*key] == row[*key])
            })
        {
            return Err(ErrorCode::WriterUnknown);
        }
    }
    Ok(pids.into_iter().collect())
}

fn validate_launch_original(tree: &mut MaintenanceTree, launch: &Value) -> Result<(), ErrorCode> {
    if !exact_keys(launch, &["units", "containers", "pending_owned_jobs"])
        || launch["pending_owned_jobs"] != json!([])
    {
        return Err(ErrorCode::WriterUnknown);
    }
    for key in ["units", "containers"] {
        let members = launch[key].as_object().ok_or(ErrorCode::WriterUnknown)?;
        for (name, hash) in members {
            let member = tree.object(hash.as_str().ok_or(ErrorCode::WriterUnknown)?)?;
            if member["pid"] != 0 {
                return Err(ErrorCode::WriterUnknown);
            }
            if key == "containers" {
                if member["generation"]["Id"] != *name || member["restart"] != "no" {
                    return Err(ErrorCode::WriterUnknown);
                }
            } else if member["name"] != *name
                || !matches!(member["active"].as_str(), Some("inactive" | "failed"))
                || !matches!(
                    member["file_state"].as_str(),
                    Some("disabled" | "static" | "indirect" | "masked" | "masked-runtime")
                )
            {
                return Err(ErrorCode::WriterUnknown);
            }
        }
    }
    Ok(())
}

fn validate_terminal_original(
    tree: &mut MaintenanceTree,
    observation: &Value,
    identity: &Value,
) -> Result<(), ErrorCode> {
    if !exact_keys(observation, &["resources", "launches"]) {
        return Err(ErrorCode::WriterUnknown);
    }
    validate_launch_original(tree, &observation["launches"])?;
    let hash = digest(canonical_text(&identity["generation"])?.as_bytes());
    let resources = observation["resources"]
        .as_array()
        .ok_or(ErrorCode::WriterUnknown)?;
    let matches: Vec<_> = resources
        .iter()
        .filter(|item| item["kind"] == identity["kind"] && item["identity_sha256"] == hash)
        .collect();
    if matches.len() != 1 {
        return Err(ErrorCode::WriterUnknown);
    }
    let resource = matches[0];
    if tree.object(field_text(resource, "identity_sha256")?)? != identity["generation"] {
        return Err(ErrorCode::IdentityConflict);
    }
    let pid = identity["generation"]["pid"]
        .as_i64()
        .ok_or(ErrorCode::WriterUnknown)?;
    let group = &resource["cgroup"];
    let has_group = identity["generation"]["cgroup"]
        .as_str()
        .is_some_and(|name| !name.is_empty());
    if group.is_null() {
        if pid > 1 || has_group {
            return Err(ErrorCode::WriterUnknown);
        }
    } else if !group["identity"].as_array().is_some_and(|id| {
        id.len() == 6
            && id.iter().all(|n| n.as_i64().is_some_and(|n| n >= 0))
            && id[1].as_i64().is_some_and(|n| n > 0)
            && id[5].as_i64().is_some_and(|n| n > 0)
    }) || !((group["removed"] == true && exact_keys(group, &["identity", "removed"]))
        || (group["removed"] == false
            && group["populated"] == 0
            && exact_keys(group, &["identity", "removed", "populated"])))
    {
        return Err(ErrorCode::WriterUnknown);
    }
    if identity["kind"] == "CONTAINER" {
        let terminal = &resource["terminal"];
        if !exact_keys(
            terminal,
            &["generation", "restart", "exit_code", "finished_at"],
        ) || terminal["generation"] != identity["generation"]["generation"]
            || terminal["restart"] != "no"
            || !terminal["exit_code"]
                .as_i64()
                .is_some_and(|code| (0..=255).contains(&code))
        {
            return Err(ErrorCode::WriterUnknown);
        }
        let generation = &terminal["generation"];
        if !exact_keys(
            generation,
            &["Id", "Image", "Created", "StartedAt", "RestartCount"],
        ) || !generation["Id"].as_str().is_some_and(|id| {
            id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) || !sha256_wire(&generation["Image"])
            || !generation["RestartCount"].as_i64().is_some_and(|n| n >= 0)
        {
            return Err(ErrorCode::WriterUnknown);
        }
        let created = chrono::DateTime::parse_from_rfc3339(field_text(generation, "Created")?)
            .map_err(|_| ErrorCode::WriterUnknown)?;
        let finish = chrono::DateTime::parse_from_rfc3339(field_text(terminal, "finished_at")?)
            .map_err(|_| ErrorCode::WriterUnknown)?;
        let start =
            chrono::DateTime::parse_from_rfc3339(field_text(&terminal["generation"], "StartedAt")?)
                .map_err(|_| ErrorCode::WriterUnknown)?;
        if finish < start
            || start < created
            || field_text(terminal, "finished_at")?.starts_with("0001-")
        {
            return Err(ErrorCode::WriterUnknown);
        }
    } else if identity["kind"] != "UNIT" {
        return Err(ErrorCode::WriterUnknown);
    }
    Ok(())
}

fn validate_launch_binding(
    tree: &mut MaintenanceTree,
    launch: &Value,
    identity: &Value,
) -> Result<(), ErrorCode> {
    let original = &identity["generation"];
    let (map, name, fields): (&str, &str, &[&str]) = if identity["kind"] == "CONTAINER" {
        (
            "containers",
            field_text(&original["generation"], "Id")?,
            &["generation", "mounts", "policy"],
        )
    } else if identity["kind"] == "UNIT" {
        if !original["originals"].is_array()
            || !sha256_wire(&original["source_config"])
            || !original["triggers"].is_array()
            || !original["triggered_by"].is_array()
        {
            return Err(ErrorCode::WriterUnknown);
        }
        (
            "units",
            field_text(original, "name")?,
            &["originals", "source_config", "triggers", "triggered_by"],
        )
    } else {
        return Err(ErrorCode::WriterUnknown);
    };
    let now = tree.object(launch[map][name].as_str().ok_or(ErrorCode::WriterUnknown)?)?;
    if fields
        .iter()
        .any(|key| original[*key].is_null() || now[*key] != original[*key])
    {
        return Err(ErrorCode::WriterUnknown);
    }
    Ok(())
}
