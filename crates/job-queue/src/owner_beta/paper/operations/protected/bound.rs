//! Fixed-name custody for §10. Live single-action and offline readback have
//! separate factories. Parsed bindings alone confer no maintenance/recovery authority.
use std::{cell::RefCell, os::unix::fs::FileExt};

use super::wire::{ActionBinding, BoundDispatch, RootIntent};
use super::*;

const ACTION_PATH: &str = "/run/owner-beta-paper-ops/action-binding.json";
pub(super) const MAX_DISPATCH_BYTES: usize = 2 * wire::MAX_EXACT_BYTES;

pub(super) struct BoundOriginals {
    pub root: RootIntent,
    action: ActionBinding,
    action_file: GuardedFile,
    artifact_bytes: Option<Vec<u8>>,
    custody: RefCell<Option<RetainedCustody>>,
}

type OpenedOriginals = (
    BoundOriginals,
    GuardedFile,
    Option<GuardedFile>,
    Option<Artifact>,
);
type OpenedReadback = (
    BoundOriginals,
    GuardedFile,
    Option<GuardedFile>,
    Option<Artifact>,
    (GuardedFile, ExactRequest),
);
type NativeContext = (
    ProtectedInputs,
    Intent,
    Option<Artifact>,
    GuardedDirectory,
    DatabaseSelector,
);

fn entry_exists(directory: &GuardedDirectory, leaf: &str) -> Result<bool, ErrorCode> {
    directory
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let leaf = CString::new(leaf).map_err(|_| ErrorCode::WireInvalid)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.directory.as_raw_fd(),
            leaf.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
        Ok(false)
    } else {
        Err(ErrorCode::ReaderIncompatible)
    }
}

pub(super) fn has_action() -> Result<bool, ErrorCode> {
    let directory =
        GuardedDirectory::open(PRIVATE_DIR).map_err(|_| ErrorCode::ReaderIncompatible)?;
    entry_exists(&directory, "action-binding.json")
}

pub(super) fn has_dispatch() -> Result<bool, ErrorCode> {
    let directory =
        GuardedDirectory::open(OUTPUT_PATH).map_err(|_| ErrorCode::ReaderIncompatible)?;
    entry_exists(&directory, "dispatch.json")
}

fn read_bytes(file: &File, max: usize) -> Result<Vec<u8>, ErrorCode> {
    let size = usize::try_from(
        file.metadata()
            .map_err(|_| ErrorCode::ReaderIncompatible)?
            .len(),
    )
    .map_err(|_| ErrorCode::WireInvalid)?;
    if size > max {
        return Err(ErrorCode::WireInvalid);
    }
    let mut bytes = vec![0; size];
    file.read_exact_at(&mut bytes, 0)
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(bytes)
}

fn recheck_original(file: &GuardedFile, expected: &[u8]) -> Result<(), ErrorCode> {
    file.revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    if read_bytes(&file.file, expected.len())? != expected {
        return Err(ErrorCode::IdentityConflict);
    }
    file.revalidate().map_err(|_| ErrorCode::ReaderIncompatible)
}

struct RetainedOutput {
    file: File,
    identity: Identity,
    leaf: String,
    bytes: Vec<u8>,
}

