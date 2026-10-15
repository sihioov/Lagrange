//! Descriptor-bound candidate storage. Stored candidates remain unapproved.
//! Linux only; no path fallback, hard links, mutable latest pointer or DB effect.

use std::fmt;
use std::path::Path;

use domain::ContentHash;

use super::candidate::HistoricalDailyCandidate;
#[cfg(target_os = "linux")]
use super::contract::fixed_limits;
use super::{DailyNormalizationProofV1Wire, DailyPriceManifestV1Wire};
#[cfg(target_os = "linux")]
use super::{
    encode_daily_normalization_proof_v1, encode_daily_price_manifest_v1,
    parse_daily_normalization_proof_v1, parse_daily_price_manifest_v1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyArtifactError {
    UnsupportedPlatform,
    InvalidRoot,
    UnsafePath,
    UnexpectedEntries,
    SourceChanged,
    InvalidArtifact,
    LimitExceeded,
    Conflict,
    Io,
    CleanupFailed,
    DurabilityUnknown,
}

impl fmt::Display for DailyArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedPlatform => "DAILY_ARTIFACT_UNSUPPORTED_PLATFORM",
            Self::InvalidRoot => "DAILY_ARTIFACT_INVALID_ROOT",
            Self::UnsafePath => "DAILY_ARTIFACT_UNSAFE_PATH",
            Self::UnexpectedEntries => "DAILY_ARTIFACT_UNEXPECTED_ENTRIES",
            Self::SourceChanged => "DAILY_ARTIFACT_SOURCE_CHANGED",
            Self::InvalidArtifact => "DAILY_ARTIFACT_INVALID",
            Self::LimitExceeded => "DAILY_ARTIFACT_LIMIT",
            Self::Conflict => "DAILY_ARTIFACT_CONFLICT",
            Self::Io => "DAILY_ARTIFACT_IO",
            Self::CleanupFailed => "DAILY_ARTIFACT_CLEANUP_FAILED",
            Self::DurabilityUnknown => "DAILY_ARTIFACT_DURABILITY_UNKNOWN",
        })
    }
}
impl std::error::Error for DailyArtifactError {}

/// Opens an existing installer-owned root. Owner/group are explicit constraints,
/// not an installer receipt or permission to provision/chown the root.
pub struct DailyCandidateRoot {
    #[cfg(target_os = "linux")]
    inner: std::sync::Arc<linux::Root>,
}

/// Separate installer-owned admission root. It is never provisioned or inferred
/// from the candidate root, and cannot be written by the candidate writer API.
pub struct DailyAdmissionRoot {
    #[cfg(target_os = "linux")]
    inner: std::sync::Arc<linux::Root>,
}

/// Separate admitter-owned operational state. It is not a D4 artifact root.
pub struct DailySourceFrontierRoot {
    #[cfg(target_os = "linux")]
    inner: std::sync::Arc<linux::Root>,
}
impl DailySourceFrontierRoot {
    pub fn open(
        path: &Path,
        trusted_owner: u32,
        reader_group: u32,
    ) -> Result<Self, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                inner: std::sync::Arc::new(linux::Root::open_frontier(
                    path,
                    trusted_owner,
                    reader_group,
                )?),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, trusted_owner, reader_group);
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}

/// A per-capture entry bound, including invisible staging entries. There is no
/// default budget and this value grants no provider acquisition permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DailyArtifactNameLimits {
    max_entries: usize,
}
impl DailyArtifactNameLimits {
    pub fn new(max_entries: u64) -> Result<Self, DailyArtifactError> {
        if max_entries == 0 || max_entries > super::contract::fixed_limits().max_sessions {
            return Err(DailyArtifactError::LimitExceeded);
        }
        Ok(Self {
            max_entries: usize::try_from(max_entries)
                .map_err(|_| DailyArtifactError::LimitExceeded)?,
        })
    }
    pub fn max_entries(self) -> usize {
        self.max_entries
    }
}

/// A stable capture of names only, never evidence, approval or a latest pointer.
/// Recognized .pending-PID-sequence entries consume the bound but are invisible.
/// They are never followed, opened, removed, or promoted by this API.
pub struct CapturedDailyArtifactNames {
    names: Vec<ContentHash>,
    #[cfg(target_os = "linux")]
    captured: linux::NameCapture,
}
impl CapturedDailyArtifactNames {
    pub fn names(&self) -> &[ContentHash] {
        &self.names
    }
    pub fn revalidate(&self) -> Result<(), DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            self.captured.revalidate()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}

impl DailyCandidateRoot {
    pub fn capture_names(
        &self,
        limits: DailyArtifactNameLimits,
    ) -> Result<CapturedDailyArtifactNames, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            capture_root_names(&self.inner, limits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = limits;
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}
impl DailyAdmissionRoot {
    pub fn capture_names(
        &self,
        limits: DailyArtifactNameLimits,
    ) -> Result<CapturedDailyArtifactNames, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            capture_root_names(&self.inner, limits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = limits;
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}
impl DailySourceFrontierRoot {
    pub fn capture_names(
        &self,
        limits: DailyArtifactNameLimits,
    ) -> Result<CapturedDailyArtifactNames, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            capture_root_names(&self.inner, limits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = limits;
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}
#[cfg(target_os = "linux")]
fn capture_root_names(
    root: &std::sync::Arc<linux::Root>,
    limits: DailyArtifactNameLimits,
) -> Result<CapturedDailyArtifactNames, DailyArtifactError> {
    let (captured, names) = linux::capture_names(root, limits.max_entries())?;
    Ok(CapturedDailyArtifactNames { names, captured })
}

/// Unapproved frontier record bytes only; its observation payload remains
/// unopened. A full input-bound frontier read is still mandatory.
pub(super) fn read_source_frontier_metadata(
    root: &DailySourceFrontierRoot,
    frontier: &ContentHash,
) -> Result<Vec<u8>, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        linux::read_frontier_metadata(&root.inner, &leaf_name(frontier)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, frontier);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

pub(super) struct OpenedSourceFrontierFiles {
    record: Vec<u8>,
    observation: Vec<u8>,
    #[cfg(target_os = "linux")]
    files: linux::OpenedLeaf,
}
impl OpenedSourceFrontierFiles {
    pub(super) fn record_bytes(&self) -> &[u8] {
        &self.record
    }
    pub(super) fn observation_bytes(&self) -> &[u8] {
        &self.observation
    }
    pub(super) fn revalidate(&self) -> Result<(), DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            self.files.revalidate()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}

pub(super) fn read_source_frontier_files(
    root: &DailySourceFrontierRoot,
    hash: &ContentHash,
    admitted: &OpenedDailyCandidate,
) -> Result<OpenedSourceFrontierFiles, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        admitted.revalidate()?;
        if !root.inner.state_for_admission(&admitted.files.root) {
            return Err(DailyArtifactError::InvalidRoot);
        }
        let (files, mut bytes) = linux::read_leaf(&root.inner, &leaf_name(hash)?)?;
        let record = bytes
            .remove("frontier.json")
            .ok_or(DailyArtifactError::InvalidArtifact)?;
        let observation = bytes
            .remove("observation.json")
            .ok_or(DailyArtifactError::InvalidArtifact)?;
        if ContentHash::from_bytes(&observation) != *hash {
            return Err(DailyArtifactError::InvalidArtifact);
        }
        admitted.revalidate()?;
        files.revalidate()?;
        Ok(OpenedSourceFrontierFiles {
            record,
            observation,
            files,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, hash, admitted);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}
pub(super) fn publish_source_frontier_files(
    root: &DailySourceFrontierRoot,
    hash: &ContentHash,
    admitted: &OpenedDailyCandidate,
    record: &[u8],
    observation: &[u8],
) -> Result<(), DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        admitted.revalidate()?;
        if !root.inner.state_for_admission(&admitted.files.root) {
            return Err(DailyArtifactError::InvalidRoot);
        }
        if ContentHash::from_bytes(observation) != *hash {
            return Err(DailyArtifactError::InvalidArtifact);
        }
        linux::publish(
            &root.inner,
            &leaf_name(hash)?,
            &[("frontier.json", record), ("observation.json", observation)],
        )?;
        admitted.revalidate()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, hash, admitted, record, observation);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

impl DailyAdmissionRoot {
    pub fn open(
        path: &Path,
        trusted_owner: u32,
        reader_group: u32,
    ) -> Result<Self, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                inner: std::sync::Arc::new(linux::Root::open_admitted(
                    path,
                    trusted_owner,
                    reader_group,
                )?),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, trusted_owner, reader_group);
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}

impl DailyCandidateRoot {
    pub fn open(
        path: &Path,
        trusted_owner: u32,
        reader_group: u32,
    ) -> Result<Self, DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                inner: std::sync::Arc::new(linux::Root::open(path, trusted_owner, reader_group)?),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, trusted_owner, reader_group);
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }
}

/// Canonical file bytes with retained read descriptors, never an admitted input.
/// A producer owns this tree; callers must independently reconstruct the sources.
pub struct OpenedDailyCandidate {
    manifest: DailyPriceManifestV1Wire,
    normalization: DailyNormalizationProofV1Wire,
    manifest_bytes: Vec<u8>,
    normalization_bytes: Vec<u8>,
    bars: Vec<u8>,
    #[cfg(target_os = "linux")]
    files: linux::OpenedLeaf,
}