impl RetainedOutput {
    fn open(output: &GuardedDirectory, leaf: String, bytes: &[u8]) -> Result<Self, ErrorCode> {
        output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let file = openat_file(
            output.directory.as_raw_fd(),
            &leaf,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let identity = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
        let object = Self {
            file,
            identity,
            leaf,
            bytes: bytes.to_vec(),
        };
        object.revalidate(output)?;
        Ok(object)
    }

    fn revalidate(&self, output: &GuardedDirectory) -> Result<(), ErrorCode> {
        if check_file(&self.file).map_err(|_| ErrorCode::ReaderIncompatible)? != self.identity
            || read_bytes(&self.file, self.bytes.len())? != self.bytes
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let named = openat_file(
            output.directory.as_raw_fd(),
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

struct RetainedCustody {
    output: GuardedDirectory,
    files: Vec<RetainedOutput>,
}

impl RetainedCustody {
    fn open(output: &GuardedDirectory, dispatch: &BoundDispatch) -> Result<Self, ErrorCode> {
        let retained =
            GuardedDirectory::open(output.path).map_err(|_| ErrorCode::ReaderIncompatible)?;
        if !same_object(retained.identity, output.identity) {
            return Err(ErrorCode::IdentityConflict);
        }
        // Read the fixed index first; a changed index is an identity conflict,
        // even when the other action's content-addressed files are absent.
        let fixed = RetainedOutput::open(
            &retained,
            "dispatch.json".to_owned(),
            dispatch.canonical.as_bytes(),
        )?;
        let artifact_bytes = dispatch
            .artifact
            .as_ref()
            .map(|a| canonical_text(&a.value))
            .transpose()?;
        let mut originals = vec![
            dispatch.root.canonical.as_bytes(),
            dispatch.action.canonical.as_bytes(),
            dispatch.request.canonical.as_bytes(),
        ];
        if let Some(bytes) = &artifact_bytes {
            originals.push(bytes.as_bytes());
        }
        originals.push(dispatch.canonical.as_bytes());
        if digest(dispatch.canonical.as_bytes()) != dispatch.sha256 {
            return Err(ErrorCode::IdentityConflict);
        }
        let mut files = Vec::with_capacity(originals.len() + 1);
        for bytes in originals {
            let hash = digest(bytes);
            files.push(RetainedOutput::open(
                &retained,
                format!("{}.json", &hash[7..]),
                bytes,
            )?);
        }
        files.push(fixed);
        let custody = Self {
            output: retained,
            files,
        };
        custody.revalidate()?;
        Ok(custody)
    }

    fn revalidate(&self) -> Result<(), ErrorCode> {
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        for file in &self.files {
            file.revalidate(&self.output)?;
        }
        self.output
            .revalidate()
            .map_err(|_| ErrorCode::ReaderIncompatible)
    }
}

// Empty is the only new-output state. Any orphan, torn file or absent named
// original remains unresolved; neither enumeration nor SQL can reconstruct it.
fn output_is_empty(output: &GuardedDirectory) -> Result<bool, ErrorCode> {
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let mut entries = std::fs::read_dir(format!("/proc/self/fd/{}", output.directory.as_raw_fd()))
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let empty = match entries.next() {
        None => true,
        Some(Ok(_)) => false,
        Some(Err(_)) => return Err(ErrorCode::EvidenceUnavailable),
    };
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(empty)
}

enum IndexPublication {
    Created,
    Existing,
}

fn publish_fixed_index(
    output: &GuardedDirectory,
    bytes: &[u8],
) -> Result<(IndexPublication, RetainedOutput), ErrorCode> {
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let name = CString::new("dispatch.json").map_err(|_| ErrorCode::InternalFailure)?;
    let fd = unsafe {
        libc::openat(
            output.directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o400,
        )
    };
    if fd < 0 {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(ErrorCode::EvidenceUnavailable);
        }
        // Identical existing bytes are custody, not a permit to submit again.
        let existing = RetainedOutput::open(output, "dispatch.json".to_owned(), bytes)?;
        existing
            .file
            .sync_all()
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        output
            .directory
            .sync_all()
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        existing.revalidate(output)?;
        return Ok((IndexPublication::Existing, existing));
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    file.sync_all()
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let id = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
    output
        .directory
        .sync_all()
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let original = RetainedOutput {
        file,
        identity: id,
        leaf: "dispatch.json".to_owned(),
        bytes: bytes.to_vec(),
    };
    original.revalidate(output)?;
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok((IndexPublication::Created, original))
}

fn publish_original(output: &GuardedDirectory, bytes: &[u8]) -> Result<RetainedOutput, ErrorCode> {
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    let leaf = format!("{}.json", &digest(bytes)[7..]);
    let name = CString::new(leaf.as_str()).map_err(|_| ErrorCode::InternalFailure)?;
    let fd = unsafe {
        libc::openat(
            output.directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o400,
        )
    };
    if fd < 0 {
        // An orphan is never reconstructed or adopted as permission to submit.
        return Err(
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
                ErrorCode::CommitUnknown
            } else {
                ErrorCode::EvidenceUnavailable
            },
        );
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    file.sync_all()
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let id = check_file(&file).map_err(|_| ErrorCode::ReaderIncompatible)?;
    output
        .directory
        .sync_all()
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
    let original = RetainedOutput {
        file,
        identity: id,
        leaf,
        bytes: bytes.to_vec(),
    };
    original.revalidate(output)?;
    output
        .revalidate()
        .map_err(|_| ErrorCode::ReaderIncompatible)?;
    Ok(original)
}

fn persist_bound_index(
    output: &GuardedDirectory,
    dispatch: &BoundDispatch,
) -> Result<(IndexPublication, RetainedCustody), ErrorCode> {
    persist_bound_index_at(output, dispatch, |_| Ok(()))
}

// The checkpoint can only fail this private storage operation. Production
// supplies no callback input, runtime selector, bypass, or authority flag.
fn persist_bound_index_at(
    output: &GuardedDirectory,
    dispatch: &BoundDispatch,
    mut checkpoint: impl FnMut(usize) -> Result<(), ErrorCode>,
) -> Result<(IndexPublication, RetainedCustody), ErrorCode> {
    if entry_exists(output, "dispatch.json")? {
        return Ok((
            IndexPublication::Existing,
            RetainedCustody::open(output, dispatch)?,
        ));
    }
    if !output_is_empty(output)? {
        return Err(ErrorCode::CommitUnknown);
    }
    let artifact_bytes = dispatch
        .artifact
        .as_ref()
        .map(|a| canonical_text(&a.value))
        .transpose()?;
    let mut originals = vec![
        dispatch.root.canonical.as_bytes(),
        dispatch.action.canonical.as_bytes(),
        dispatch.request.canonical.as_bytes(),
    ];
    if let Some(bytes) = &artifact_bytes {
        originals.push(bytes.as_bytes());
    }
    originals.push(dispatch.canonical.as_bytes());
    let mut files = Vec::with_capacity(originals.len() + 1);
    for (n, bytes) in originals.into_iter().enumerate() {
        files.push(publish_original(output, bytes)?);
        checkpoint(n)?;
    }
    let (published, fixed) = publish_fixed_index(output, dispatch.canonical.as_bytes())?;
    files.push(fixed);
    checkpoint(files.len() - 1)?;
    let retained =
        GuardedDirectory::open(output.path).map_err(|_| ErrorCode::ReaderIncompatible)?;
    if !same_object(retained.identity, output.identity) {
        return Err(ErrorCode::IdentityConflict);
    }
    let custody = RetainedCustody {
        output: retained,
        files,
    };
    custody.revalidate()?;
    Ok((published, custody))
}

fn original_evidence(
    action: &ActionBinding,
) -> Result<(Option<GuardedFile>, Option<Vec<u8>>), ErrorCode> {
    if action.artifact_sha256.is_some() {
        let (file, bytes) = read_fixed(EVIDENCE_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        Ok((Some(file), Some(bytes)))
    } else {
        let directory =
            GuardedDirectory::open(PRIVATE_DIR).map_err(|_| ErrorCode::ReaderIncompatible)?;
        if entry_exists(&directory, "evidence.json")? {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok((None, None))
    }
}

impl BoundOriginals {
    fn open_originals(verb: Verb) -> Result<OpenedOriginals, ErrorCode> {
        let (request, root_bytes) = read_fixed(REQUEST_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let root = wire::parse_root_intent(&root_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        let (action_file, action_bytes) = read_fixed(ACTION_PATH, wire::MAX_WIRE_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let action =
            wire::parse_action(&action_bytes, &root).map_err(|_| ErrorCode::IdentityConflict)?;
        if verb != Verb::Readback && action.verb != verb {
            return Err(ErrorCode::IdentityConflict);
        }
        let (evidence, artifact_bytes) = original_evidence(&action)?;
        let artifact = artifact_bytes
            .as_ref()
            .map(|bytes| wire::parse_bound_artifact(bytes, &root, &action))
            .transpose()
            .map_err(|_| ErrorCode::WireInvalid)?;
        Ok((
            Self {
                root,
                action,
                action_file,
                artifact_bytes,
                custody: RefCell::new(None),
            },
            request,
            evidence,
            artifact,
        ))
    }

    fn open_live(verb: Verb) -> Result<OpenedOriginals, ErrorCode> {
        let originals = Self::open_originals(verb)?;
        if originals.0.root.maintenance()
            && (!matches!(verb, Verb::Begin | Verb::Reconcile | Verb::Off)
                || !matches!(originals.0.root.intent.purpose.as_str(), "STOP" | "BACKUP"))
        {
            // Each supported mutation still needs the separate retained live
            // chain and selected-image pins. RESTORE has no archive proof.
            return Err(ErrorCode::EvidenceUnavailable);
        }
        if !originals.0.root.maintenance()
            && (originals.0.action.sequence != 0 || originals.0.action.previous_binding_sha256.is_some())
        {
            return Err(ErrorCode::IdentityConflict);
        }
        Ok(originals)
    }

    pub(super) fn action(&self) -> &ActionBinding {
        &self.action
    }

    pub(super) fn action_bytes(&self) -> &[u8] {
        self.action.canonical.as_bytes()
    }

    pub(super) fn artifact_bytes(&self) -> Option<&[u8]> {
        self.artifact_bytes.as_deref()
    }

    pub(super) fn revalidate_originals(&self, inputs: &ProtectedInputs) -> Result<(), ErrorCode> {
        self.revalidate(inputs)
    }

    fn open_readback(output: &GuardedDirectory) -> Result<OpenedReadback, ErrorCode> {
        let (originals, request, evidence, artifact) = Self::open_originals(Verb::Readback)?;
        let (sql_file, sql_bytes) = read_fixed(SQL_REQUEST_PATH, wire::MAX_EXACT_BYTES)
            .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let sql = wire::parse_exact_bound(&sql_bytes, &originals.root, &originals.action)
            .map_err(|_| ErrorCode::IdentityConflict)?;
        let fixed = openat_file(
            output.directory.as_raw_fd(),
            "dispatch.json",
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
        .map_err(|_| ErrorCode::EvidenceUnavailable)?;
        let fixed_id = check_file(&fixed).map_err(|_| ErrorCode::ReaderIncompatible)?;
        let dispatch = BoundDispatch::parse(&read_bytes(&fixed, MAX_DISPATCH_BYTES)?)
            .map_err(|_| ErrorCode::WireInvalid)?;
        if dispatch.root.canonical != originals.root.canonical
            || dispatch.action.canonical != originals.action.canonical
            || dispatch.request.canonical != sql.canonical
            || dispatch.artifact.as_ref().map(|a| &a.sha256) != artifact.as_ref().map(|a| &a.sha256)
        {
            return Err(ErrorCode::IdentityConflict);
        }
        let custody = RetainedCustody::open(output, &dispatch)?;
        if custody.files.last().map(|f| f.identity) != Some(fixed_id)
            || check_file(&fixed).map_err(|_| ErrorCode::ReaderIncompatible)? != fixed_id
        {
            return Err(ErrorCode::IdentityConflict);
        }
        *originals.custody.borrow_mut() = Some(custody);
        // No operation/, Current, Daily, worker, provenance, anchor, expiry or
        // current-epoch access occurs anywhere in this deliberately small path.
        Ok((originals, request, evidence, artifact, (sql_file, sql)))
    }

    pub(super) fn revalidate(&self, inputs: &ProtectedInputs) -> Result<(), ErrorCode> {
        recheck_original(&inputs.request, self.root.canonical.as_bytes())?;
        recheck_original(&self.action_file, self.action.canonical.as_bytes())?;
        match (&inputs.evidence, &self.artifact_bytes) {
            (Some(file), Some(bytes)) => recheck_original(file, bytes)?,
            (None, None) => {
                original_evidence(&self.action)?;
            }
            _ => return Err(ErrorCode::IdentityConflict),
        }
        if let Some((file, sql)) = &inputs.saved_request {
            recheck_original(file, sql.canonical.as_bytes())?;
        }
        if let Some(custody) = &*self.custody.borrow() {
            custody.revalidate()?;
        }
        Ok(())
    }

    pub(super) fn persist_bound_dispatch(
        &self,
        output: &GuardedDirectory,
        inputs: &ProtectedInputs,
        artifact: Option<&Artifact>,
        request: &ExactRequest,
    ) -> Result<(), ErrorCode> {
        self.revalidate(inputs)?;
        let dispatch = BoundDispatch::build(&self.root, &self.action, artifact, request)
            .map_err(|_| ErrorCode::IdentityConflict)?;
        let (published, custody) = persist_bound_index(output, &dispatch)?;
        *self.custody.borrow_mut() = Some(custody);
        self.revalidate(inputs)?;
        if matches!(published, IndexPublication::Existing) {
            return Err(ErrorCode::CommitUnknown);
        }
        Ok(())
    }
}

pub(super) fn load_bound_context(verb: Verb) -> Result<NativeContext, ErrorCode> {
    let load = || -> Result<_, ErrorCode> {
        let output =
            GuardedDirectory::open(OUTPUT_PATH).map_err(|_| ErrorCode::ReaderIncompatible)?;
        let (bound, request, evidence, artifact, saved_request) = if verb == Verb::Readback {
            let (bound, request, evidence, artifact, saved) =
                BoundOriginals::open_readback(&output)?;
            (bound, request, evidence, artifact, Some(saved))
        } else {
            let (bound, request, evidence, artifact) = BoundOriginals::open_live(verb)?;
            (bound, request, evidence, artifact, None)
        };
        let (database, db_bytes) = read_fixed(DATABASE_PATH, MAX_DB_SELECTOR)
            .map_err(|_| ErrorCode::ReaderIncompatible)?;
        let selector = parse_database(&db_bytes).map_err(|_| ErrorCode::WireInvalid)?;
        // Intent remains the immutable R. Only single-action live consumers use
        // this legacy view; maintenance uses the explicit closed boundary above.
        let intent = bound.root.intent.clone();
        let inputs = ProtectedInputs {
            request,
            database,
            evidence,
            saved_request,
            bound: Some(bound),
        };
        inputs.revalidate()?;
        Ok((inputs, intent, artifact, output, selector))
    };
    load()
}

#[cfg(test)]
mod tests;