impl OpenedDailyCandidate {
    pub fn manifest(&self) -> &DailyPriceManifestV1Wire {
        &self.manifest
    }
    pub fn normalization(&self) -> &DailyNormalizationProofV1Wire {
        &self.normalization
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }
    pub fn normalization_bytes(&self) -> &[u8] {
        &self.normalization_bytes
    }
    pub fn bars_ndjson(&self) -> &[u8] {
        &self.bars
    }
    pub fn revalidate(&self) -> Result<(), DailyArtifactError> {
        #[cfg(target_os = "linux")]
        {
            self.files.revalidate()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DailyArtifactError::UnsupportedPlatform)
        }
    }

    /// The argument must come from the verifier's own captured source scan.
    /// Equality alone does not establish process separation, approval or DB state.
    pub fn matches_reconstruction(
        &self,
        candidate: &HistoricalDailyCandidate<'_, '_, '_, '_>,
    ) -> Result<(), DailyArtifactError> {
        self.revalidate()?;
        if self.manifest_bytes != candidate.manifest_bytes()
            || self.normalization_bytes != candidate.normalization_bytes()
            || self.bars != candidate.bars_ndjson()
        {
            return Err(DailyArtifactError::Conflict);
        }
        self.revalidate()
    }
    pub fn matches_continuation_reconstruction(
        &self,
        candidate: &super::continuation::DailyContinuationCandidate<'_, '_, '_>,
    ) -> Result<(), DailyArtifactError> {
        self.revalidate()?;
        if self.manifest_bytes != candidate.manifest_bytes()
            || self.normalization_bytes != candidate.normalization_bytes()
            || self.bars != candidate.bars_ndjson()
        {
            return Err(DailyArtifactError::Conflict);
        }
        self.revalidate()
    }
}

pub fn read_daily_candidate(
    root: &DailyCandidateRoot,
    content: &ContentHash,
) -> Result<OpenedDailyCandidate, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        let (files, bytes) = linux::read_leaf(&root.inner, &leaf_name(content)?)?;
        opened_from_files(content, files, bytes)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, content);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn opened_from_files(
    content: &ContentHash,
    files: linux::OpenedLeaf,
    mut bytes: std::collections::BTreeMap<&'static str, Vec<u8>>,
) -> Result<OpenedDailyCandidate, DailyArtifactError> {
    let manifest_bytes = bytes
        .remove("manifest.json")
        .ok_or(DailyArtifactError::InvalidArtifact)?;
    let normalization_bytes = bytes
        .remove("normalization.json")
        .ok_or(DailyArtifactError::InvalidArtifact)?;
    let bars = bytes
        .remove("bars.ndjson")
        .ok_or(DailyArtifactError::InvalidArtifact)?;
    let (manifest, normalization) =
        check_bytes(content, &manifest_bytes, &normalization_bytes, &bars)?;
    files.revalidate()?;
    Ok(OpenedDailyCandidate {
        manifest,
        normalization,
        manifest_bytes,
        normalization_bytes,
        bars,
        files,
    })
}

/// Unapproved parent-navigation bytes only. Full bundle/base verification is
/// mandatory before any admitted capability can be constructed.
pub(super) fn read_admission_metadata(
    root: &DailyAdmissionRoot,
    admission: &ContentHash,
) -> Result<Vec<u8>, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        linux::read_admission_metadata(&root.inner, &leaf_name(admission)?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, admission);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

pub(super) fn read_admission_files(
    root: &DailyAdmissionRoot,
    admission: &ContentHash,
    content: &ContentHash,
) -> Result<(OpenedDailyCandidate, Vec<u8>), DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        let (files, mut bytes) = linux::read_leaf(&root.inner, &leaf_name(admission)?)?;
        let admission_bytes = bytes
            .remove("admission.json")
            .ok_or(DailyArtifactError::InvalidArtifact)?;
        if ContentHash::from_bytes(&admission_bytes) != *admission {
            return Err(DailyArtifactError::InvalidArtifact);
        }
        Ok((opened_from_files(content, files, bytes)?, admission_bytes))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, admission, content);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

pub(super) fn publish_verified_admission(
    root: &DailyAdmissionRoot,
    verified: &super::admission::VerifiedHistoricalAdmission<'_, '_>,
) -> Result<(), DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        verified.candidate().revalidate()?;
        if !root
            .inner
            .independent_from(&verified.candidate().files.root)
        {
            return Err(DailyArtifactError::InvalidRoot);
        }
        linux::publish(
            &root.inner,
            &leaf_name(&verified.pins().admission_sha256)?,
            &[
                ("admission.json", verified.admission_bytes()),
                ("bars.ndjson", verified.candidate().bars_ndjson()),
                ("manifest.json", verified.candidate().manifest_bytes()),
                (
                    "normalization.json",
                    verified.candidate().normalization_bytes(),
                ),
            ],
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, verified);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

pub(super) fn publish_verified_continuation(
    root: &DailyAdmissionRoot,
    verified: &super::continuation::VerifiedDailyContinuation<'_, '_, '_, '_>,
) -> Result<(), DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        verified.candidate().revalidate()?;
        if !root
            .inner
            .independent_from(&verified.candidate().files.root)
        {
            return Err(DailyArtifactError::InvalidRoot);
        }
        linux::publish(
            &root.inner,
            &leaf_name(&verified.pins().admission_sha256)?,
            &[
                ("admission.json", verified.admission_bytes()),
                ("bars.ndjson", verified.candidate().bars_ndjson()),
                ("manifest.json", verified.candidate().manifest_bytes()),
                (
                    "normalization.json",
                    verified.candidate().normalization_bytes(),
                ),
            ],
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, verified);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

/// Unapproved producer copy; only an opaque reconstructed continuation is accepted.
pub fn write_daily_continuation_candidate(
    root: &DailyCandidateRoot,
    candidate: &super::continuation::DailyContinuationCandidate<'_, '_, '_>,
) -> Result<OpenedDailyCandidate, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        candidate
            .parent()
            .revalidate()
            .map_err(|_| DailyArtifactError::SourceChanged)?;
        check_bytes(
            candidate.content_sha256(),
            candidate.manifest_bytes(),
            candidate.normalization_bytes(),
            candidate.bars_ndjson(),
        )?;
        linux::publish(
            &root.inner,
            &leaf_name(candidate.content_sha256())?,
            &[
                ("bars.ndjson", candidate.bars_ndjson()),
                ("manifest.json", candidate.manifest_bytes()),
                ("normalization.json", candidate.normalization_bytes()),
            ],
        )?;
        let opened = read_daily_candidate(root, candidate.content_sha256())?;
        opened.matches_continuation_reconstruction(candidate)?;
        candidate
            .parent()
            .revalidate()
            .map_err(|_| DailyArtifactError::SourceChanged)?;
        Ok(opened)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, candidate);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

/// Copies into fresh inodes, seals, syncs and atomically publishes without replace.
/// An existing exact leaf is replayed. A differing or damaged leaf is never replaced.
pub fn write_daily_candidate(
    root: &DailyCandidateRoot,
    candidate: &HistoricalDailyCandidate<'_, '_, '_, '_>,
) -> Result<OpenedDailyCandidate, DailyArtifactError> {
    #[cfg(target_os = "linux")]
    {
        check_bytes(
            candidate.content_sha256(),
            candidate.manifest_bytes(),
            candidate.normalization_bytes(),
            candidate.bars_ndjson(),
        )?;
        linux::publish(
            &root.inner,
            &leaf_name(candidate.content_sha256())?,
            &[
                ("bars.ndjson", candidate.bars_ndjson()),
                ("manifest.json", candidate.manifest_bytes()),
                ("normalization.json", candidate.normalization_bytes()),
            ],
        )?;
        let opened = read_daily_candidate(root, candidate.content_sha256())?;
        opened.matches_reconstruction(candidate)?;
        Ok(opened)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, candidate);
        Err(DailyArtifactError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn leaf_name(hash: &ContentHash) -> Result<String, DailyArtifactError> {
    let value = hash
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(DailyArtifactError::UnsafePath)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(DailyArtifactError::UnsafePath);
    }
    Ok(value.into())
}

#[cfg(target_os = "linux")]
fn check_bytes(
    content: &ContentHash,
    manifest_bytes: &[u8],
    normalization_bytes: &[u8],
    bars: &[u8],
) -> Result<(DailyPriceManifestV1Wire, DailyNormalizationProofV1Wire), DailyArtifactError> {
    let limits = fixed_limits();
    if manifest_bytes.len() as u64 > limits.max_manifest_bytes
        || normalization_bytes.len() as u64 > limits.max_normalization_bytes
        || bars.len() as u64 > limits.max_bars_bytes
    {
        return Err(DailyArtifactError::LimitExceeded);
    }
    let manifest = parse_daily_price_manifest_v1(manifest_bytes)
        .map_err(|_| DailyArtifactError::InvalidArtifact)?;
    let proof = parse_daily_normalization_proof_v1(normalization_bytes)
        .map_err(|_| DailyArtifactError::InvalidArtifact)?;
    if &manifest.content_sha256 != content
        || encode_daily_price_manifest_v1(&manifest)
            .map_err(|_| DailyArtifactError::InvalidArtifact)?
            != manifest_bytes
        || encode_daily_normalization_proof_v1(&proof)
            .map_err(|_| DailyArtifactError::InvalidArtifact)?
            != normalization_bytes
        || manifest.bars.size_bytes != bars.len() as u64
        || manifest.bars.sha256 != ContentHash::from_bytes(bars)
        || manifest.normalization.size_bytes != normalization_bytes.len() as u64
        || manifest.normalization.sha256 != ContentHash::from_bytes(normalization_bytes)
        || manifest.normalization.row_count != proof.documents.len() as u64
    {
        return Err(DailyArtifactError::InvalidArtifact);
    }
    // Cross-source and exact price semantics are independently reconstructed by
    // the verifier; this reader deliberately yields no approved capability.
    Ok((manifest, proof))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::DailyArtifactError as Error;
    use super::fixed_limits;
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RawDir, RenameFlags, Stat};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileExt;
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    const FILES: [&str; 3] = ["bars.ndjson", "manifest.json", "normalization.json"];
    const ADMITTED_FILES: [&str; 4] = [
        "admission.json",
        "bars.ndjson",
        "manifest.json",
        "normalization.json",
    ];
    const FRONTIER_FILES: [&str; 2] = ["frontier.json", "observation.json"];
    #[derive(Clone, Copy)]
    enum Layout {
        Candidate,
        Admitted,
        Frontier,
    }
    impl Layout {
        fn files(self) -> &'static [&'static str] {
            match self {
                Self::Candidate => &FILES,
                Self::Admitted => &ADMITTED_FILES,
                Self::Frontier => &FRONTIER_FILES,
            }
        }
    }
    const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Identity {
        dev: u64,
        ino: u64,
        mode: u32,
        uid: u32,
        gid: u32,
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Snapshot {
        identity: Identity,
        links: u64,
        size: i64,
        mtime: i64,
        mtime_ns: i64,
        ctime: i64,
        ctime_ns: i64,
    }
    fn identity(s: &Stat) -> Identity {
        Identity {
            dev: s.st_dev,
            ino: s.st_ino,
            mode: s.st_mode,
            uid: s.st_uid,
            gid: s.st_gid,
        }
    }
    fn snapshot(s: &Stat) -> Snapshot {
        Snapshot {
            identity: identity(s),
            links: s.st_nlink,
            size: s.st_size,
            mtime: s.st_mtime,
            mtime_ns: s.st_mtime_nsec as i64,
            ctime: s.st_ctime,
            ctime_ns: s.st_ctime_nsec as i64,
        }
    }
    fn io(_: rustix::io::Errno) -> Error {
        Error::Io
    }

    fn no_acl(fd: &impl AsFd, directory: bool) -> Result<(), Error> {
        for name in ["system.posix_acl_access", "system.posix_acl_default"]
            .into_iter()
            .take(if directory { 2 } else { 1 })
        {
            let mut byte = [0u8; 1];
            match fs::fgetxattr(fd, name, byte.as_mut_slice()) {
                Err(rustix::io::Errno::NODATA) => {}
                _ => return Err(Error::UnsafePath),
            }
        }
        Ok(())
    }

    fn components(path: &Path) -> Result<Vec<Vec<u8>>, Error> {
        let bytes = path.as_os_str().as_bytes();
        if bytes.len() < 2 || bytes.len() > 4096 || bytes[0] != b'/' {
            return Err(Error::InvalidRoot);
        }
        let parts = bytes[1..].split(|b| *b == b'/').collect::<Vec<_>>();
        if parts.len() > 64
            || parts
                .iter()
                .any(|p| p.is_empty() || *p == b"." || *p == b".." || p.contains(&0))
        {
            return Err(Error::InvalidRoot);
        }
        Ok(parts.into_iter().map(Vec::from).collect())
    }

    pub(super) struct Root {
        dirs: Vec<OwnedFd>,
        names: Vec<Vec<u8>>,
        identities: Vec<Identity>,
        owner: u32,
        group: u32,
        layout: Layout,
    }
    impl Root {
        pub(super) fn open(path: &Path, owner: u32, group: u32) -> Result<Self, Error> {
            Self::open_with_layout(path, owner, group, Layout::Candidate)
        }
        pub(super) fn open_admitted(path: &Path, owner: u32, group: u32) -> Result<Self, Error> {
            Self::open_with_layout(path, owner, group, Layout::Admitted)
        }
        pub(super) fn open_frontier(path: &Path, owner: u32, group: u32) -> Result<Self, Error> {
            Self::open_with_layout(path, owner, group, Layout::Frontier)
        }
        pub(super) fn state_for_admission(&self, admitted: &Self) -> bool {
            matches!(self.layout, Layout::Frontier)
                && matches!(admitted.layout, Layout::Admitted)
                && self.owner == admitted.owner
                && self.identities.last() != admitted.identities.last()
        }
        fn open_with_layout(
            path: &Path,
            owner: u32,
            group: u32,
            layout: Layout,
        ) -> Result<Self, Error> {
            if owner == u32::MAX || group == u32::MAX {
                return Err(Error::InvalidRoot);
            }
            let parts = components(path)?;
            let slash = fs::open("/", DIRECTORY_FLAGS, Mode::empty()).map_err(io)?;
            let slash_stat = fs::fstat(&slash).map_err(io)?;
            if slash_stat.st_uid != 0 || slash_stat.st_mode & 0o022 != 0 {
                return Err(Error::UnsafePath);
            }
            no_acl(&slash, true)?;
            let mut dirs = vec![slash];
            let mut names = Vec::new();
            let mut identities = Vec::new();
            for name in parts {
                let parent = dirs.last().ok_or(Error::InvalidRoot)?;
                let fd = fs::openat(parent, name.as_slice(), DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|_| Error::UnsafePath)?;
                let stat = fs::fstat(&fd).map_err(io)?;
                if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
                    || (stat.st_uid != 0 && stat.st_uid != owner)
                    || stat.st_mode & 0o022 != 0
                {
                    return Err(Error::UnsafePath);
                }
                no_acl(&fd, true)?;
                let named =
                    fs::statat(parent, name.as_slice(), AtFlags::SYMLINK_NOFOLLOW).map_err(io)?;
                if identity(&named) != identity(&stat) {
                    return Err(Error::SourceChanged);
                }
                identities.push(identity(&stat));
                names.push(name);
                dirs.push(fd);
            }
            let last = identities.last().ok_or(Error::InvalidRoot)?;
            if last.uid != owner || last.gid != group || last.mode & 0o7777 != 0o750 {
                return Err(Error::InvalidRoot);
            }
            let root = Self {
                dirs,
                names,
                identities,
                owner,
                group,
                layout,
            };
            root.revalidate()?;
            Ok(root)
        }
        fn fd(&self) -> &OwnedFd {
            self.dirs.last().expect("validated nonempty root")
        }
        pub(super) fn independent_from(&self, candidate: &Self) -> bool {
            self.owner != candidate.owner && self.identities.last() != candidate.identities.last()
        }
        fn revalidate(&self) -> Result<(), Error> {
            let slash = fs::fstat(&self.dirs[0]).map_err(io)?;
            if slash.st_uid != 0 || slash.st_mode & 0o022 != 0 {
                return Err(Error::SourceChanged);
            }
            no_acl(&self.dirs[0], true)?;
            for (i, name) in self.names.iter().enumerate() {
                let held = fs::fstat(&self.dirs[i + 1]).map_err(io)?;
                let named = fs::statat(&self.dirs[i], name.as_slice(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| Error::SourceChanged)?;
                if identity(&held) != self.identities[i] || identity(&named) != self.identities[i] {
                    return Err(Error::SourceChanged);
                }
                no_acl(&self.dirs[i + 1], true)?;
            }
            Ok(())
        }
    }

    struct HeldFile {
        name: &'static str,
        file: File,
        stat: Snapshot,
        sha256: [u8; 32],
    }
    pub(super) struct OpenedLeaf {
        pub(super) root: Arc<Root>,
        name: String,
        directory: OwnedFd,
        stat: Snapshot,
        files: Vec<HeldFile>,
    }
    impl OpenedLeaf {
        pub(super) fn revalidate(&self) -> Result<(), Error> {
            self.root.revalidate()?;
            let named = fs::statat(
                self.root.fd(),
                self.name.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| Error::SourceChanged)?;
            if snapshot(&named) != self.stat
                || snapshot(&fs::fstat(&self.directory).map_err(io)?) != self.stat
            {
                return Err(Error::SourceChanged);
            }
            no_acl(&self.directory, true)?;
            exact_entries(&self.directory, self.root.layout.files())?;
            for file in &self.files {
                let held = fs::fstat(&file.file).map_err(io)?;
                let named = fs::statat(&self.directory, file.name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| Error::SourceChanged)?;
                if snapshot(&held) != file.stat || snapshot(&named) != file.stat {
                    return Err(Error::SourceChanged);
                }
                no_acl(&file.file, false)?;
                if descriptor_hash(&file.file, file.stat.size as u64)? != file.sha256
                    || snapshot(&fs::fstat(&file.file).map_err(io)?) != file.stat
                    || snapshot(
                        &fs::statat(&self.directory, file.name, AtFlags::SYMLINK_NOFOLLOW)
                            .map_err(|_| Error::SourceChanged)?,
                    ) != file.stat
                {
                    return Err(Error::SourceChanged);
                }
            }
            self.root.revalidate()
        }
    }

    fn exact_entries(directory: &OwnedFd, expected: &[&str]) -> Result<(), Error> {
        // A fresh directory description avoids sharing directory cursor state.
        let scan = fs::openat(directory, ".", DIRECTORY_FLAGS, Mode::empty()).map_err(io)?;
        if identity(&fs::fstat(&scan).map_err(io)?) != identity(&fs::fstat(directory).map_err(io)?)
        {
            return Err(Error::SourceChanged);
        }
        let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
        let mut iterator = RawDir::new(&scan, &mut buffer);
        let mut names = Vec::new();
        while let Some(entry) = iterator.next() {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if names.len() == expected.len()
                || !expected.iter().any(|file| file.as_bytes() == name)
                || names.iter().any(|old| old == name)
            {
                return Err(Error::UnexpectedEntries);
            }
            names.push(name.to_vec());
        }
        names.sort_unstable();
        if names
            .iter()
            .map(Vec::as_slice)
            .ne(expected.iter().map(|name| name.as_bytes()))
        {
            return Err(Error::UnexpectedEntries);
        }
        Ok(())
    }

    fn maximum(name: &str) -> u64 {
        let limits = fixed_limits();
        match name {
            "admission.json" | "frontier.json" => limits.max_admission_bytes,
            "observation.json" => limits.max_manifest_bytes,
            "bars.ndjson" => limits.max_bars_bytes,
            "manifest.json" => limits.max_manifest_bytes,
            _ => limits.max_normalization_bytes,
        }
    }
    fn descriptor_hash(file: &File, expected_size: u64) -> Result<[u8; 32], Error> {
        let mut hash = Sha256::new();
        let mut position = 0u64;
        let mut chunk = [0u8; 8192];
        loop {
            let read = file.read_at(&mut chunk, position).map_err(|_| Error::Io)?;
            if read == 0 {
                break;
            }
            position = position
                .checked_add(read as u64)
                .ok_or(Error::LimitExceeded)?;
            if position > expected_size {
                return Err(Error::SourceChanged);
            }
            hash.update(&chunk[..read]);
        }
        if position != expected_size {
            return Err(Error::SourceChanged);
        }
        Ok(hash.finalize().into())
    }

    fn read_file(
        directory: &OwnedFd,
        name: &'static str,
        root: &Root,
    ) -> Result<(HeldFile, Vec<u8>), Error> {
        let fd = fs::openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| Error::UnsafePath)?;
        let initial = fs::fstat(&fd).map_err(io)?;
        if FileType::from_raw_mode(initial.st_mode) != FileType::RegularFile
            || initial.st_uid != root.owner
            || initial.st_gid != root.group
            || initial.st_mode & 0o7777 != 0o440
            || initial.st_nlink != 1
        {
            return Err(Error::UnsafePath);
        }
        if initial.st_size < 0 || initial.st_size as u64 > maximum(name) {
            return Err(Error::LimitExceeded);
        }
        no_acl(&fd, false)?;
        let stat = snapshot(&initial);
        let mut file = File::from(fd);
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = file.read(&mut chunk).map_err(|_| Error::Io)?;
            if read == 0 {
                break;
            }
            let total = bytes.len().checked_add(read).ok_or(Error::LimitExceeded)?;
            if total as u64 > maximum(name) {
                return Err(Error::LimitExceeded);
            }
            bytes.try_reserve(read).map_err(|_| Error::LimitExceeded)?;
            bytes.extend_from_slice(&chunk[..read]);
        }
        let named = fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Error::SourceChanged)?;
        if bytes.len() as u64 != initial.st_size as u64
            || snapshot(&fs::fstat(&file).map_err(io)?) != stat
            || snapshot(&named) != stat
        {
            return Err(Error::SourceChanged);
        }
        let sha256 = Sha256::digest(&bytes).into();
        Ok((
            HeldFile {
                name,
                file,
                stat,
                sha256,
            },
            bytes,
        ))
    }

    fn open_leaf(root: &Arc<Root>, name: &str) -> Result<OpenedLeaf, Error> {
        root.revalidate()?;
        let directory = fs::openat(root.fd(), name, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| Error::UnsafePath)?;
        let stat = fs::fstat(&directory).map_err(io)?;
        if stat.st_uid != root.owner || stat.st_gid != root.group || stat.st_mode & 0o7777 != 0o750
        {
            return Err(Error::UnsafePath);
        }
        no_acl(&directory, true)?;
        exact_entries(&directory, root.layout.files())?;
        Ok(OpenedLeaf {
            root: Arc::clone(root),
            name: name.into(),
            directory,
            stat: snapshot(&stat),
            files: Vec::new(),
        })
    }

    type LeafBytes = BTreeMap<&'static str, Vec<u8>>;
    pub(super) fn read_leaf(
        root: &Arc<Root>,
        name: &str,
    ) -> Result<(OpenedLeaf, LeafBytes), Error> {
        let mut leaf = open_leaf(root, name)?;
        let mut bytes = BTreeMap::new();
        for &name in root.layout.files() {
            let (file, content) = read_file(&leaf.directory, name, root)?;
            leaf.files.push(file);
            bytes.insert(name, content);
        }
        leaf.revalidate()?;
        Ok((leaf, bytes))
    }

    /// Reads only navigation metadata under the same directory/file guards.
    /// The other three filenames are checked, but their bytes are deliberately
    /// not read here. This return value can never stand in for a full leaf.
    pub(super) fn read_admission_metadata(root: &Arc<Root>, name: &str) -> Result<Vec<u8>, Error> {
        if !matches!(root.layout, Layout::Admitted) {
            return Err(Error::InvalidRoot);
        }
        let mut leaf = open_leaf(root, name)?;
        let (file, bytes) = read_file(&leaf.directory, "admission.json", root)?;
        leaf.files.push(file);
        leaf.revalidate()?;
        Ok(bytes)
    }

    #[derive(PartialEq, Eq)]
    struct NameEntry {
        name: Vec<u8>,
        committed: Option<Snapshot>,
    }
    pub(super) struct NameCapture {
        root: Arc<Root>,
        max_entries: usize,
        root_snapshot: Snapshot,
        entries: Vec<NameEntry>,
    }
    impl NameCapture {
        pub(super) fn revalidate(&self) -> Result<(), Error> {
            let (entries, root_snapshot) = read_names(&self.root, self.max_entries)?;
            if entries != self.entries || root_snapshot != self.root_snapshot {
                return Err(Error::SourceChanged);
            }
            Ok(())
        }
    }

    fn pending_name(name: &[u8]) -> bool {
        let Some(tail) = name.strip_prefix(b".pending-") else {
            return false;
        };
        let Some(separator) = tail.iter().position(|byte| *byte == b'-') else {
            return false;
        };
        let (pid, sequence) = (&tail[..separator], &tail[separator + 1..]);
        if pid.is_empty()
            || pid.len() > 10
            || sequence.is_empty()
            || sequence.len() > 20
            || !pid.iter().all(u8::is_ascii_digit)
            || !sequence.iter().all(u8::is_ascii_digit)
        {
            return false;
        }
        let Ok(pid_value) = std::str::from_utf8(pid).unwrap_or("").parse::<u32>() else {
            return false;
        };
        let Ok(sequence_value) = std::str::from_utf8(sequence).unwrap_or("").parse::<u64>() else {
            return false;
        };
        pid_value != 0
            && pid_value.to_string().as_bytes() == pid
            && sequence_value.to_string().as_bytes() == sequence
    }

    fn read_names(
        root: &Arc<Root>,
        max_entries: usize,
    ) -> Result<(Vec<NameEntry>, Snapshot), Error> {
        if max_entries == 0 || max_entries as u64 > fixed_limits().max_sessions {
            return Err(Error::LimitExceeded);
        }
        root.revalidate()?;
        let before = snapshot(&fs::fstat(root.fd()).map_err(io)?);
        let scan = fs::openat(root.fd(), ".", DIRECTORY_FLAGS, Mode::empty()).map_err(io)?;
        if snapshot(&fs::fstat(&scan).map_err(io)?) != before {
            return Err(Error::SourceChanged);
        }
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
        let mut iterator = RawDir::new(&scan, &mut buffer);
        let mut entries: Vec<NameEntry> = Vec::new();
        while let Some(entry) = iterator.next() {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if entries.len() >= max_entries {
                return Err(Error::LimitExceeded);
            }
            let committed = if pending_name(name) {
                None
            } else {
                if name.len() != 64
                    || !name
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
                {
                    return Err(Error::UnexpectedEntries);
                }
                let named = fs::statat(root.fd(), name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| Error::SourceChanged)?;
                if FileType::from_raw_mode(named.st_mode) != FileType::Directory
                    || named.st_uid != root.owner
                    || named.st_gid != root.group
                    || named.st_mode & 0o7777 != 0o750
                {
                    return Err(Error::UnsafePath);
                }
                let directory = fs::openat(root.fd(), name, DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|_| Error::SourceChanged)?;
                if snapshot(&fs::fstat(&directory).map_err(io)?) != snapshot(&named) {
                    return Err(Error::SourceChanged);
                }
                no_acl(&directory, true)?;
                Some(snapshot(&named))
            };
            entries.try_reserve(1).map_err(|_| Error::LimitExceeded)?;
            entries.push(NameEntry {
                name: name.to_vec(),
                committed,
            });
        }
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        if entries.windows(2).any(|pair| pair[0].name == pair[1].name) {
            return Err(Error::UnexpectedEntries);
        }
        root.revalidate()?;
        if snapshot(&fs::fstat(root.fd()).map_err(io)?) != before {
            return Err(Error::SourceChanged);
        }
        Ok((entries, before))
    }

    pub(super) fn capture_names(
        root: &Arc<Root>,
        max_entries: usize,
    ) -> Result<(NameCapture, Vec<domain::ContentHash>), Error> {
        let (entries, root_snapshot) = read_names(root, max_entries)?;
        let mut names = Vec::new();
        names
            .try_reserve_exact(entries.len())
            .map_err(|_| Error::LimitExceeded)?;
        for entry in &entries {
            if entry.committed.is_some() {
                let name =
                    std::str::from_utf8(&entry.name).map_err(|_| Error::UnexpectedEntries)?;
                names.push(
                    domain::ContentHash::parse(&format!("sha256:{name}"))
                        .map_err(|_| Error::UnexpectedEntries)?,
                );
            }
        }
        let captured = NameCapture {
            root: Arc::clone(root),
            max_entries,
            root_snapshot,
            entries,
        };
        captured.revalidate()?;
        Ok((captured, names))
    }

    pub(super) fn read_frontier_metadata(root: &Arc<Root>, name: &str) -> Result<Vec<u8>, Error> {
        if !matches!(root.layout, Layout::Frontier) {
            return Err(Error::InvalidRoot);
        }
        let mut leaf = open_leaf(root, name)?;
        let (file, bytes) = read_file(&leaf.directory, "frontier.json", root)?;
        leaf.files.push(file);
        leaf.revalidate()?;
        Ok(bytes)
    }

    struct Staging {
        name: String,
        fd: OwnedFd,
        identity: Identity,
        files: Vec<(&'static str, File)>,
    }
    fn cleanup(root: &Root, staging: &Staging) -> Result<(), Error> {
        let named = fs::statat(root.fd(), staging.name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| Error::CleanupFailed)?;
        if identity(&named) != staging.identity
            || identity(&fs::fstat(&staging.fd).map_err(|_| Error::CleanupFailed)?)
                != staging.identity
        {
            return Err(Error::CleanupFailed);
        }
        for (name, file) in &staging.files {
            let held = fs::fstat(file).map_err(|_| Error::CleanupFailed)?;
            let named = fs::statat(&staging.fd, *name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| Error::CleanupFailed)?;
            if snapshot(&held) != snapshot(&named) {
                return Err(Error::CleanupFailed);
            }
        }
        for (name, _) in &staging.files {
            fs::unlinkat(&staging.fd, *name, AtFlags::empty()).map_err(|_| Error::CleanupFailed)?;
        }
        // rmdir refuses an unexpected remaining entry; no recursive deletion.
        fs::unlinkat(root.fd(), staging.name.as_str(), AtFlags::REMOVEDIR)
            .map_err(|_| Error::CleanupFailed)?;
        fs::fsync(root.fd()).map_err(|_| Error::CleanupFailed)
    }

    pub(super) fn publish(
        root: &Arc<Root>,
        destination: &str,
        files: &[(&'static str, &[u8])],
    ) -> Result<(), Error> {
        root.revalidate()?;
        if rustix::process::geteuid().as_raw() != root.owner {
            return Err(Error::InvalidRoot);
        }
        if files
            .iter()
            .map(|(name, _)| *name)
            .ne(root.layout.files().iter().copied())
            || files
                .iter()
                .any(|(name, bytes)| bytes.len() as u64 > maximum(name))
        {
            return Err(Error::LimitExceeded);
        }
        let staging = (0..32)
            .find_map(|_| {
                let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let name = format!(".pending-{}-{sequence}", std::process::id());
                match fs::mkdirat(root.fd(), name.as_str(), Mode::from_raw_mode(0o700)) {
                    Ok(()) => Some(Ok(name)),
                    Err(rustix::io::Errno::EXIST) => None,
                    Err(_) => Some(Err(Error::Io)),
                }
            })
            .ok_or(Error::Conflict)??;
        let fd =
            fs::openat(root.fd(), staging.as_str(), DIRECTORY_FLAGS, Mode::empty()).map_err(io)?;
        let mut staging = Staging {
            name: staging,
            identity: identity(&fs::fstat(&fd).map_err(io)?),
            fd,
            files: Vec::new(),
        };
        let prepared = (|| {
            fs::fchown(
                &staging.fd,
                None,
                Some(rustix::process::Gid::from_raw(root.group)),
            )
            .map_err(io)?;
            staging.identity = identity(&fs::fstat(&staging.fd).map_err(io)?);
            no_acl(&staging.fd, true)?;
            for (name, bytes) in files {
                let fd = fs::openat(
                    &staging.fd,
                    *name,
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(0o600),
                )
                .map_err(io)?;
                staging.files.push((*name, File::from(fd)));
                let file = &mut staging.files.last_mut().ok_or(Error::Io)?.1;
                file.write_all(bytes).map_err(|_| Error::Io)?;
                fs::fchown(
                    &*file,
                    None,
                    Some(rustix::process::Gid::from_raw(root.group)),
                )
                .map_err(io)?;
                fs::fchmod(&*file, Mode::from_raw_mode(0o440)).map_err(io)?;
                no_acl(file, false)?;
                file.sync_all().map_err(|_| Error::Io)?;
            }
            fs::fchmod(&staging.fd, Mode::from_raw_mode(0o750)).map_err(io)?;
            staging.identity = identity(&fs::fstat(&staging.fd).map_err(io)?);
            fs::fsync(&staging.fd).map_err(io)?;
            let (opened, actual) = read_leaf(root, &staging.name)?;
            if files
                .iter()
                .any(|(name, bytes)| actual.get(name).map(Vec::as_slice) != Some(*bytes))
            {
                return Err(Error::Conflict);
            }
            opened.revalidate()?;
            root.revalidate()?;
            Ok(())
        })();
        if let Err(error) = prepared {
            cleanup(root, &staging)?;
            return Err(error);
        }
        match fs::renameat_with(
            root.fd(),
            staging.name.as_str(),
            root.fd(),
            destination,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                fs::fsync(root.fd()).map_err(|_| Error::DurabilityUnknown)?;
                let (opened, actual) = read_leaf(root, destination)?;
                if files
                    .iter()
                    .any(|(name, bytes)| actual.get(name).map(Vec::as_slice) != Some(*bytes))
                {
                    return Err(Error::Conflict);
                }
                opened.revalidate()
            }
            Err(rustix::io::Errno::EXIST) => {
                cleanup(root, &staging)?;
                let (opened, actual) = read_leaf(root, destination)?;
                if files
                    .iter()
                    .any(|(name, bytes)| actual.get(name).map(Vec::as_slice) != Some(*bytes))
                {
                    return Err(Error::Conflict);
                }
                opened.revalidate()
            }
            Err(error) => {
                cleanup(root, &staging)?;
                Err(io(error))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        use std::sync::Barrier;
        use tempfile::{Builder, TempDir};

        const NAME: &str = "0000000000000000000000000000000000000000000000000000000000000000";
        const BYTES: [(&str, &[u8]); 3] = [
            ("bars.ndjson", b"opaque fixture bars\n"),
            ("manifest.json", b"opaque fixture manifest"),
            ("normalization.json", b"opaque fixture normalization"),
        ];

        struct Fixture {
            directory: TempDir,
            path: std::path::PathBuf,
            root: Arc<Root>,
        }
        impl Fixture {
            fn new() -> Self {
                // Hosted CI supplies a dedicated root-owned ancestor tree.
                // Local runs may use HOME, which must pass the identical guards.
                // Never bypass ancestry/ACL checks or fall back to /tmp.
                let fixture_parent = std::env::var_os("LAGRANGE_DAILY_ARTIFACT_TEST_ROOT")
                    .unwrap_or_else(|| std::env::var_os("HOME").expect("fixture home is required"));
                let directory = Builder::new()
                    .prefix(".daily-artifact-")
                    .tempdir_in(std::path::PathBuf::from(fixture_parent))
                    .unwrap();
                let path = directory.path().join("candidates");
                std::fs::create_dir(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
                let owner = rustix::process::geteuid().as_raw();
                let group = rustix::process::getegid().as_raw();
                let root = Arc::new(Root::open(&path, owner, group).unwrap());
                Self {
                    directory,
                    path,
                    root,
                }
            }
            fn publish(&self) {
                publish(&self.root, NAME, &BYTES).unwrap();
            }
            fn leaf(&self) -> std::path::PathBuf {
                self.path.join(NAME)
            }
        }

        fn frontier_fixture(fixture: &Fixture) -> Arc<Root> {
            let path = fixture.directory.path().join("frontiers");
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
            Arc::new(Root::open_frontier(&path, fixture.root.owner, fixture.root.group).unwrap())
        }
        #[test]
        fn s5e_frontier_storage_is_an_exact_separate_two_file_tree() {
            let fixture = Fixture::new();
            let root = frontier_fixture(&fixture);
            let bytes = [
                ("frontier.json", b"private record".as_slice()),
                ("observation.json", b"private observation".as_slice()),
            ];
            publish(&root, NAME, &bytes).unwrap();
            let (opened, actual) = read_leaf(&root, NAME).unwrap();
            assert_eq!(actual.len(), 2);
            assert_eq!(actual["frontier.json"], bytes[0].1);
            assert_eq!(actual["observation.json"], bytes[1].1);
            opened.revalidate().unwrap();
            let path = fixture.directory.path().join("frontiers");
            let candidate_layout = Arc::new(Root::open(&path, root.owner, root.group).unwrap());
            let admission_layout =
                Arc::new(Root::open_admitted(&path, root.owner, root.group).unwrap());
            assert!(matches!(
                read_leaf(&candidate_layout, NAME),
                Err(Error::UnexpectedEntries)
            ));
            assert!(matches!(
                read_leaf(&admission_layout, NAME),
                Err(Error::UnexpectedEntries)
            ));
            assert_eq!(publish(&root, NAME, &BYTES), Err(Error::LimitExceeded));
        }
        #[test]
        fn s5e_frontier_root_binds_admitter_identity_and_rejects_alias_or_candidate() {
            let fixture = Fixture::new();
            let state = frontier_fixture(&fixture);
            let admission_path = fixture.directory.path().join("admitted");
            std::fs::create_dir(&admission_path).unwrap();
            std::fs::set_permissions(&admission_path, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            let mut admitted =
                Root::open_admitted(&admission_path, fixture.root.owner, fixture.root.group)
                    .unwrap();
            assert!(state.state_for_admission(&admitted));
            assert!(!state.state_for_admission(&fixture.root));
            let alias = Root::open_admitted(
                &fixture.directory.path().join("frontiers"),
                state.owner,
                state.group,
            )
            .unwrap();
            assert!(!state.state_for_admission(&alias));
            // Pure expected-principal mismatch; this is not a distinct-UID runtime fixture.
            admitted.owner = admitted.owner.checked_add(1).unwrap();
            assert!(!state.state_for_admission(&admitted));
        }
        #[test]
        fn s5e_frontier_exact_replay_keeps_inodes_and_changed_record_conflicts() {
            let fixture = Fixture::new();
            let root = frontier_fixture(&fixture);
            let bytes = [
                ("frontier.json", b"record".as_slice()),
                ("observation.json", b"observation".as_slice()),
            ];
            publish(&root, NAME, &bytes).unwrap();
            let path = fixture
                .directory
                .path()
                .join("frontiers")
                .join(NAME)
                .join("frontier.json");
            let inode = std::fs::metadata(&path).unwrap().ino();
            publish(&root, NAME, &bytes).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
            let changed = [("frontier.json", b"changed admission".as_slice()), bytes[1]];
            assert_eq!(publish(&root, NAME, &changed), Err(Error::Conflict));
            assert_eq!(std::fs::read(&path).unwrap(), bytes[0].1);
        }
        #[test]
        fn s5e_frontier_revalidation_rejects_changed_observation_or_missing_record() {
            let fixture = Fixture::new();
            let root = frontier_fixture(&fixture);
            let bytes = [
                ("frontier.json", b"record".as_slice()),
                ("observation.json", b"observation".as_slice()),
            ];
            publish(&root, NAME, &bytes).unwrap();
            let (opened, _) = read_leaf(&root, NAME).unwrap();
            let leaf = fixture.directory.path().join("frontiers").join(NAME);
            let observation = leaf.join("observation.json");
            std::fs::set_permissions(&observation, std::fs::Permissions::from_mode(0o640)).unwrap();
            std::fs::write(&observation, b"other-value").unwrap();
            std::fs::set_permissions(&observation, std::fs::Permissions::from_mode(0o440)).unwrap();
            assert_eq!(opened.revalidate(), Err(Error::SourceChanged));
            std::fs::remove_file(leaf.join("frontier.json")).unwrap();
            assert!(matches!(
                read_leaf(&root, NAME),
                Err(Error::UnexpectedEntries)
            ));
        }
        fn admitted_metadata_fixture(fixture: &Fixture) -> Arc<Root> {
            let root = Arc::new(
                Root::open_admitted(&fixture.path, fixture.root.owner, fixture.root.group).unwrap(),
            );
            let bytes = [
                ("admission.json", b"navigation bytes only".as_slice()),
                BYTES[0],
                BYTES[1],
                BYTES[2],
            ];
            publish(&root, NAME, &bytes).unwrap();
            root
        }

        #[test]
        fn i4d_metadata_reads_only_record_and_cannot_validate_other_files() {
            let fixture = Fixture::new();
            let root = admitted_metadata_fixture(&fixture);
            let body = fixture.leaf().join("bars.ndjson");
            std::fs::set_permissions(&body, std::fs::Permissions::from_mode(0o000)).unwrap();
            assert_eq!(
                read_admission_metadata(&root, NAME).unwrap(),
                b"navigation bytes only"
            );
            assert!(matches!(read_leaf(&root, NAME), Err(Error::UnsafePath)));
        }

        #[test]
        fn i4d_metadata_rejects_non_admission_layout_before_leaf_lookup() {
            let fixture = Fixture::new();
            assert!(matches!(
                read_admission_metadata(&fixture.root, NAME),
                Err(Error::InvalidRoot)
            ));
            let frontier = frontier_fixture(&fixture);
            assert!(matches!(
                read_admission_metadata(&frontier, NAME),
                Err(Error::InvalidRoot)
            ));
        }

        #[test]
        fn i4d_metadata_requires_exact_entries_and_safe_record() {
            for change in 0..3 {
                let fixture = Fixture::new();
                let root = admitted_metadata_fixture(&fixture);
                let record = fixture.leaf().join("admission.json");
                match change {
                    0 => {
                        std::fs::write(fixture.leaf().join("extra"), b"unexpected").unwrap();
                    }
                    1 => {
                        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o640))
                            .unwrap();
                    }
                    _ => {
                        std::fs::remove_file(&record).unwrap();
                        symlink("bars.ndjson", &record).unwrap();
                    }
                }
                let expected = if change == 0 {
                    Error::UnexpectedEntries
                } else {
                    Error::UnsafePath
                };
                assert!(
                    matches!(read_admission_metadata(&root, NAME), Err(error) if error == expected)
                );
            }
        }

        #[test]
        fn i4d_metadata_rejects_oversize_before_reading_payload() {
            let fixture = Fixture::new();
            let root = admitted_metadata_fixture(&fixture);
            let record = fixture.leaf().join("admission.json");
            std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o640)).unwrap();
            File::options()
                .write(true)
                .open(&record)
                .unwrap()
                .set_len(fixed_limits().max_admission_bytes + 1)
                .unwrap();
            std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o440)).unwrap();
            assert!(matches!(
                read_admission_metadata(&root, NAME),
                Err(Error::LimitExceeded)
            ));
        }

        #[test]
        fn s5c_storage_copies_seals_and_reopens_exact_three_files() {
            let fixture = Fixture::new();
            fixture.publish();
            let (opened, actual) = read_leaf(&fixture.root, NAME).unwrap();
            assert_eq!(actual.len(), 3);
            for (name, bytes) in BYTES {
                assert_eq!(actual[name], bytes);
                let metadata = std::fs::metadata(fixture.leaf().join(name)).unwrap();
                assert_eq!(metadata.mode() & 0o7777, 0o440);
                assert_eq!(metadata.uid(), fixture.root.owner);
                assert_eq!(metadata.gid(), fixture.root.group);
                assert_eq!(metadata.nlink(), 1);
            }
            assert_eq!(
                std::fs::metadata(fixture.leaf()).unwrap().mode() & 0o7777,
                0o750
            );
            opened.revalidate().unwrap();
            // Private storage fixtures cannot be mistaken for a valid D4 candidate.
            let public = super::super::DailyCandidateRoot {
                inner: Arc::clone(&fixture.root),
            };
            let hash = domain::ContentHash::parse(&format!("sha256:{NAME}")).unwrap();
            assert!(matches!(
                super::super::read_daily_candidate(&public, &hash),
                Err(Error::InvalidArtifact)
            ));
        }

        #[test]
        fn s5c_storage_replay_preserves_destination_inodes_and_bytes() {
            let fixture = Fixture::new();
            fixture.publish();
            let before = std::fs::metadata(fixture.leaf().join("bars.ndjson"))
                .unwrap()
                .ino();
            fixture.publish();
            assert_eq!(
                std::fs::metadata(fixture.leaf().join("bars.ndjson"))
                    .unwrap()
                    .ino(),
                before
            );
            let (opened, actual) = read_leaf(&fixture.root, NAME).unwrap();
            assert_eq!(actual["bars.ndjson"], BYTES[0].1);
            opened.revalidate().unwrap();
            assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 1);
        }

        #[test]
        fn s5c_storage_conflict_never_replaces_existing_leaf() {
            let fixture = Fixture::new();
            fixture.publish();
            let before = std::fs::metadata(fixture.leaf()).unwrap().ino();
            let mut changed = BYTES;
            changed[0].1 = b"changed bars";
            assert_eq!(publish(&fixture.root, NAME, &changed), Err(Error::Conflict));
            assert_eq!(std::fs::metadata(fixture.leaf()).unwrap().ino(), before);
            assert_eq!(
                std::fs::read(fixture.leaf().join("bars.ndjson")).unwrap(),
                BYTES[0].1
            );
            assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 1);
        }

        #[test]
        fn s5c_storage_rejects_relative_dot_empty_and_unbounded_root_paths() {
            for path in [
                "relative",
                "/",
                "//root",
                "/root/",
                "/root//leaf",
                "/root/./leaf",
                "/root/../leaf",
            ] {
                assert_eq!(components(Path::new(path)), Err(Error::InvalidRoot));
            }
            assert_eq!(
                components(Path::new(&format!("/{}", "a".repeat(4096)))),
                Err(Error::InvalidRoot)
            );
            assert_eq!(
                components(Path::new(&format!("/{}", vec!["a"; 65].join("/")))),
                Err(Error::InvalidRoot)
            );
            assert!(components(Path::new("/safe/root")).is_ok());
        }

        #[test]
        fn s5c_storage_root_requires_explicit_owner_group_and_private_permissions() {
            let fixture = Fixture::new();
            assert!(Root::open(&fixture.path, u32::MAX, fixture.root.group).is_err());
            assert!(Root::open(&fixture.path, fixture.root.owner, fixture.root.group + 1).is_err());
            std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o770))
                .unwrap();
            assert!(Root::open(&fixture.path, fixture.root.owner, fixture.root.group).is_err());
            assert_eq!(fixture.root.revalidate(), Err(Error::SourceChanged));
        }

        #[test]
        fn s5c_storage_rejects_symlink_ancestor_and_leaf_without_following() {
            let fixture = Fixture::new();
            fixture.publish();
            let alias = fixture.directory.path().join("alias");
            symlink(&fixture.path, &alias).unwrap();
            assert!(Root::open(&alias, fixture.root.owner, fixture.root.group).is_err());
            let moved = fixture.path.join("real");
            std::fs::rename(fixture.leaf(), &moved).unwrap();
            symlink(&moved, fixture.leaf()).unwrap();
            assert!(matches!(
                read_leaf(&fixture.root, NAME),
                Err(Error::UnsafePath)
            ));
            assert_eq!(
                std::fs::read(moved.join("bars.ndjson")).unwrap(),
                BYTES[0].1
            );
        }

        #[test]
        fn s5c_storage_rejects_symlink_or_hardlinked_regular_file() {
            for hard in [false, true] {
                let fixture = Fixture::new();
                fixture.publish();
                let path = fixture.leaf().join("bars.ndjson");
                let outside = fixture.directory.path().join("outside");
                std::fs::rename(&path, &outside).unwrap();
                if hard {
                    std::fs::hard_link(&outside, &path).unwrap();
                } else {
                    symlink(&outside, &path).unwrap();
                }
                assert!(matches!(
                    read_leaf(&fixture.root, NAME),
                    Err(Error::UnsafePath)
                ));
                assert_eq!(std::fs::read(&outside).unwrap(), BYTES[0].1);
            }
        }

        #[test]
        fn s5c_storage_rejects_missing_extra_and_directory_in_place_of_file() {
            for kind in 0..3 {
                let fixture = Fixture::new();
                fixture.publish();
                let path = fixture.leaf().join("bars.ndjson");
                match kind {
                    0 => std::fs::remove_file(&path).unwrap(),
                    1 => std::fs::write(fixture.leaf().join("extra"), b"must remain").unwrap(),
                    _ => {
                        std::fs::remove_file(&path).unwrap();
                        std::fs::create_dir(&path).unwrap();
                    }
                }
                assert!(read_leaf(&fixture.root, NAME).is_err());
            }
        }

        #[test]
        fn s5c_storage_rejects_writable_files_before_reading() {
            let fixture = Fixture::new();
            fixture.publish();
            std::fs::set_permissions(
                fixture.leaf().join("bars.ndjson"),
                std::fs::Permissions::from_mode(0o640),
            )
            .unwrap();
            assert!(matches!(
                read_leaf(&fixture.root, NAME),
                Err(Error::UnsafePath)
            ));
        }

        #[test]
        fn s5c_storage_rejects_oversized_sparse_file_before_allocation() {
            let fixture = Fixture::new();
            fixture.publish();
            let path = fixture.leaf().join("bars.ndjson");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_len(maximum("bars.ndjson") + 1).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
            assert!(matches!(
                read_leaf(&fixture.root, NAME),
                Err(Error::LimitExceeded)
            ));
        }

        #[test]
        fn s5c_storage_retained_descriptor_detects_in_place_mutation() {
            let fixture = Fixture::new();
            fixture.publish();
            let (opened, bytes) = read_leaf(&fixture.root, NAME).unwrap();
            let path = fixture.leaf().join("bars.ndjson");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            std::fs::write(&path, vec![b'x'; BYTES[0].1.len()]).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
            assert_eq!(opened.revalidate(), Err(Error::SourceChanged));
            assert_eq!(bytes["bars.ndjson"], BYTES[0].1);
        }

        #[test]
        fn s5c_storage_digest_detects_rewrite_even_with_matching_stat_snapshot() {
            let fixture = Fixture::new();
            fixture.publish();
            let (mut opened, _) = read_leaf(&fixture.root, NAME).unwrap();
            let path = fixture.leaf().join("bars.ndjson");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            std::fs::write(&path, vec![b'x'; BYTES[0].1.len()]).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
            // Model an undetectable timestamp change without changing the digest.
            // Production never refreshes this frozen snapshot.
            opened.files[0].stat = snapshot(&fs::fstat(&opened.files[0].file).unwrap());
            assert_eq!(opened.revalidate(), Err(Error::SourceChanged));
        }

        #[test]
        fn s5c_storage_admitted_layout_requires_exact_four_files() {
            let fixture = Fixture::new();
            let root = Arc::new(
                Root::open_admitted(&fixture.path, fixture.root.owner, fixture.root.group).unwrap(),
            );
            let files = [
                ("admission.json", b"opaque fixture admission".as_slice()),
                BYTES[0],
                BYTES[1],
                BYTES[2],
            ];
            assert_eq!(publish(&root, NAME, &BYTES), Err(Error::LimitExceeded));
            assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 0);
            publish(&root, NAME, &files).unwrap();
            let (opened, actual) = read_leaf(&root, NAME).unwrap();
            assert_eq!(actual.len(), 4);
            for (name, bytes) in files {
                assert_eq!(actual[name], bytes);
            }
            opened.revalidate().unwrap();
            assert!(matches!(
                read_leaf(&fixture.root, NAME),
                Err(Error::UnexpectedEntries)
            ));
            std::fs::remove_file(fixture.leaf().join("admission.json")).unwrap();
            assert!(matches!(
                read_leaf(&root, NAME),
                Err(Error::UnexpectedEntries)
            ));
        }

        #[test]
        fn s5c_storage_separate_paths_with_same_uid_do_not_establish_independence() {
            let candidate = Fixture::new();
            let admitted = Fixture::new();
            assert_ne!(
                candidate.root.identities.last(),
                admitted.root.identities.last()
            );
            assert!(!admitted.root.independent_from(&candidate.root));
            assert!(!candidate.root.independent_from(&candidate.root));
        }

        #[test]
        fn s5c_storage_retained_root_and_leaf_detect_replacement() {
            for root in [false, true] {
                let fixture = Fixture::new();
                fixture.publish();
                let (opened, _) = read_leaf(&fixture.root, NAME).unwrap();
                if root {
                    std::fs::rename(&fixture.path, fixture.directory.path().join("old-root"))
                        .unwrap();
                    std::fs::create_dir(&fixture.path).unwrap();
                } else {
                    std::fs::rename(fixture.leaf(), fixture.path.join("old-leaf")).unwrap();
                    std::fs::create_dir(fixture.leaf()).unwrap();
                }
                assert_eq!(opened.revalidate(), Err(Error::SourceChanged));
            }
        }

        #[test]
        fn s5c_storage_concurrent_exact_publish_converges_without_overwrite() {
            let fixture = Fixture::new();
            let barrier = Arc::new(Barrier::new(2));
            let joins = (0..2)
                .map(|_| {
                    let root = Arc::clone(&fixture.root);
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        publish(&root, NAME, &BYTES)
                    })
                })
                .collect::<Vec<_>>();
            for join in joins {
                assert_eq!(join.join().unwrap(), Ok(()));
            }
            let (opened, actual) = read_leaf(&fixture.root, NAME).unwrap();
            assert_eq!(actual["bars.ndjson"], BYTES[0].1);
            opened.revalidate().unwrap();
            assert_eq!(std::fs::read_dir(&fixture.path).unwrap().count(), 1);
        }

        #[test]
        fn s5c_storage_rejects_named_acl_even_when_mode_is_read_only() {
            let fixture = Fixture::new();
            fixture.publish();
            let path = fixture.leaf().join("bars.ndjson");
            let file = File::open(&path).unwrap();
            let mut acl = 2u32.to_le_bytes().to_vec();
            // Linux POSIX ACL xattr format: tag, permissions, qualifier.
            for (tag, permissions, qualifier) in [
                (1u16, 4u16, u32::MAX),
                (2, 4, fixture.root.owner + 1),
                (4, 4, u32::MAX),
                (16, 4, u32::MAX),
                (32, 0, u32::MAX),
            ] {
                acl.extend_from_slice(&tag.to_le_bytes());
                acl.extend_from_slice(&permissions.to_le_bytes());
                acl.extend_from_slice(&qualifier.to_le_bytes());
            }
            fs::fsetxattr(
                &file,
                "system.posix_acl_access",
                &acl,
                fs::XattrFlags::empty(),
            )
            .unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o440);
            assert_eq!(no_acl(&file, false), Err(Error::UnsafePath));
            assert!(matches!(
                read_leaf(&fixture.root, NAME),
                Err(Error::UnsafePath)
            ));
        }

        #[test]
        fn i4g_catalog_limits_are_explicit_and_bounded() {
            assert!(matches!(
                super::super::DailyArtifactNameLimits::new(0),
                Err(Error::LimitExceeded)
            ));
            assert!(matches!(
                super::super::DailyArtifactNameLimits::new(fixed_limits().max_sessions + 1),
                Err(Error::LimitExceeded)
            ));
            assert_eq!(
                super::super::DailyArtifactNameLimits::new(2)
                    .unwrap()
                    .max_entries(),
                2
            );
            let fixture = Fixture::new();
            assert!(matches!(
                capture_names(&fixture.root, 0),
                Err(Error::LimitExceeded)
            ));
        }

        #[test]
        fn i4g_catalog_empty_and_sorted_names_are_navigation_only() {
            let fixture = Fixture::new();
            let (empty, names) = capture_names(&fixture.root, 2).unwrap();
            assert!(names.is_empty());
            empty.revalidate().unwrap();
            for name in ["b".repeat(64), "1".repeat(64)] {
                // Deliberately no bundle files: discovery conveys no artifact proof.
                let path = fixture.path.join(name);
                std::fs::create_dir(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
            }
            assert!(matches!(empty.revalidate(), Err(Error::SourceChanged)));
            let (capture, names) = capture_names(&fixture.root, 2).unwrap();
            assert_eq!(names.len(), 2);
            assert_eq!(names[0].as_str(), format!("sha256:{}", "1".repeat(64)));
            assert_eq!(names[1].as_str(), format!("sha256:{}", "b".repeat(64)));
            capture.revalidate().unwrap();
            assert!(matches!(
                read_leaf(&fixture.root, &"1".repeat(64)),
                Err(Error::UnexpectedEntries)
            ));
        }

        #[test]
        fn i4g_catalog_staging_is_invisible_unopened_and_counted() {
            let fixture = Fixture::new();
            fixture.publish();
            let path = fixture.path.join(".pending-12-0");
            // Even a symlink in this reserved namespace is never followed.
            symlink("/nonexistent-pending-target", &path).unwrap();
            assert!(matches!(
                capture_names(&fixture.root, 1),
                Err(Error::LimitExceeded)
            ));
            let (capture, names) = capture_names(&fixture.root, 2).unwrap();
            assert_eq!(names.len(), 1);
            assert_eq!(names[0].as_str(), format!("sha256:{NAME}"));
            capture.revalidate().unwrap();
            assert!(
                std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            std::fs::remove_file(&path).unwrap();
            assert!(matches!(capture.revalidate(), Err(Error::SourceChanged)));
        }

        #[test]
        fn i4g_catalog_unknown_names_and_malformed_staging_fail_closed() {
            for name in [
                "latest",
                "README",
                ".pending-0-0",
                ".pending-01-0",
                ".pending-1-00",
                ".pending-1-1-extra",
                ".pending-4294967296-0",
                ".pending-1-18446744073709551616",
            ] {
                let fixture = Fixture::new();
                std::fs::write(fixture.path.join(name), b"unapproved").unwrap();
                assert!(
                    matches!(
                        capture_names(&fixture.root, 2),
                        Err(Error::UnexpectedEntries)
                    ),
                    "{name}"
                );
            }
            for name in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
                let fixture = Fixture::new();
                std::fs::create_dir(fixture.path.join(name)).unwrap();
                assert!(matches!(
                    capture_names(&fixture.root, 2),
                    Err(Error::UnexpectedEntries)
                ));
            }
        }

        #[test]
        fn i4g_catalog_committed_names_require_guarded_directories() {
            for change in 0..3 {
                let fixture = Fixture::new();
                let path = fixture.leaf();
                match change {
                    0 => std::fs::write(&path, b"not directory").unwrap(),
                    1 => symlink(fixture.directory.path(), &path).unwrap(),
                    _ => {
                        std::fs::create_dir(&path).unwrap();
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o770))
                            .unwrap();
                    }
                }
                assert!(matches!(
                    capture_names(&fixture.root, 2),
                    Err(Error::UnsafePath)
                ));
            }
        }

        #[test]
        fn i4g_catalog_detects_replacement_and_nested_name_mutation() {
            for change in 0..2 {
                let fixture = Fixture::new();
                fixture.publish();
                let (capture, _) = capture_names(&fixture.root, 3).unwrap();
                if change == 0 {
                    std::fs::rename(fixture.leaf(), fixture.directory.path().join("old")).unwrap();
                    std::fs::create_dir(fixture.leaf()).unwrap();
                    std::fs::set_permissions(
                        fixture.leaf(),
                        std::fs::Permissions::from_mode(0o750),
                    )
                    .unwrap();
                } else {
                    std::fs::write(fixture.leaf().join("extra"), b"not approved").unwrap();
                }
                assert!(matches!(capture.revalidate(), Err(Error::SourceChanged)));
            }
        }

        #[test]
        fn i4g_catalog_rejects_root_rebinding_after_capture() {
            let fixture = Fixture::new();
            let (capture, _) = capture_names(&fixture.root, 2).unwrap();
            std::fs::rename(&fixture.path, fixture.directory.path().join("moved")).unwrap();
            std::fs::create_dir(&fixture.path).unwrap();
            std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            assert!(matches!(capture.revalidate(), Err(Error::SourceChanged)));
        }

        #[test]
        fn i4g_catalog_public_roots_keep_distinct_layouts() {
            let fixture = Fixture::new();
            let owner = fixture.root.owner;
            let group = fixture.root.group;
            let frontier = frontier_fixture(&fixture);
            let limits = super::super::DailyArtifactNameLimits::new(2).unwrap();
            let candidate =
                super::super::DailyCandidateRoot::open(&fixture.path, owner, group).unwrap();
            let admission =
                super::super::DailyAdmissionRoot::open(&fixture.path, owner, group).unwrap();
            let state = super::super::DailySourceFrontierRoot::open(
                &fixture.directory.path().join("frontiers"),
                owner,
                group,
            )
            .unwrap();
            assert!(candidate.capture_names(limits).unwrap().names().is_empty());
            assert!(admission.capture_names(limits).unwrap().names().is_empty());
            state.capture_names(limits).unwrap().revalidate().unwrap();
            assert!(matches!(
                read_frontier_metadata(&fixture.root, NAME),
                Err(Error::InvalidRoot)
            ));
            assert!(matches!(
                read_admission_metadata(&frontier, NAME),
                Err(Error::InvalidRoot)
            ));
        }

        #[test]
        fn i4g_frontier_navigation_opens_only_record_with_full_record_guards() {
            let fixture = Fixture::new();
            let root = frontier_fixture(&fixture);
            let files = [
                ("frontier.json", b"unapproved navigation".as_slice()),
                ("observation.json", b"unapproved observation".as_slice()),
            ];
            publish(&root, NAME, &files).unwrap();
            let observation = fixture
                .directory
                .path()
                .join("frontiers")
                .join(NAME)
                .join("observation.json");
            std::fs::set_permissions(&observation, std::fs::Permissions::from_mode(0o000)).unwrap();
            assert_eq!(read_frontier_metadata(&root, NAME).unwrap(), files[0].1);
            assert!(matches!(read_leaf(&root, NAME), Err(Error::UnsafePath)));
            let record = observation.with_file_name("frontier.json");
            std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(matches!(
                read_frontier_metadata(&root, NAME),
                Err(Error::UnsafePath)
            ));
        }

        #[test]
        fn i4g_frontier_navigation_rejects_extras_symlinks_and_oversize() {
            for change in 0..3 {
                let fixture = Fixture::new();
                let root = frontier_fixture(&fixture);
                let files = [
                    ("frontier.json", b"unapproved navigation".as_slice()),
                    ("observation.json", b"unapproved observation".as_slice()),
                ];
                publish(&root, NAME, &files).unwrap();
                let leaf = fixture.directory.path().join("frontiers").join(NAME);
                let record = leaf.join("frontier.json");
                match change {
                    0 => std::fs::write(leaf.join("extra"), b"extra").unwrap(),
                    1 => {
                        std::fs::remove_file(&record).unwrap();
                        symlink("observation.json", &record).unwrap();
                    }
                    _ => {
                        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o640))
                            .unwrap();
                        File::options()
                            .write(true)
                            .open(&record)
                            .unwrap()
                            .set_len(fixed_limits().max_admission_bytes + 1)
                            .unwrap();
                        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o440))
                            .unwrap();
                    }
                }
                let expected = match change {
                    0 => Error::UnexpectedEntries,
                    1 => Error::UnsafePath,
                    _ => Error::LimitExceeded,
                };
                assert!(matches!(read_frontier_metadata(&root,NAME),Err(error) if error==expected));
            }
        }
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod unsupported_tests {
    use super::*;

    #[test]
    fn s5c_storage_non_linux_has_no_path_fallback() {
        for result in [
            DailyCandidateRoot::open(Path::new("/unused"), 1, 1).map(|_| ()),
            DailyAdmissionRoot::open(Path::new("/unused"), 1, 1).map(|_| ()),
        ] {
            assert_eq!(result, Err(DailyArtifactError::UnsupportedPlatform));
        }
    }
}
