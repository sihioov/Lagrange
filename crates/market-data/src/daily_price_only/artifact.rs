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

/// Scoped archive-only use of the ordinary typed history algorithms. Every
/// original and navigation FD is checked-close owned by the real invocation.
/// All outputs are provisional until both roots and the native scope settle.
#[cfg(target_os = "linux")]
pub(crate) fn with_archive_daily_roots(
    invocation: &crate::RawInvocation<'_>, expected: &crate::checked_close::Attempt,
    captured: &crate::checked_close::Attempt, admissions: &Path, frontiers: &Path,
    use_roots: impl FnOnce(&DailyAdmissionRoot, &DailySourceFrontierRoot) -> Result<(), DailyArtifactError>,
) -> Result<(), DailyArtifactError> {
    use crate::checked_close::Gap;
    struct Guard<'a> { expected: &'a crate::checked_close::Attempt, captured: &'a crate::checked_close::Attempt, armed: bool }
    impl Drop for Guard<'_> {
        fn drop(&mut self) { if self.armed { self.expected.gap(Gap::Abandoned); self.captured.gap(Gap::Abandoned); } }
    }
    let mut guard = Guard { expected, captured, armed: true };
    let check = || {
        let first = invocation.settle_local_in(expected, Ok::<(), std::convert::Infallible>(()));
        let second = invocation.settle_local_in(captured, Ok::<(), std::convert::Infallible>(()));
        let native = crate::close_gate::require_native(invocation, expected);
        if first.is_ok() && second.is_ok() && native.is_ok() { Ok(()) } else { Err(DailyArtifactError::SourceChanged) }
    };
    check()?;
    let mut admitted = None; let mut frontier = None;
    let body = (|| {
        admitted = Some(DailyAdmissionRoot { inner: std::sync::Arc::new(linux::Root::archive(admissions, false, expected)?) });
        frontier = Some(DailySourceFrontierRoot { inner: std::sync::Arc::new(linux::Root::archive(frontiers, true, expected)?) });
        use_roots(admitted.as_ref().expect("opened root"), frontier.as_ref().expect("opened root"))
    })();
    let mut clean = true;
    // Evaluate every original even if callback, parse or cancellation failed.
    if let Some(root) = &admitted { clean &= root.inner.revalidate().is_ok(); clean &= std::sync::Arc::strong_count(&root.inner) == 1; }
    if let Some(root) = &frontier { clean &= root.inner.revalidate().is_ok(); clean &= std::sync::Arc::strong_count(&root.inner) == 1; }
    drop(frontier); drop(admitted);
    let after = check();
    if body.is_err() || !clean || after.is_err() { return Err(body.err().unwrap_or(DailyArtifactError::SourceChanged)); }
    guard.armed = false;
    Ok(())
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
    bytes: std::collections::BTreeMap<&'static str, Vec<u8>>,
) -> Result<OpenedDailyCandidate, DailyArtifactError> {
    linux::candidate_from_files(content, files, bytes)
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
        linux::admission_files(&root.inner, admission, content)
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

    // The ordinary reader and archive reader use the same path/layout and
    // parsing algorithms. Only descriptor retirement differs. Archive owners
    // are adopted immediately, including scan descriptors and partial opens.
    struct DailyDescriptor {
        ordinary: Option<File>,
        checked: Option<crate::checked_close::Owner<File>>,
    }
    impl DailyDescriptor {
        fn new(fd: OwnedFd, attempt: Option<&crate::checked_close::Attempt>, role: crate::checked_close::Role) -> Self {
            let file = File::from(fd);
            match attempt {
                Some(attempt) => Self { ordinary: None, checked: Some(attempt.adopt(file, role)) },
                None => Self { ordinary: Some(file), checked: None },
            }
        }
        fn attempt(&self) -> Option<&crate::checked_close::Attempt> {
            self.checked.as_ref().map(crate::checked_close::Owner::attempt)
        }
    }
    impl std::ops::Deref for DailyDescriptor {
        type Target = File;
        fn deref(&self) -> &File {
            match &self.checked { Some(owner) => owner.get(), None => self.ordinary.as_ref().expect("held descriptor") }
        }
    }
    impl std::ops::DerefMut for DailyDescriptor {
        fn deref_mut(&mut self) -> &mut File {
            match &mut self.checked { Some(owner) => owner.get_mut(), None => self.ordinary.as_mut().expect("held descriptor") }
        }
    }
    impl AsFd for DailyDescriptor {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> { std::ops::Deref::deref(self).as_fd() }
    }
    impl Drop for DailyDescriptor {
        fn drop(&mut self) {
            if let Some(owner) = self.checked.take() { let _ = owner.finish(); }
        }
    }

    pub(super) struct Root {
        dirs: Vec<DailyDescriptor>,
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
            checked::legacy_root(path, owner, group, layout)
        }
        fn fd(&self) -> &DailyDescriptor {
            self.dirs.last().expect("validated nonempty root")
        }
        pub(super) fn independent_from(&self, candidate: &Self) -> bool {
            self.owner != candidate.owner && self.identities.last() != candidate.identities.last()
        }
        pub(super) fn revalidate(&self) -> Result<(), Error> {
            checked::legacy_revalidate_root(self)
        }
        pub(super) fn archive(path: &Path, frontier: bool, attempt: &crate::checked_close::Attempt) -> Result<Self, Error> {
            checked::archive_root(path, 10003, 10001,
                if frontier { Layout::Frontier } else { Layout::Admitted }, attempt)
        }
    }

    struct HeldFile {
        name: &'static str,
        file: DailyDescriptor,
        stat: Snapshot,
        sha256: [u8; 32],
    }
    pub(super) struct OpenedLeaf {
        pub(super) root: Arc<Root>,
        name: String,
        directory: DailyDescriptor,
        stat: Snapshot,
        files: Vec<HeldFile>,
    }
    impl OpenedLeaf {
        pub(super) fn revalidate(&self) -> Result<(), Error> {
            checked::legacy_revalidate_leaf(self)
        }
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

    fn read_file(
        directory: &DailyDescriptor,
        name: &'static str,
        root: &Root,
    ) -> Result<(HeldFile, Vec<u8>), Error> {
        checked::legacy_read_file(directory, name, root)
    }

    fn open_leaf(root: &Arc<Root>, name: &str) -> Result<OpenedLeaf, Error> {
        checked::legacy_open_leaf(root, name)
    }

    type LeafBytes = BTreeMap<&'static str, Vec<u8>>;
    pub(super) fn read_leaf(
        root: &Arc<Root>,
        name: &str,
    ) -> Result<(OpenedLeaf, LeafBytes), Error> {
        checked::legacy_read_leaf(root, name)
    }

    /// Reads only navigation metadata under the same directory/file guards.
    /// The other three filenames are checked, but their bytes are deliberately
    /// not read here. This return value can never stand in for a full leaf.
    pub(super) fn read_admission_metadata(root: &Arc<Root>, name: &str) -> Result<Vec<u8>, Error> {
        checked::legacy_metadata(root, name)
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
        let scan = DailyDescriptor::new(scan, root.fd().attempt(), crate::checked_close::Role::Enumeration);
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
                if root.fd().attempt().is_some() { return Err(Error::UnexpectedEntries); }
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
                let directory = DailyDescriptor::new(directory, root.fd().attempt(), crate::checked_close::Role::Child);
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
        if root.fd().attempt().is_some() || rustix::process::geteuid().as_raw() != root.owner {
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

    pub(super) fn candidate_from_files(
        content: &domain::ContentHash,
        files: OpenedLeaf,
        bytes: LeafBytes,
    ) -> Result<super::OpenedDailyCandidate, Error> {
        checked::legacy_candidate(content, files, bytes)
    }
    pub(super) fn admission_files(
        root: &Arc<Root>,
        admission: &domain::ContentHash,
        content: &domain::ContentHash,
    ) -> Result<(super::OpenedDailyCandidate, Vec<u8>), Error> {
        checked::legacy_admission(root, admission, content)
    }

    // C14 shared checked Daily read graph. Native algorithms below are shared
    // with the concrete Legacy wrappers; these holders confer no DB authority.
    pub(super) mod checked {
        use super::*;
        use crate::checked_close::{Attempt, Close, Failure, Gap, Owner, Report, Role};
        use crate::raw_custody::RawInvocation;
        use std::cell::RefCell;
        use std::convert::Infallible;
        use std::fmt;
        use std::os::fd::BorrowedFd;

        pub type DailyUnitResult = Result<(), Failure<Infallible>>;
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum DailyArtifactFirstKind {
            Scope,
            Binding,
            Artifact,
        }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum DailyReadPhase {
            Pending,
            Scan,
            File,
            Leaf,
            Root,
            Local,
        }
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub enum DailyReturnedStage {
            #[default]
            NotReturned,
            PartialFiles,
            Metadata,
            FullFiles,
            Parsed,
            Complete,
        }
        pub struct DailyUseFailure {
            expected: DailyUnitResult,
            captured: DailyUnitResult,
        }
        impl DailyUseFailure {
            pub fn expected_result(&self) -> &DailyUnitResult {
                &self.expected
            }
            pub fn captured_result(&self) -> &DailyUnitResult {
                &self.captured
            }
            pub fn source_report(&self) -> Option<&Report> {
                self.expected
                    .as_ref()
                    .err()
                    .or(self.captured.as_ref().err())
                    .map(Failure::report)
            }
        }
        enum First {
            Scope(DailyUseFailure),
            Binding(Failure<Infallible>),
            Artifact(Error),
            Execution {
                original: Option<Box<First>>,
                failure: crate::ExecutionGateFailure,
            },
        }
        impl First {
            fn kind(&self) -> Option<DailyArtifactFirstKind> {
                match self {
                    Self::Scope(_) => Some(DailyArtifactFirstKind::Scope),
                    Self::Binding(_) => Some(DailyArtifactFirstKind::Binding),
                    Self::Artifact(_) => Some(DailyArtifactFirstKind::Artifact),
                    Self::Execution { original, .. } => {
                        original.as_ref().and_then(|first| first.kind())
                    }
                }
            }
            fn report(&self) -> Option<&Report> {
                match self {
                    Self::Scope(f) => f.source_report(),
                    Self::Binding(f) => Some(f.report()),
                    Self::Artifact(_) => None,
                    Self::Execution { original, failure } => match original {
                        Some(first) => first.report(),
                        None => failure.source_report(),
                    },
                }
            }
            fn primary(&self) -> &Self {
                match self {
                    Self::Execution {
                        original: Some(first),
                        ..
                    } => first.primary(),
                    _ => self,
                }
            }
            fn execution(&self) -> Option<&crate::ExecutionGateFailure> {
                match self {
                    Self::Execution { failure, .. } => Some(failure),
                    _ => None,
                }
            }
        }
        enum Stop {
            First(First),
            Late,
        }
        pub struct DailyLocalCompletion {
            phase: DailyReadPhase,
            before: Option<DailyUseFailure>,
            expected: DailyUnitResult,
            captured: DailyUnitResult,
        }
        impl DailyLocalCompletion {
            pub fn phase(&self) -> DailyReadPhase {
                self.phase
            }
            pub fn before(&self) -> Option<&DailyUseFailure> {
                self.before.as_ref()
            }
            pub fn expected_result(&self) -> &DailyUnitResult {
                &self.expected
            }
            pub fn captured_result(&self) -> &DailyUnitResult {
                &self.captured
            }
            fn is_clean(&self) -> bool {
                self.before.is_none() && self.expected.is_ok() && self.captured.is_ok()
            }
        }
        #[derive(Default)]
        struct Trace {
            locals: Vec<DailyLocalCompletion>,
        }
        impl Trace {
            fn is_clean(&self) -> bool {
                self.locals.iter().all(DailyLocalCompletion::is_clean)
            }
        }

        fn compare_token(actual: &Attempt, other: &Attempt) {
            if !actual.same_scope(other) {
                actual.gap(Gap::ScopeMismatch);
                other.gap(Gap::ScopeMismatch);
            }
            let _ = other.check();
        }
        struct Scope<'inv, 'attempt> {
            invocation: &'inv RawInvocation<'attempt>,
            actual: Attempt,
            expected: Attempt,
            captured: Attempt,
        }
        impl<'inv, 'attempt> Scope<'inv, 'attempt> {
            fn new(
                invocation: &'inv RawInvocation<'attempt>,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Self {
                Self {
                    invocation,
                    actual: invocation.attempt().clone(),
                    expected: expected.clone(),
                    captured: captured.clone(),
                }
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            fn check<'a>(
                &self,
                expected: &Attempt,
                captured: &Attempt,
                owners: impl Iterator<Item = &'a Attempt>,
            ) -> Result<(), DailyUseFailure> {
                let actual = self.invocation.attempt();
                // Visit every token even when an earlier token or ledger is dirty.
                for other in [
                    expected,
                    captured,
                    &self.actual,
                    &self.expected,
                    &self.captured,
                ] {
                    compare_token(actual, other);
                }
                for owner in owners {
                    compare_token(actual, owner);
                }
                let first = self
                    .invocation
                    .settle_local_in(expected, Ok::<(), Infallible>(()));
                let second = self
                    .invocation
                    .settle_local_in(captured, Ok::<(), Infallible>(()));
                if first.is_ok() && second.is_ok() {
                    Ok(())
                } else {
                    Err(DailyUseFailure {
                        expected: first,
                        captured: second,
                    })
                }
            }
            fn local(
                &self,
                phase: DailyReadPhase,
                before: Option<DailyUseFailure>,
            ) -> DailyLocalCompletion {
                let expected = self
                    .invocation
                    .settle_local_in(&self.expected, Ok::<(), Infallible>(()));
                let captured = self
                    .invocation
                    .settle_local_in(&self.captured, Ok::<(), Infallible>(()));
                DailyLocalCompletion {
                    phase,
                    before,
                    expected,
                    captured,
                }
            }
            fn failed(&self) {
                if self.actual.check() {
                    self.actual.gap(Gap::Abandoned);
                }
            }
            fn adopt<R: Close>(&self, value: R, role: Role) -> Owner<R> {
                self.actual.adopt(value, role)
            }
        }
        struct Unwind {
            actual: Attempt,
            armed: bool,
        }
        impl Unwind {
            fn new(actual: &Attempt) -> Self {
                Self {
                    actual: actual.clone(),
                    armed: true,
                }
            }
            fn disarm(&mut self) {
                self.armed = false;
            }
        }
        impl Drop for Unwind {
            fn drop(&mut self) {
                if self.armed {
                    self.actual.gap(Gap::Abandoned);
                }
            }
        }
        // This same close/local seam is used for native File and ordinary Counted owners.
        fn retire_owned<R: Close>(
            scope: &Scope<'_, '_>,
            owner: Owner<R>,
            phase: DailyReadPhase,
        ) -> DailyLocalCompletion {
            let before = scope
                .check(
                    &scope.expected,
                    &scope.captured,
                    std::iter::once(owner.attempt()),
                )
                .err();
            let _aggregate_clean = owner.finish();
            scope.local(phase, before)
        }
        fn drain_owned<R: Close>(
            scope: &Scope<'_, '_>,
            owners: impl IntoIterator<Item = Owner<R>>,
            phase: DailyReadPhase,
            mut record: impl FnMut(DailyLocalCompletion),
        ) {
            for owner in owners {
                record(retire_owned(scope, owner, phase));
            }
        }
        fn after_retirement<E>(
            original: Result<(), E>,
            clean: bool,
            late: impl FnOnce() -> E,
        ) -> Result<(), E> {
            match original {
                Err(error) => Err(error),
                Ok(()) if clean => Ok(()),
                Ok(()) => Err(late()),
            }
        }
        fn retain_returned(map: &mut LeafBytes, name: &'static str, bytes: Vec<u8>) {
            map.insert(name, bytes);
        }

        #[derive(Default)]
        struct Data {
            stage: DailyReturnedStage,
            files: LeafBytes,
            admission: Option<Vec<u8>>,
            manifest: Option<Vec<u8>>,
            normalization: Option<Vec<u8>>,
            bars: Option<Vec<u8>>,
            parsed: Option<(
                super::super::DailyPriceManifestV1Wire,
                super::super::DailyNormalizationProofV1Wire,
            )>,
        }
        impl Data {
            fn file(&self, name: &str) -> Option<&[u8]> {
                match name {
                    "admission.json" => self.admission.as_deref(),
                    "manifest.json" => self.manifest.as_deref(),
                    "normalization.json" => self.normalization.as_deref(),
                    "bars.ndjson" => self.bars.as_deref(),
                    _ => None,
                }
                .or_else(|| self.files.get(name).map(Vec::as_slice))
            }
        }
        pub struct DailyRetainedData {
            inner: Data,
        }
        impl DailyRetainedData {
            pub fn stage(&self) -> DailyReturnedStage {
                self.inner.stage
            }
            /// Diagnostic returned bytes; not a newly admitted input.
            pub fn returned_file(&self, name: &str) -> Option<&[u8]> {
                self.inner.file(name)
            }
            pub fn parsed(
                &self,
            ) -> Option<(
                &super::super::DailyPriceManifestV1Wire,
                &super::super::DailyNormalizationProofV1Wire,
            )> {
                self.inner.parsed.as_ref().map(|(a, b)| (a, b))
            }
        }
        pub struct CheckedDailyArtifactFailure {
            original: Result<(), First>,
            retained: DailyRetainedData,
            trace: Trace,
        }
        impl CheckedDailyArtifactFailure {
            pub fn first_kind(&self) -> Option<DailyArtifactFirstKind> {
                self.original.as_ref().err().and_then(First::kind)
            }
            pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
                self.original.as_ref().err().and_then(First::execution)
            }
            pub fn artifact_error(&self) -> Option<&Error> {
                match self.original.as_ref().err().map(First::primary) {
                    Some(First::Artifact(e)) => Some(e),
                    _ => None,
                }
            }
            pub fn scope_failure(&self) -> Option<&DailyUseFailure> {
                match self.original.as_ref().err().map(First::primary) {
                    Some(First::Scope(e)) => Some(e),
                    _ => None,
                }
            }
            pub fn binding_failure(&self) -> Option<&Failure<Infallible>> {
                match self.original.as_ref().err().map(First::primary) {
                    Some(First::Binding(e)) => Some(e),
                    _ => None,
                }
            }
            pub fn source_report(&self) -> Option<&Report> {
                self.original.as_ref().err().and_then(First::report)
            }
            pub fn original_ok(&self) -> bool {
                self.original.is_ok()
            }
            pub fn retained(&self) -> &DailyRetainedData {
                &self.retained
            }
            pub fn local_completions(&self) -> &[DailyLocalCompletion] {
                &self.trace.locals
            }
        }
        pub struct CheckedDailyCompletion {
            retained: DailyRetainedData,
            trace: Trace,
        }
        impl CheckedDailyCompletion {
            pub fn retained(&self) -> &DailyRetainedData {
                &self.retained
            }
            pub fn local_completions(&self) -> &[DailyLocalCompletion] {
                &self.trace.locals
            }
            pub fn is_clean(&self) -> bool {
                self.trace.is_clean()
            }
        }
        fn failure(
            original: Result<(), Stop>,
            data: Data,
            trace: Trace,
        ) -> CheckedDailyArtifactFailure {
            let original = match original {
                Err(Stop::First(first)) => Err(first),
                Ok(()) | Err(Stop::Late) => Ok(()),
            };
            CheckedDailyArtifactFailure {
                original,
                retained: DailyRetainedData { inner: data },
                trace,
            }
        }

        struct CheckedRoot {
            dirs: Vec<Owner<File>>,
            names: Vec<Vec<u8>>,
            identities: Vec<Identity>,
            owner: u32,
            group: u32,
            layout: Layout,
        }
        struct CheckedHeld {
            name: &'static str,
            file: Owner<File>,
            stat: Snapshot,
            sha256: [u8; 32],
        }
        struct CheckedLeaf<'root> {
            root: &'root CheckedRoot,
            name: String,
            directory: Owner<File>,
            stat: Snapshot,
            files: Vec<CheckedHeld>,
        }
        pub struct CheckedDailyAdmissionRoot<'inv, 'attempt> {
            scope: Scope<'inv, 'attempt>,
            graph: Option<CheckedRoot>,
            trace: Trace,
        }
        pub struct CheckedDailyAdmissionMetadata<'root, 'inv, 'attempt> {
            root: &'root CheckedDailyAdmissionRoot<'inv, 'attempt>,
            retained: DailyRetainedData,
            trace: Trace,
        }
        pub struct CheckedDailyAdmissionFiles<'root, 'inv, 'attempt> {
            root: &'root CheckedDailyAdmissionRoot<'inv, 'attempt>,
            graph: Option<CheckedLeaf<'root>>,
            retained: DailyRetainedData,
            trace: Trace,
        }
        pub struct CheckedDailyAdmissionView<'a> {
            data: &'a Data,
        }
        impl CheckedDailyAdmissionView<'_> {
            pub fn manifest(&self) -> &super::super::DailyPriceManifestV1Wire {
                &self.data.parsed.as_ref().expect("parsed held leaf").0
            }
            pub fn normalization(&self) -> &super::super::DailyNormalizationProofV1Wire {
                &self.data.parsed.as_ref().expect("parsed held leaf").1
            }
            pub fn admission_bytes(&self) -> &[u8] {
                self.data.admission.as_deref().expect("held admission")
            }
            pub fn manifest_bytes(&self) -> &[u8] {
                self.data.manifest.as_deref().expect("held manifest")
            }
            pub fn normalization_bytes(&self) -> &[u8] {
                self.data
                    .normalization
                    .as_deref()
                    .expect("held normalization")
            }
            pub fn bars_ndjson(&self) -> &[u8] {
                self.data.bars.as_deref().expect("held bars")
            }
        }
        #[expect(
            clippy::result_large_err,
            reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
        )]
        fn admit<T>(
            scope: &Scope<'_, '_>,
            expected: &Attempt,
            captured: &Attempt,
            operation: impl FnOnce() -> Result<T, Stop>,
        ) -> Result<T, Stop> {
            let original = scope
                .check(expected, captured, std::iter::empty())
                .map_err(First::Scope);
            let execution = scope.invocation.compare_native_context_scopes(expected);
            crate::raw_custody::join_native_checks(original, execution, |original, failure| {
                First::Execution {
                    original: original.map(Box::new),
                    failure,
                }
            })
            .map_err(Stop::First)?;
            match crate::close_gate::require_native(scope.invocation, expected) {
                Ok(()) => {}
                Err(crate::close_gate::NativeAdmissionFailure::Unbound) => {
                    return Err(Stop::First(First::Binding(
                        scope.invocation.refused(Gap::Unbound),
                    )));
                }
                Err(crate::close_gate::NativeAdmissionFailure::Execution(failure)) => {
                    return Err(Stop::First(First::Execution {
                        original: None,
                        failure,
                    }));
                }
            }
            operation()
        }
        impl<'inv, 'attempt> CheckedDailyAdmissionRoot<'inv, 'attempt> {
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn open_in(
                invocation: &'inv RawInvocation<'attempt>,
                expected: &Attempt,
                captured: &Attempt,
                path: &Path,
                trusted_owner: u32,
                reader_group: u32,
            ) -> Result<Self, CheckedDailyArtifactFailure> {
                let scope = Scope::new(invocation, expected, captured);
                let mut unwind = Unwind::new(&scope.actual);
                let mode = CheckedMode::new(&scope, expected, captured);
                let result = admit(&scope, expected, captured, || {
                    open_root_with(&mode, path, trusted_owner, reader_group, Layout::Admitted)
                });
                if result.is_err() {
                    scope.failed();
                }
                mode.record_local(DailyReadPhase::Local);
                let result = match result {
                    Ok(root) if !mode.trace.borrow().is_clean() => {
                        mode.drain_root(root);
                        Err(Stop::Late)
                    }
                    other => other,
                };
                let trace = mode.trace.into_inner();
                unwind.disarm();
                match result {
                    Ok(graph) => Ok(Self {
                        scope,
                        graph: Some(graph),
                        trace,
                    }),
                    Err(e) => Err(failure(Err(e), Data::default(), trace)),
                }
            }
            #[expect(
                clippy::result_large_err,
                reason = "Both original scope failures remain whole, as in the existing checked view."
            )]
            pub(crate) fn check_scope_in(
                &self,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Result<(), DailyUseFailure> {
                self.scope.check(
                    expected,
                    captured,
                    self.graph().dirs.iter().map(Owner::attempt),
                )
            }
            fn graph(&self) -> &CheckedRoot {
                self.graph.as_ref().expect("held checked root")
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn revalidate_in(
                &self,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Result<(), CheckedDailyArtifactFailure> {
                let mut unwind = Unwind::new(&self.scope.actual);
                let mode = CheckedMode::new(&self.scope, expected, captured);
                let result = revalidate_root_with(&mode, self.graph());
                if result.is_err() {
                    self.scope.failed();
                }
                mode.record_local(DailyReadPhase::Local);
                unwind.disarm();
                let trace = mode.trace.into_inner();
                if result.is_ok() && trace.is_clean() {
                    Ok(())
                } else {
                    Err(failure(result, Data::default(), trace))
                }
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn read_metadata_in<'root>(
                &'root self,
                expected: &Attempt,
                captured: &Attempt,
                admission: &domain::ContentHash,
            ) -> Result<
                CheckedDailyAdmissionMetadata<'root, 'inv, 'attempt>,
                CheckedDailyArtifactFailure,
            > {
                let mut unwind = Unwind::new(&self.scope.actual);
                let mode = CheckedMode::new(&self.scope, expected, captured);
                let mut leaf = None;
                let mut data = Data::default();
                let original = (|| {
                    checkpoint(&mode, &self.graph().dirs, None, &[], None, None)?;
                    let name = super::super::leaf_name(admission).map_err(CheckedMode::policy)?;
                    metadata_with(&mode, self.graph(), &name, &mut leaf, &mut data)
                })();
                if original.is_err() {
                    self.scope.failed();
                }
                if let Some(leaf) = leaf.take() {
                    mode.drain_leaf(leaf);
                }
                mode.record_local(DailyReadPhase::Local);
                unwind.disarm();
                let trace = mode.trace.into_inner();
                if original.is_ok() && trace.is_clean() {
                    Ok(CheckedDailyAdmissionMetadata {
                        root: self,
                        retained: DailyRetainedData { inner: data },
                        trace,
                    })
                } else {
                    Err(failure(original, data, trace))
                }
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn read_files_in<'root>(
                &'root self,
                expected: &Attempt,
                captured: &Attempt,
                admission: &domain::ContentHash,
                content: &domain::ContentHash,
            ) -> Result<
                CheckedDailyAdmissionFiles<'root, 'inv, 'attempt>,
                CheckedDailyArtifactFailure,
            > {
                let mut unwind = Unwind::new(&self.scope.actual);
                let mode = CheckedMode::new(&self.scope, expected, captured);
                let mut leaf = None;
                let mut data = Data::default();
                let original = (|| {
                    checkpoint(&mode, &self.graph().dirs, None, &[], None, None)?;
                    let name = super::super::leaf_name(admission).map_err(CheckedMode::policy)?;
                    read_leaf_with(&mode, self.graph(), &name, &mut leaf, &mut data)?;
                    let held = leaf.as_ref().expect("returned full leaf");
                    check_leaf(&mode, held, None, None)?;
                    admission_data(admission, &mut data).map_err(CheckedMode::policy)?;
                    check_leaf(&mode, held, None, None)?;
                    candidate_data(content, &mut data).map_err(CheckedMode::policy)?;
                    revalidate_leaf_with(&mode, held)?;
                    data.stage = DailyReturnedStage::Complete;
                    Ok(())
                })();
                if original.is_err() {
                    self.scope.failed();
                }
                mode.record_local(DailyReadPhase::Local);
                let clean = original.is_ok() && mode.trace.borrow().is_clean();
                if !clean && let Some(leaf) = leaf.take() {
                    mode.drain_leaf(leaf);
                }
                unwind.disarm();
                let trace = mode.trace.into_inner();
                if clean {
                    Ok(CheckedDailyAdmissionFiles {
                        root: self,
                        graph: leaf,
                        retained: DailyRetainedData { inner: data },
                        trace,
                    })
                } else {
                    Err(failure(original, data, trace))
                }
            }
            pub fn finish_local(mut self) -> CheckedDailyCompletion {
                let mode =
                    CheckedMode::new(&self.scope, &self.scope.expected, &self.scope.captured);
                if let Some(root) = self.graph.take() {
                    mode.drain_root(root);
                }
                mode.record_local(DailyReadPhase::Local);
                CheckedDailyCompletion {
                    retained: DailyRetainedData {
                        inner: Data::default(),
                    },
                    trace: {
                        let mut trace = std::mem::take(&mut self.trace);
                        trace.locals.extend(mode.trace.into_inner().locals);
                        trace
                    },
                }
            }
        }
        impl Drop for CheckedDailyAdmissionRoot<'_, '_> {
            fn drop(&mut self) {
                if let Some(root) = self.graph.take() {
                    self.scope.actual.gap(Gap::Abandoned);
                    let mode =
                        CheckedMode::new(&self.scope, &self.scope.expected, &self.scope.captured);
                    mode.drain_root(root);
                }
            }
        }
        impl CheckedDailyAdmissionMetadata<'_, '_, '_> {
            pub(crate) fn into_completion(self) -> CheckedDailyCompletion {
                CheckedDailyCompletion {
                    retained: self.retained,
                    trace: self.trace,
                }
            }

            pub fn retained(&self) -> &DailyRetainedData {
                &self.retained
            }
            pub fn local_completions(&self) -> &[DailyLocalCompletion] {
                &self.trace.locals
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn bytes_in(
                &self,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Result<&[u8], DailyUseFailure> {
                self.root.scope.check(
                    expected,
                    captured,
                    self.root.graph().dirs.iter().map(Owner::attempt),
                )?;
                Ok(self
                    .retained
                    .inner
                    .admission
                    .as_deref()
                    .expect("returned metadata"))
            }
        }
        impl CheckedDailyAdmissionFiles<'_, '_, '_> {
            pub fn retained(&self) -> &DailyRetainedData {
                &self.retained
            }
            pub fn local_completions(&self) -> &[DailyLocalCompletion] {
                &self.trace.locals
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn view_in(
                &self,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Result<CheckedDailyAdmissionView<'_>, DailyUseFailure> {
                let leaf = self.graph.as_ref().expect("held leaf");
                self.root.scope.check(
                    expected,
                    captured,
                    self.root
                        .graph()
                        .dirs
                        .iter()
                        .map(Owner::attempt)
                        .chain(std::iter::once(leaf.directory.attempt()))
                        .chain(leaf.files.iter().map(|f| f.file.attempt())),
                )?;
                Ok(CheckedDailyAdmissionView {
                    data: &self.retained.inner,
                })
            }
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            pub fn revalidate_in(
                &self,
                expected: &Attempt,
                captured: &Attempt,
            ) -> Result<(), CheckedDailyArtifactFailure> {
                let mut unwind = Unwind::new(&self.root.scope.actual);
                let mode = CheckedMode::new(&self.root.scope, expected, captured);
                let original = revalidate_leaf_with(&mode, self.graph.as_ref().expect("held leaf"));
                if original.is_err() {
                    self.root.scope.failed();
                }
                mode.record_local(DailyReadPhase::Local);
                unwind.disarm();
                let trace = mode.trace.into_inner();
                if original.is_ok() && trace.is_clean() {
                    Ok(())
                } else {
                    Err(failure(original, Data::default(), trace))
                }
            }
            pub fn finish_local(mut self) -> CheckedDailyCompletion {
                let scope = &self.root.scope;
                let mode = CheckedMode::new(scope, &scope.expected, &scope.captured);
                if let Some(leaf) = self.graph.take() {
                    mode.drain_leaf(leaf);
                }
                mode.record_local(DailyReadPhase::Local);
                let mut trace = std::mem::take(&mut self.trace);
                trace.locals.extend(mode.trace.into_inner().locals);
                CheckedDailyCompletion {
                    retained: DailyRetainedData {
                        inner: std::mem::take(&mut self.retained.inner),
                    },
                    trace,
                }
            }
        }
        impl Drop for CheckedDailyAdmissionFiles<'_, '_, '_> {
            fn drop(&mut self) {
                if let Some(leaf) = self.graph.take() {
                    self.root.scope.actual.gap(Gap::Abandoned);
                    let mode = CheckedMode::new(
                        &self.root.scope,
                        &self.root.scope.expected,
                        &self.root.scope.captured,
                    );
                    mode.drain_leaf(leaf);
                }
            }
        }
        macro_rules! bounded_debug {
            ($($name:ty),+ $(,)?) => { $(impl fmt::Debug for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.debug_struct(stringify!($name)).finish_non_exhaustive()
                }
            })+ };
        }
        bounded_debug!(
            DailyUseFailure,
            DailyLocalCompletion,
            DailyRetainedData,
            CheckedDailyArtifactFailure,
            CheckedDailyCompletion,
            CheckedDailyAdmissionRoot<'_, '_>,
            CheckedDailyAdmissionMetadata<'_, '_, '_>,
            CheckedDailyAdmissionFiles<'_, '_, '_>,
            CheckedDailyAdmissionView<'_>
        );

        struct RootView<'a, D> {
            dirs: &'a [D],
            names: &'a [Vec<u8>],
            identities: &'a [Identity],
            owner: u32,
            group: u32,
            layout: Layout,
        }
        struct HeldView<'a, F> {
            name: &'static str,
            file: &'a F,
            stat: &'a Snapshot,
            sha256: &'a [u8; 32],
        }
        struct LeafView<'a, M: DailyReadMode> {
            root: &'a M::Root,
            name: &'a str,
            directory: &'a M::Dir,
            stat: &'a Snapshot,
            files: &'a [M::Held],
        }
        // Sealed by this private module. Existing concrete Legacy types stay concrete.
        trait DailyReadMode: Sized {
            type Error;
            type Dir: 'static;
            type File: 'static;
            type Held: 'static;
            type Root: 'static;
            type Link<'a>: Copy;
            type Leaf<'a>;
            fn policy(error: Error) -> Self::Error;
            fn late() -> Self::Error;
            fn failed(&self);
            fn check<'a>(
                &self,
                owners: impl Iterator<Item = &'a Attempt>,
            ) -> Result<(), Self::Error>;
            fn adopt(&self, fd: OwnedFd, role: Role) -> Self::Dir;
            fn promote(fd: Self::Dir) -> Self::File;
            fn dir(fd: &Self::Dir) -> BorrowedFd<'_>;
            fn file(fd: &Self::File) -> &File;
            fn file_mut(fd: &mut Self::File) -> &mut File;
            fn dir_attempt(fd: &Self::Dir) -> Option<&Attempt>;
            fn file_attempt(fd: &Self::File) -> Option<&Attempt>;
            fn retire_dir(&self, fd: Self::Dir, phase: DailyReadPhase) -> bool;
            fn retire_file(&self, fd: Self::File) -> bool;
            fn make_root(
                dirs: Vec<Self::Dir>,
                names: Vec<Vec<u8>>,
                identities: Vec<Identity>,
                owner: u32,
                group: u32,
                layout: Layout,
            ) -> Self::Root;
            fn root_view(root: &Self::Root) -> RootView<'_, Self::Dir>;
            fn root<'a>(link: Self::Link<'a>) -> &'a Self::Root;
            fn make_leaf<'a>(
                link: Self::Link<'a>,
                name: &str,
                directory: Self::Dir,
                stat: Stat,
            ) -> Self::Leaf<'a>;
            fn leaf_view<'a, 'r>(leaf: &'a Self::Leaf<'r>) -> LeafView<'a, Self>;
            fn files_mut<'a, 'r>(leaf: &'a mut Self::Leaf<'r>) -> &'a mut Vec<Self::Held>;
            fn make_held(
                name: &'static str,
                file: Self::File,
                stat: Snapshot,
                sha256: [u8; 32],
            ) -> Self::Held;
            fn held_view(held: &Self::Held) -> HeldView<'_, Self::File>;
            fn cleanup_partial_dirs(&self, dirs: &mut Vec<Self::Dir>);
            fn drain_dirs(&self, dirs: Vec<Self::Dir>);
            fn drain_root(&self, root: Self::Root);
            fn drain_leaf(&self, leaf: Self::Leaf<'_>);
        }
        pub(super) struct Legacy { attempt: Option<Attempt> }
        impl Legacy {
            fn ordinary() -> Self { Self { attempt: None } }
            fn for_root(root: &Root) -> Self {
                Self { attempt: root.dirs.first().and_then(DailyDescriptor::attempt).cloned() }
            }
        }
        impl DailyReadMode for Legacy {
            type Error = Error;
            type Dir = DailyDescriptor;
            type File = DailyDescriptor;
            type Held = HeldFile;
            type Root = Root;
            type Link<'a> = &'a Arc<Root>;
            type Leaf<'a> = OpenedLeaf;
            fn policy(error: Error) -> Error {
                error
            }
            fn late() -> Error { Error::SourceChanged }
            fn failed(&self) {
                if let Some(attempt) = &self.attempt { attempt.gap(Gap::Abandoned); }
            }
            fn check<'a>(&self, owners: impl Iterator<Item = &'a Attempt>) -> Result<(), Error> {
                let mut clean = true;
                for owner in owners {
                    match &self.attempt {
                        Some(attempt) if attempt.same_scope(owner) => {},
                        _ => { owner.gap(Gap::ScopeMismatch); self.failed(); clean = false; }
                    }
                    clean &= owner.check();
                }
                if let Some(attempt) = &self.attempt { clean &= attempt.check(); }
                if clean { Ok(()) } else { Err(Error::SourceChanged) }
            }
            fn adopt(&self, fd: OwnedFd, role: Role) -> DailyDescriptor {
                DailyDescriptor::new(fd, self.attempt.as_ref(), role)
            }
            fn promote(fd: DailyDescriptor) -> DailyDescriptor { fd }
            fn dir(fd: &DailyDescriptor) -> BorrowedFd<'_> { fd.as_fd() }
            fn file(fd: &DailyDescriptor) -> &File { fd }
            fn file_mut(fd: &mut DailyDescriptor) -> &mut File { fd }
            fn dir_attempt(fd: &DailyDescriptor) -> Option<&Attempt> { fd.attempt() }
            fn file_attempt(fd: &DailyDescriptor) -> Option<&Attempt> { fd.attempt() }
            fn retire_dir(&self, fd: DailyDescriptor, _: DailyReadPhase) -> bool {
                drop(fd); self.attempt.as_ref().is_none_or(Attempt::check)
            }
            fn retire_file(&self, file: DailyDescriptor) -> bool {
                drop(file); self.attempt.as_ref().is_none_or(Attempt::check)
            }
            fn make_root(
                dirs: Vec<DailyDescriptor>,
                names: Vec<Vec<u8>>,
                identities: Vec<Identity>,
                owner: u32,
                group: u32,
                layout: Layout,
            ) -> Root {
                Root {
                    dirs,
                    names,
                    identities,
                    owner,
                    group,
                    layout,
                }
            }
            fn root_view(root: &Root) -> RootView<'_, DailyDescriptor> {
                RootView {
                    dirs: &root.dirs,
                    names: &root.names,
                    identities: &root.identities,
                    owner: root.owner,
                    group: root.group,
                    layout: root.layout,
                }
            }
            fn root<'a>(link: Self::Link<'a>) -> &'a Self::Root {
                link
            }
            fn make_leaf<'a>(
                root: Self::Link<'a>,
                name: &str,
                directory: Self::Dir,
                stat: Stat,
            ) -> Self::Leaf<'a> {
                OpenedLeaf {
                    root: Arc::clone(root),
                    name: name.into(),
                    directory,
                    stat: snapshot(&stat),
                    files: Vec::new(),
                }
            }
            fn leaf_view<'a, 'r>(leaf: &'a OpenedLeaf) -> LeafView<'a, Self> {
                LeafView {
                    root: &leaf.root,
                    name: &leaf.name,
                    directory: &leaf.directory,
                    stat: &leaf.stat,
                    files: &leaf.files,
                }
            }
            fn files_mut<'r>(leaf: &mut OpenedLeaf) -> &mut Vec<HeldFile> {
                &mut leaf.files
            }
            fn make_held(
                name: &'static str,
                file: DailyDescriptor,
                stat: Snapshot,
                sha256: [u8; 32],
            ) -> HeldFile {
                HeldFile {
                    name,
                    file,
                    stat,
                    sha256,
                }
            }
            fn held_view(held: &HeldFile) -> HeldView<'_, DailyDescriptor> {
                HeldView {
                    name: held.name,
                    file: &held.file,
                    stat: &held.stat,
                    sha256: &held.sha256,
                }
            }
            fn cleanup_partial_dirs(&self, _: &mut Vec<DailyDescriptor>) {}
            fn drain_dirs(&self, dirs: Vec<DailyDescriptor>) {
                drop(dirs);
            }
            fn drain_root(&self, root: Root) {
                drop(root);
            }
            fn drain_leaf(&self, leaf: OpenedLeaf) {
                drop(leaf);
            }
        }
        struct CheckedMode<'s, 'inv, 'attempt> {
            scope: &'s Scope<'inv, 'attempt>,
            expected: &'s Attempt,
            captured: &'s Attempt,
            trace: RefCell<Trace>,
        }
        impl<'s, 'inv, 'attempt> CheckedMode<'s, 'inv, 'attempt> {
            fn new(
                scope: &'s Scope<'inv, 'attempt>,
                expected: &'s Attempt,
                captured: &'s Attempt,
            ) -> Self {
                Self {
                    scope,
                    expected,
                    captured,
                    trace: RefCell::new(Trace::default()),
                }
            }
            fn record_local(&self, phase: DailyReadPhase) {
                self.trace
                    .borrow_mut()
                    .locals
                    .push(self.scope.local(phase, None));
            }
            fn record(&self, local: DailyLocalCompletion) -> bool {
                let clean = local.is_clean();
                self.trace.borrow_mut().locals.push(local);
                clean
            }
        }
        impl DailyReadMode for CheckedMode<'_, '_, '_> {
            type Error = Stop;
            type Dir = Owner<File>;
            type File = Owner<File>;
            type Held = CheckedHeld;
            type Root = CheckedRoot;
            type Link<'a> = &'a CheckedRoot;
            type Leaf<'a> = CheckedLeaf<'a>;
            fn policy(error: Error) -> Stop {
                Stop::First(First::Artifact(error))
            }
            fn late() -> Stop {
                Stop::Late
            }
            fn failed(&self) {
                self.scope.failed();
            }
            fn check<'a>(&self, owners: impl Iterator<Item = &'a Attempt>) -> Result<(), Stop> {
                self.scope
                    .check(self.expected, self.captured, owners)
                    .map_err(|e| Stop::First(First::Scope(e)))
            }
            fn adopt(&self, fd: OwnedFd, role: Role) -> Owner<File> {
                self.scope.adopt(File::from(fd), role)
            }
            fn promote(fd: Owner<File>) -> Owner<File> {
                fd
            }
            fn dir(fd: &Owner<File>) -> BorrowedFd<'_> {
                fd.get().as_fd()
            }
            fn file(fd: &Owner<File>) -> &File {
                fd.get()
            }
            fn file_mut(fd: &mut Owner<File>) -> &mut File {
                fd.get_mut()
            }
            fn dir_attempt(fd: &Owner<File>) -> Option<&Attempt> {
                Some(fd.attempt())
            }
            fn file_attempt(fd: &Owner<File>) -> Option<&Attempt> {
                Some(fd.attempt())
            }
            fn retire_dir(&self, fd: Owner<File>, phase: DailyReadPhase) -> bool {
                self.record(retire_owned(self.scope, fd, phase))
            }
            fn retire_file(&self, fd: Owner<File>) -> bool {
                self.record(retire_owned(self.scope, fd, DailyReadPhase::File))
            }
            fn make_root(
                dirs: Vec<Owner<File>>,
                names: Vec<Vec<u8>>,
                identities: Vec<Identity>,
                owner: u32,
                group: u32,
                layout: Layout,
            ) -> CheckedRoot {
                CheckedRoot {
                    dirs,
                    names,
                    identities,
                    owner,
                    group,
                    layout,
                }
            }
            fn root_view(root: &CheckedRoot) -> RootView<'_, Owner<File>> {
                RootView {
                    dirs: &root.dirs,
                    names: &root.names,
                    identities: &root.identities,
                    owner: root.owner,
                    group: root.group,
                    layout: root.layout,
                }
            }
            fn root<'a>(root: Self::Link<'a>) -> &'a Self::Root {
                root
            }
            fn make_leaf<'a>(
                root: Self::Link<'a>,
                name: &str,
                directory: Self::Dir,
                stat: Stat,
            ) -> Self::Leaf<'a> {
                CheckedLeaf {
                    root,
                    name: name.into(),
                    directory,
                    stat: snapshot(&stat),
                    files: Vec::new(),
                }
            }
            fn leaf_view<'a, 'r>(leaf: &'a CheckedLeaf<'r>) -> LeafView<'a, Self> {
                LeafView {
                    root: leaf.root,
                    name: &leaf.name,
                    directory: &leaf.directory,
                    stat: &leaf.stat,
                    files: &leaf.files,
                }
            }
            fn files_mut<'a, 'r>(leaf: &'a mut CheckedLeaf<'r>) -> &'a mut Vec<CheckedHeld> {
                &mut leaf.files
            }
            fn make_held(
                name: &'static str,
                file: Owner<File>,
                stat: Snapshot,
                sha256: [u8; 32],
            ) -> CheckedHeld {
                CheckedHeld {
                    name,
                    file,
                    stat,
                    sha256,
                }
            }
            fn held_view(held: &CheckedHeld) -> HeldView<'_, Owner<File>> {
                HeldView {
                    name: held.name,
                    file: &held.file,
                    stat: &held.stat,
                    sha256: &held.sha256,
                }
            }
            fn cleanup_partial_dirs(&self, dirs: &mut Vec<Owner<File>>) {
                self.drain_dirs(std::mem::take(dirs));
            }
            fn drain_dirs(&self, dirs: Vec<Owner<File>>) {
                drain_owned(self.scope, dirs, DailyReadPhase::Root, |local| {
                    self.record(local);
                });
            }
            fn drain_root(&self, root: CheckedRoot) {
                self.drain_dirs(root.dirs);
            }
            fn drain_leaf(&self, leaf: CheckedLeaf<'_>) {
                drain_owned(
                    self.scope,
                    leaf.files.into_iter().map(|held| held.file),
                    DailyReadPhase::File,
                    |local| {
                        self.record(local);
                    },
                );
                self.retire_dir(leaf.directory, DailyReadPhase::Leaf);
            }
        }
        fn checkpoint<M: DailyReadMode>(
            mode: &M,
            dirs: &[M::Dir],
            directory: Option<&M::Dir>,
            files: &[M::Held],
            pending: Option<&M::Dir>,
            current: Option<&M::File>,
        ) -> Result<(), M::Error> {
            mode.check(
                dirs.iter()
                    .filter_map(M::dir_attempt)
                    .chain(directory.into_iter().filter_map(M::dir_attempt))
                    .chain(
                        files
                            .iter()
                            .filter_map(|f| M::file_attempt(M::held_view(f).file)),
                    )
                    .chain(pending.into_iter().filter_map(M::dir_attempt))
                    .chain(current.into_iter().filter_map(M::file_attempt)),
            )
        }
        fn check_leaf<M: DailyReadMode>(
            mode: &M,
            leaf: &M::Leaf<'_>,
            pending: Option<&M::Dir>,
            current: Option<&M::File>,
        ) -> Result<(), M::Error> {
            let leaf = M::leaf_view(leaf);
            checkpoint(
                mode,
                M::root_view(leaf.root).dirs,
                Some(leaf.directory),
                leaf.files,
                pending,
                current,
            )
        }
        fn keep_or_retire<M: DailyReadMode, T>(
            mode: &M,
            value: M::Dir,
            result: Result<T, M::Error>,
            phase: DailyReadPhase,
        ) -> Result<(M::Dir, T), M::Error> {
            match result {
                Ok(data) => Ok((value, data)),
                Err(error) => {
                    mode.failed();
                    mode.retire_dir(value, phase);
                    Err(error)
                }
            }
        }

        fn open_root_with<M: DailyReadMode>(
            mode: &M,
            path: &Path,
            owner: u32,
            group: u32,
            layout: Layout,
        ) -> Result<M::Root, M::Error> {
            mode.check(std::iter::empty())?;
            if owner == u32::MAX || group == u32::MAX {
                return Err(M::policy(Error::InvalidRoot));
            }
            let parts = components(path).map_err(M::policy)?;
            mode.check(std::iter::empty())?;
            let slash = mode.adopt(
                fs::open("/", DIRECTORY_FLAGS, Mode::empty()).map_err(|e| M::policy(io(e)))?,
                Role::Ancestor,
            );
            let valid = (|| {
                checkpoint(mode, &[], None, &[], Some(&slash), None)?;
                let slash_stat = fs::fstat(M::dir(&slash)).map_err(|e| M::policy(io(e)))?;
                if slash_stat.st_uid != 0 || slash_stat.st_mode & 0o022 != 0 {
                    return Err(M::policy(Error::UnsafePath));
                }
                checkpoint(mode, &[], None, &[], Some(&slash), None)?;
                no_acl(&M::dir(&slash), true).map_err(M::policy)
            })();
            let (slash, ()) = keep_or_retire(mode, slash, valid, DailyReadPhase::Pending)?;
            let mut dirs = vec![slash];
            let mut names = Vec::new();
            let mut identities = Vec::new();
            let result = (|| {
                for name in parts {
                    checkpoint(mode, &dirs, None, &[], None, None)?;
                    let parent = dirs.last().ok_or_else(|| M::policy(Error::InvalidRoot))?;
                    let fd = mode.adopt(
                        fs::openat(
                            M::dir(parent),
                            name.as_slice(),
                            DIRECTORY_FLAGS,
                            Mode::empty(),
                        )
                        .map_err(|_| M::policy(Error::UnsafePath))?,
                        Role::Ancestor,
                    );
                    let valid = (|| {
                        checkpoint(mode, &dirs, None, &[], Some(&fd), None)?;
                        let stat = fs::fstat(M::dir(&fd)).map_err(|e| M::policy(io(e)))?;
                        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
                            || (stat.st_uid != 0 && stat.st_uid != owner)
                            || stat.st_mode & 0o022 != 0
                        {
                            return Err(M::policy(Error::UnsafePath));
                        }
                        checkpoint(mode, &dirs, None, &[], Some(&fd), None)?;
                        no_acl(&M::dir(&fd), true).map_err(M::policy)?;
                        checkpoint(mode, &dirs, None, &[], Some(&fd), None)?;
                        let named =
                            fs::statat(M::dir(parent), name.as_slice(), AtFlags::SYMLINK_NOFOLLOW)
                                .map_err(|e| M::policy(io(e)))?;
                        if identity(&named) != identity(&stat) {
                            return Err(M::policy(Error::SourceChanged));
                        }
                        Ok(stat)
                    })();
                    let (fd, stat) = keep_or_retire(mode, fd, valid, DailyReadPhase::Pending)?;
                    identities.push(identity(&stat));
                    names.push(name);
                    dirs.push(fd);
                }
                let last = identities
                    .last()
                    .ok_or_else(|| M::policy(Error::InvalidRoot))?;
                if last.uid != owner || last.gid != group || last.mode & 0o7777 != 0o750 {
                    return Err(M::policy(Error::InvalidRoot));
                }
                Ok(())
            })();
            if let Err(error) = result {
                mode.failed();
                mode.cleanup_partial_dirs(&mut dirs);
                return Err(error);
            }
            let root = M::make_root(dirs, names, identities, owner, group, layout);
            if let Err(error) = revalidate_root_with(mode, &root) {
                mode.failed();
                mode.drain_root(root);
                return Err(error);
            }
            Ok(root)
        }
        fn revalidate_root_with<M: DailyReadMode>(
            mode: &M,
            root: &M::Root,
        ) -> Result<(), M::Error> {
            revalidate_root_extra(mode, root, || Ok(()))
        }
        fn revalidate_root_extra<M: DailyReadMode>(
            mode: &M,
            root: &M::Root,
            extra: impl Fn() -> Result<(), M::Error>,
        ) -> Result<(), M::Error> {
            let root = M::root_view(root);
            let check = || {
                extra()?;
                checkpoint(mode, root.dirs, None, &[], None, None)
            };
            check()?;
            let slash = fs::fstat(M::dir(&root.dirs[0])).map_err(|e| M::policy(io(e)))?;
            if slash.st_uid != 0 || slash.st_mode & 0o022 != 0 {
                return Err(M::policy(Error::SourceChanged));
            }
            check()?;
            no_acl(&M::dir(&root.dirs[0]), true).map_err(M::policy)?;
            for (i, name) in root.names.iter().enumerate() {
                check()?;
                let held = fs::fstat(M::dir(&root.dirs[i + 1])).map_err(|e| M::policy(io(e)))?;
                check()?;
                let named = fs::statat(
                    M::dir(&root.dirs[i]),
                    name.as_slice(),
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(|_| M::policy(Error::SourceChanged))?;
                if identity(&held) != root.identities[i] || identity(&named) != root.identities[i] {
                    return Err(M::policy(Error::SourceChanged));
                }
                check()?;
                no_acl(&M::dir(&root.dirs[i + 1]), true).map_err(M::policy)?;
            }
            Ok(())
        }
        fn exact_entries_with<M: DailyReadMode>(
            mode: &M,
            directory: &M::Dir,
            expected: &[&str],
            check: impl Fn(Option<&M::Dir>) -> Result<(), M::Error>,
        ) -> Result<(), M::Error> {
            check(None)?;
            let scan = mode.adopt(
                fs::openat(M::dir(directory), ".", DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|e| M::policy(io(e)))?,
                Role::Enumeration,
            );
            let result = (|| {
                check(Some(&scan))?;
                let scan_stat = fs::fstat(M::dir(&scan)).map_err(|e| M::policy(io(e)))?;
                check(Some(&scan))?;
                if identity(&scan_stat)
                    != identity(&fs::fstat(M::dir(directory)).map_err(|e| M::policy(io(e)))?)
                {
                    return Err(M::policy(Error::SourceChanged));
                }
                let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
                let mut iterator = RawDir::new(M::dir(&scan), &mut buffer);
                let mut names = Vec::new();
                loop {
                    check(Some(&scan))?;
                    let Some(entry) = iterator.next() else {
                        break;
                    };
                    let entry = entry.map_err(|e| M::policy(io(e)))?;
                    let name = entry.file_name().to_bytes();
                    if name == b"." || name == b".." {
                        continue;
                    }
                    if names.len() == expected.len()
                        || !expected.iter().any(|file| file.as_bytes() == name)
                        || names.iter().any(|old| old == name)
                    {
                        return Err(M::policy(Error::UnexpectedEntries));
                    }
                    names.push(name.to_vec());
                }
                names.sort_unstable();
                if names
                    .iter()
                    .map(Vec::as_slice)
                    .ne(expected.iter().map(|name| name.as_bytes()))
                {
                    return Err(M::policy(Error::UnexpectedEntries));
                }
                Ok(())
            })();
            if result.is_err() {
                mode.failed();
            }
            let clean = mode.retire_dir(scan, DailyReadPhase::Scan);
            after_retirement(result, clean, M::late)
        }
        fn hash_with<M: DailyReadMode>(
            file: &M::File,
            expected_size: u64,
            check: impl Fn() -> Result<(), M::Error>,
        ) -> Result<[u8; 32], M::Error> {
            let mut hash = Sha256::new();
            let mut position = 0u64;
            let mut chunk = [0u8; 8192];
            loop {
                check()?;
                let read = M::file(file)
                    .read_at(&mut chunk, position)
                    .map_err(|_| M::policy(Error::Io))?;
                if read == 0 {
                    break;
                }
                position = position
                    .checked_add(read as u64)
                    .ok_or_else(|| M::policy(Error::LimitExceeded))?;
                if position > expected_size {
                    return Err(M::policy(Error::SourceChanged));
                }
                hash.update(&chunk[..read]);
            }
            if position != expected_size {
                return Err(M::policy(Error::SourceChanged));
            }
            Ok(hash.finalize().into())
        }
        fn read_file_with<M: DailyReadMode>(
            mode: &M,
            directory: &M::Dir,
            name: &'static str,
            root: &M::Root,
            files: &[M::Held],
        ) -> Result<(M::Held, Vec<u8>), M::Error> {
            let root = M::root_view(root);
            checkpoint(mode, root.dirs, Some(directory), files, None, None)?;
            let fd = mode.adopt(
                fs::openat(
                    M::dir(directory),
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|_| M::policy(Error::UnsafePath))?,
                Role::Source,
            );
            let valid = (|| {
                checkpoint(mode, root.dirs, Some(directory), files, Some(&fd), None)?;
                let initial = fs::fstat(M::dir(&fd)).map_err(|e| M::policy(io(e)))?;
                if FileType::from_raw_mode(initial.st_mode) != FileType::RegularFile
                    || initial.st_uid != root.owner
                    || initial.st_gid != root.group
                    || initial.st_mode & 0o7777 != 0o440
                    || initial.st_nlink != 1
                {
                    return Err(M::policy(Error::UnsafePath));
                }
                if initial.st_size < 0 || initial.st_size as u64 > maximum(name) {
                    return Err(M::policy(Error::LimitExceeded));
                }
                checkpoint(mode, root.dirs, Some(directory), files, Some(&fd), None)?;
                no_acl(&M::dir(&fd), false).map_err(M::policy)?;
                Ok(initial)
            })();
            let (fd, initial) = keep_or_retire(mode, fd, valid, DailyReadPhase::Pending)?;
            let stat = snapshot(&initial);
            let mut file = M::promote(fd);
            let result = (|| {
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    checkpoint(mode, root.dirs, Some(directory), files, None, Some(&file))?;
                    let read = M::file_mut(&mut file)
                        .read(&mut chunk)
                        .map_err(|_| M::policy(Error::Io))?;
                    if read == 0 {
                        break;
                    }
                    let total = bytes
                        .len()
                        .checked_add(read)
                        .ok_or_else(|| M::policy(Error::LimitExceeded))?;
                    if total as u64 > maximum(name) {
                        return Err(M::policy(Error::LimitExceeded));
                    }
                    bytes
                        .try_reserve(read)
                        .map_err(|_| M::policy(Error::LimitExceeded))?;
                    bytes.extend_from_slice(&chunk[..read]);
                }
                checkpoint(mode, root.dirs, Some(directory), files, None, Some(&file))?;
                let named = fs::statat(M::dir(directory), name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| M::policy(Error::SourceChanged))?;
                if bytes.len() as u64 != initial.st_size as u64
                    || {
                        checkpoint(mode, root.dirs, Some(directory), files, None, Some(&file))?;
                        snapshot(&fs::fstat(M::file(&file)).map_err(|e| M::policy(io(e)))?) != stat
                    }
                    || snapshot(&named) != stat
                {
                    return Err(M::policy(Error::SourceChanged));
                }
                let sha256 = Sha256::digest(&bytes).into();
                Ok((bytes, sha256))
            })();
            match result {
                Ok((bytes, sha256)) => Ok((M::make_held(name, file, stat, sha256), bytes)),
                Err(error) => {
                    mode.failed();
                    mode.retire_file(file);
                    Err(error)
                }
            }
        }
        fn open_leaf_with<'r, M: DailyReadMode>(
            mode: &M,
            link: M::Link<'r>,
            name: &str,
        ) -> Result<M::Leaf<'r>, M::Error> {
            let root = M::root(link);
            revalidate_root_with(mode, root)?;
            let root = M::root_view(root);
            checkpoint(mode, root.dirs, None, &[], None, None)?;
            let parent = root.dirs.last().expect("validated nonempty root");
            let directory = mode.adopt(
                fs::openat(M::dir(parent), name, DIRECTORY_FLAGS, Mode::empty())
                    .map_err(|_| M::policy(Error::UnsafePath))?,
                Role::Child,
            );
            let valid = (|| {
                let check = || checkpoint(mode, root.dirs, None, &[], Some(&directory), None);
                check()?;
                let stat = fs::fstat(M::dir(&directory)).map_err(|e| M::policy(io(e)))?;
                if stat.st_uid != root.owner
                    || stat.st_gid != root.group
                    || stat.st_mode & 0o7777 != 0o750
                {
                    return Err(M::policy(Error::UnsafePath));
                }
                check()?;
                no_acl(&M::dir(&directory), true).map_err(M::policy)?;
                exact_entries_with(mode, &directory, root.layout.files(), |scan| {
                    checkpoint(mode, root.dirs, Some(&directory), &[], scan, None)
                })?;
                Ok(stat)
            })();
            let (directory, stat) =
                keep_or_retire(mode, directory, valid, DailyReadPhase::Pending)?;
            Ok(M::make_leaf(link, name, directory, stat))
        }
        fn revalidate_leaf_with<M: DailyReadMode>(
            mode: &M,
            held_leaf: &M::Leaf<'_>,
        ) -> Result<(), M::Error> {
            let leaf = M::leaf_view(held_leaf);
            revalidate_root_extra(mode, leaf.root, || check_leaf(mode, held_leaf, None, None))?;
            let root = M::root_view(leaf.root);
            let check = || check_leaf(mode, held_leaf, None, None);
            check()?;
            let named = fs::statat(
                M::dir(root.dirs.last().expect("validated nonempty root")),
                leaf.name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| M::policy(Error::SourceChanged))?;
            if snapshot(&named) != *leaf.stat || {
                check()?;
                snapshot(&fs::fstat(M::dir(leaf.directory)).map_err(|e| M::policy(io(e)))?)
                    != *leaf.stat
            } {
                return Err(M::policy(Error::SourceChanged));
            }
            check()?;
            no_acl(&M::dir(leaf.directory), true).map_err(M::policy)?;
            exact_entries_with(mode, leaf.directory, root.layout.files(), |scan| {
                check_leaf(mode, held_leaf, scan, None)
            })?;
            for held in leaf.files {
                let file = M::held_view(held);
                check()?;
                let current = fs::fstat(M::file(file.file)).map_err(|e| M::policy(io(e)))?;
                check()?;
                let named =
                    fs::statat(M::dir(leaf.directory), file.name, AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| M::policy(Error::SourceChanged))?;
                if snapshot(&current) != *file.stat || snapshot(&named) != *file.stat {
                    return Err(M::policy(Error::SourceChanged));
                }
                check()?;
                no_acl(M::file(file.file), false).map_err(M::policy)?;
                if hash_with::<M>(file.file, file.stat.size as u64, check)? != *file.sha256
                    || {
                        check()?;
                        snapshot(&fs::fstat(M::file(file.file)).map_err(|e| M::policy(io(e)))?)
                            != *file.stat
                    }
                    || {
                        check()?;
                        snapshot(
                            &fs::statat(
                                M::dir(leaf.directory),
                                file.name,
                                AtFlags::SYMLINK_NOFOLLOW,
                            )
                            .map_err(|_| M::policy(Error::SourceChanged))?,
                        ) != *file.stat
                    }
                {
                    return Err(M::policy(Error::SourceChanged));
                }
            }
            revalidate_root_extra(mode, leaf.root, || check_leaf(mode, held_leaf, None, None))
        }
        fn read_leaf_with<'r, M: DailyReadMode>(
            mode: &M,
            link: M::Link<'r>,
            name: &str,
            retained: &mut Option<M::Leaf<'r>>,
            data: &mut Data,
        ) -> Result<(), M::Error> {
            *retained = Some(open_leaf_with(mode, link, name)?);
            for &name in M::root_view(M::root(link)).layout.files() {
                let leaf = retained.as_mut().expect("opened leaf");
                let view = M::leaf_view(leaf);
                let (file, content) =
                    read_file_with(mode, view.directory, name, view.root, view.files)?;
                M::files_mut(leaf).push(file);
                retain_returned(&mut data.files, name, content);
                data.stage = DailyReturnedStage::PartialFiles;
            }
            revalidate_leaf_with(mode, retained.as_ref().expect("opened leaf"))?;
            data.stage = DailyReturnedStage::FullFiles;
            Ok(())
        }
        fn metadata_with<'r, M: DailyReadMode>(
            mode: &M,
            link: M::Link<'r>,
            name: &str,
            retained: &mut Option<M::Leaf<'r>>,
            data: &mut Data,
        ) -> Result<(), M::Error> {
            if !matches!(M::root_view(M::root(link)).layout, Layout::Admitted) {
                return Err(M::policy(Error::InvalidRoot));
            }
            *retained = Some(open_leaf_with(mode, link, name)?);
            let leaf = retained.as_mut().expect("opened metadata leaf");
            let view = M::leaf_view(leaf);
            let (file, bytes) = read_file_with(
                mode,
                view.directory,
                "admission.json",
                view.root,
                view.files,
            )?;
            M::files_mut(leaf).push(file);
            data.admission = Some(bytes);
            data.stage = DailyReturnedStage::Metadata;
            revalidate_leaf_with(mode, leaf)
        }
        fn admission_data(admission: &domain::ContentHash, data: &mut Data) -> Result<(), Error> {
            data.admission = Some(
                data.files
                    .remove("admission.json")
                    .ok_or(Error::InvalidArtifact)?,
            );
            if domain::ContentHash::from_bytes(data.admission.as_ref().expect("returned admission"))
                != *admission
            {
                return Err(Error::InvalidArtifact);
            }
            Ok(())
        }
        fn candidate_data(content: &domain::ContentHash, data: &mut Data) -> Result<(), Error> {
            data.manifest = Some(
                data.files
                    .remove("manifest.json")
                    .ok_or(Error::InvalidArtifact)?,
            );
            data.normalization = Some(
                data.files
                    .remove("normalization.json")
                    .ok_or(Error::InvalidArtifact)?,
            );
            data.bars = Some(
                data.files
                    .remove("bars.ndjson")
                    .ok_or(Error::InvalidArtifact)?,
            );
            data.parsed = Some(super::super::check_bytes(
                content,
                data.manifest.as_deref().expect("returned manifest"),
                data.normalization
                    .as_deref()
                    .expect("returned normalization"),
                data.bars.as_deref().expect("returned bars"),
            )?);
            data.stage = DailyReturnedStage::Parsed;
            Ok(())
        }

        pub(super) fn legacy_root(
            path: &Path,
            owner: u32,
            group: u32,
            layout: Layout,
        ) -> Result<Root, Error> {
            open_root_with(&Legacy::ordinary(), path, owner, group, layout)
        }
        pub(super) fn legacy_revalidate_root(root: &Root) -> Result<(), Error> {
            revalidate_root_with(&Legacy::for_root(root), root)
        }
        pub(super) fn legacy_revalidate_leaf(leaf: &OpenedLeaf) -> Result<(), Error> {
            revalidate_leaf_with(&Legacy::for_root(&leaf.root), leaf)
        }
        pub(super) fn legacy_read_file(
            directory: &DailyDescriptor,
            name: &'static str,
            root: &Root,
        ) -> Result<(HeldFile, Vec<u8>), Error> {
            read_file_with(&Legacy::for_root(root), directory, name, root, &[])
        }
        pub(super) fn legacy_open_leaf(root: &Arc<Root>, name: &str) -> Result<OpenedLeaf, Error> {
            open_leaf_with(&Legacy::for_root(root), root, name)
        }
        pub(super) fn legacy_read_leaf(
            root: &Arc<Root>,
            name: &str,
        ) -> Result<(OpenedLeaf, LeafBytes), Error> {
            let mut leaf = None;
            let mut data = Data::default();
            read_leaf_with(&Legacy::for_root(root), root, name, &mut leaf, &mut data)?;
            Ok((leaf.expect("returned leaf"), data.files))
        }
        pub(super) fn legacy_metadata(root: &Arc<Root>, name: &str) -> Result<Vec<u8>, Error> {
            let mut leaf = None;
            let mut data = Data::default();
            metadata_with(&Legacy::for_root(root), root, name, &mut leaf, &mut data)?;
            Ok(data.admission.expect("returned metadata"))
        }
        pub(super) fn archive_root(path: &Path, owner: u32, group: u32, layout: Layout,
            attempt: &Attempt) -> Result<Root, Error> {
            open_root_with(&Legacy { attempt: Some(attempt.clone()) }, path, owner, group, layout)
        }
        pub(super) fn legacy_candidate(
            content: &domain::ContentHash,
            files: OpenedLeaf,
            bytes: LeafBytes,
        ) -> Result<super::super::OpenedDailyCandidate, Error> {
            let mut data = Data {
                files: bytes,
                ..Data::default()
            };
            candidate_data(content, &mut data)?;
            files.revalidate()?;
            let (manifest, normalization) = data.parsed.expect("returned parsed tuple");
            Ok(super::super::OpenedDailyCandidate {
                manifest,
                normalization,
                manifest_bytes: data.manifest.expect("returned manifest"),
                normalization_bytes: data.normalization.expect("returned normalization"),
                bars: data.bars.expect("returned bars"),
                files,
            })
        }
        pub(super) fn legacy_admission(
            root: &Arc<Root>,
            admission: &domain::ContentHash,
            content: &domain::ContentHash,
        ) -> Result<(super::super::OpenedDailyCandidate, Vec<u8>), Error> {
            let (files, bytes) = legacy_read_leaf(root, &super::super::leaf_name(admission)?)?;
            let mut data = Data {
                files: bytes,
                ..Data::default()
            };
            admission_data(admission, &mut data)?;
            Ok((
                legacy_candidate(content, files, data.files)?,
                data.admission.expect("returned admission"),
            ))
        }

        #[cfg(test)]
        mod custody_tests {
            use super::*;
            use crate::checked_close::{Errno, Issue};
            use std::cell::Cell;
            use std::rc::Rc;

            struct Counted {
                id: usize,
                calls: Rc<RefCell<Vec<usize>>>,
                error: Option<Errno>,
            }
            impl Close for Counted {
                fn close(self) -> Result<(), Errno> {
                    self.calls.borrow_mut().push(self.id);
                    match self.error {
                        Some(e) => Err(e),
                        None => Ok(()),
                    }
                }
            }
            fn resource(
                id: usize,
                calls: &Rc<RefCell<Vec<usize>>>,
                error: Option<Errno>,
            ) -> Counted {
                Counted {
                    id,
                    calls: calls.clone(),
                    error,
                }
            }
            struct Body(Box<str>);

            #[test]
            #[expect(
                clippy::result_large_err,
                reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
            )]
            fn c14_actual_none_root_and_shared_admission_stop_before_effect() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let error = CheckedDailyAdmissionRoot::open_in(
                    &inv,
                    &actual,
                    &actual,
                    Path::new("/must-not-open"),
                    10003,
                    10001,
                )
                .unwrap_err();
                assert_eq!(error.first_kind(), Some(DailyArtifactFirstKind::Binding));
                assert_eq!(error.source_report().unwrap().attempted(), 0);
                assert_eq!(error.source_report().unwrap().live(), 0);
                let next = Attempt::default();
                let inv = RawInvocation::new(&next);
                let scope = Scope::new(&inv, &next, &next);
                let calls = Cell::new(0);
                let result = admit(&scope, &next, &next, || {
                    calls.set(calls.get() + 1);
                    Ok(())
                });
                assert!(matches!(result, Err(Stop::First(First::Binding(_)))));
                assert_eq!(calls.get(), 0);
            }
            #[test]
            fn c14_all_tokens_and_foreign_owner_poison_even_initially_dirty() {
                let actual = Attempt::default();
                actual.gap(Gap::Abandoned);
                let expected = Attempt::default();
                let captured = Attempt::default();
                let stored = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &stored, &actual);
                let foreign = Attempt::default();
                let calls = Rc::new(RefCell::new(Vec::new()));
                let owner = foreign.adopt(resource(7, &calls, None), Role::Child);
                let error = scope
                    .check(&expected, &captured, std::iter::once(owner.attempt()))
                    .unwrap_err();
                for token in [&actual, &expected, &captured, &stored, &foreign] {
                    assert!(!token.check());
                    let report = token.report();
                    assert!(
                        report.first() == Some(Issue::Gap(Gap::ScopeMismatch))
                            || report.secondary().contains(&Issue::Gap(Gap::ScopeMismatch))
                    );
                }
                assert!(error.expected_result().is_err() && error.captured_result().is_err());
                let done = retire_owned(&scope, owner, DailyReadPhase::Pending);
                assert!(!done.is_clean());
                assert_eq!(&*calls.borrow(), &[7]);
            }
            #[test]
            fn c14_adoption_precedes_postopen_failure_and_partial_graph_drain() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let outer = actual.adopt(resource(9, &calls, None), Role::Source);
                let held = vec![scope.adopt(resource(1, &calls, None), Role::Ancestor)];
                let pending = scope.adopt(resource(2, &calls, None), Role::Child);
                assert_eq!(actual.report().live(), 3);
                let original: Result<(), Body> = Err(Body("postopen".into()));
                let pending_done = retire_owned(&scope, pending, DailyReadPhase::Pending);
                assert!(pending_done.is_clean());
                drain_owned(&scope, held, DailyReadPhase::Root, |_| {});
                assert_eq!(&*calls.borrow(), &[2, 1]);
                assert_eq!(actual.report().live(), 1);
                assert!(matches!(original, Err(Body(_))));
                drop(outer);
            }
            #[test]
            fn c14_cursor_retirement_late_none_blocks_next_policy() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let scan = scope.adopt(
                    resource(1, &calls, Some(Errno::Interrupted)),
                    Role::Enumeration,
                );
                let done = retire_owned(&scope, scan, DailyReadPhase::Scan);
                let next = Cell::new(0);
                let result: Result<(), Stop> =
                    after_retirement(Ok(()), done.is_clean(), || Stop::Late);
                if result.is_ok() {
                    next.set(1);
                }
                assert!(matches!(result, Err(Stop::Late)));
                assert_eq!(next.get(), 0);
                assert_eq!(&*calls.borrow(), &[1]);
                assert_eq!(actual.report().unknown(), 1);
            }
            #[test]
            fn c14_first_nonclone_error_survives_close_without_late_mapping() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let owner = scope.adopt(resource(1, &calls, Some(Errno::Io)), Role::Source);
                let body = Body("primary".into());
                let pointer = body.0.as_ptr();
                let original: Result<(), Body> = Err(body);
                let done = retire_owned(&scope, owner, DailyReadPhase::File);
                let maps = Cell::new(0);
                let result = after_retirement(original, done.is_clean(), || {
                    maps.set(1);
                    Body("late".into())
                });
                let body = match result {
                    Err(e) => e,
                    Ok(()) => panic!("lost primary"),
                };
                assert_eq!(body.0.as_ptr(), pointer);
                assert_eq!(maps.get(), 0);
                assert_eq!(&*calls.borrow(), &[1]);
            }
            #[test]
            fn c14_success_data_stays_outside_actual_unit_local_error() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let value = Body("returned".into());
                let pointer = value.0.as_ptr();
                let held = scope.adopt(resource(1, &calls, Some(Errno::Io)), Role::Source);
                let done = retire_owned(&scope, held, DailyReadPhase::File);
                assert!(!done.is_clean());
                assert!(
                    done.expected_result()
                        .as_ref()
                        .err()
                        .unwrap()
                        .body()
                        .is_none()
                );
                assert_eq!(value.0.as_ptr(), pointer);
                assert_eq!(actual.report().live(), 0);
            }
            #[test]
            fn c14_partial_returned_files_exclude_failed_read_and_metadata_is_navigation() {
                let mut data = Data::default();
                retain_returned(&mut data.files, "admission.json", b"first".to_vec());
                data.stage = DailyReturnedStage::PartialFiles;
                assert_eq!(data.file("admission.json"), Some(b"first".as_slice()));
                assert!(data.file("bars.ndjson").is_none());
                assert!(data.parsed.is_none());
                data.admission = data.files.remove("admission.json");
                data.stage = DailyReturnedStage::Metadata;
                assert_eq!(data.stage, DailyReturnedStage::Metadata);
                assert!(data.parsed.is_none());
            }
            #[test]
            fn c14_stored_order_dirty_cleanup_and_outer_owner_remains_live() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let outer = scope.adopt(resource(8, &calls, None), Role::Source);
                let owners = vec![
                    scope.adopt(resource(1, &calls, Some(Errno::Io)), Role::Ancestor),
                    scope.adopt(resource(2, &calls, None), Role::Ancestor),
                ];
                let mut results = Vec::new();
                drain_owned(&scope, owners, DailyReadPhase::Root, |r| results.push(r));
                assert_eq!(&*calls.borrow(), &[1, 2]);
                assert_eq!(results.len(), 2);
                assert_eq!(actual.report().live(), 1);
                assert!(!results[1].is_clean());
                drop(outer);
            }
            #[test]
            fn c14_unwind_marks_abandoned_and_canonical_fallback_closes_once() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _gap = Unwind::new(&actual);
                    let _owner = scope.adopt(resource(1, &calls, None), Role::Child);
                    panic!("ordinary model unwind");
                }));
                assert!(result.is_err());
                assert_eq!(&*calls.borrow(), &[1]);
                assert_eq!(actual.report().first(), Some(Issue::Gap(Gap::Abandoned)));
            }
            #[test]
            fn c14_reuse_after_dirt_refuses_and_does_not_consume_outer_owner() {
                let actual = Attempt::default();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let outer = scope.adopt(resource(1, &calls, None), Role::Source);
                assert!(
                    scope
                        .check(&actual, &actual, std::iter::once(outer.attempt()))
                        .is_ok()
                );
                actual.gap(Gap::Reuse);
                assert!(
                    scope
                        .check(&actual, &actual, std::iter::once(outer.attempt()))
                        .is_err()
                );
                assert!(calls.borrow().is_empty());
                drop(outer);
            }
            #[test]
            fn c14_canonical_some_none_snapshot_and_redacted_debug() {
                let actual = Attempt::default();
                let first = actual.failure(Some(Body("secret-marker".into())));
                let original_report = first.report().first();
                let pointer = first.body().unwrap().0.as_ptr();
                let inv = RawInvocation::new(&actual);
                let scope = Scope::new(&inv, &actual, &actual);
                let calls = Rc::new(RefCell::new(Vec::new()));
                let owner = scope.adopt(resource(1, &calls, Some(Errno::Io)), Role::Source);
                let _later = retire_owned(&scope, owner, DailyReadPhase::File);
                assert_eq!(first.body().unwrap().0.as_ptr(), pointer);
                assert_eq!(first.report().first(), original_report);
                assert_eq!(first.report().unknown(), 0);
                let error = scope
                    .check(&actual, &actual, std::iter::empty())
                    .unwrap_err();
                assert!(
                    error
                        .expected_result()
                        .as_ref()
                        .err()
                        .unwrap()
                        .body()
                        .is_none()
                );
                assert!(!format!("{error:?}").contains("secret-marker"));
                assert!(!format!("{error:?}").contains("Report"));
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

        // SOURCE_ONLY / NOT_RUN. Ordinary protected fixture roots retain
        // their real UID; production archive construction still fixes10003:10001.
        #[test]
        fn archive_descriptor_mode_uses_same_four_original_reads_and_checked_close() {
            let fixture = Fixture::new(); let _ordinary = admitted_metadata_fixture(&fixture);
            let attempt = crate::checked_close::Attempt::default();
            let root = Arc::new(checked::archive_root(&fixture.path, fixture.root.owner, fixture.root.group,
                Layout::Admitted, &attempt).unwrap());
            let (leaf, bytes) = read_leaf(&root, NAME).unwrap();
            assert_eq!(bytes["admission.json"], b"navigation bytes only");
            for (name, expected) in BYTES { assert_eq!(bytes[name], expected); }
            leaf.revalidate().unwrap(); drop(leaf);
            let (names, hashes) = capture_names(&root, 4).unwrap(); assert_eq!(hashes.len(), 1);
            names.revalidate().unwrap(); drop(names);
            assert_eq!(Arc::strong_count(&root), 1); drop(root);
            assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
        }
        #[test]
        fn archive_descriptor_mode_rejects_pending_members_and_same_byte_replacements() {
            for replacement in [false, true] {
                let fixture = Fixture::new(); let _ordinary = admitted_metadata_fixture(&fixture);
                let attempt = crate::checked_close::Attempt::default();
                let root = Arc::new(checked::archive_root(&fixture.path, fixture.root.owner, fixture.root.group,
                    Layout::Admitted, &attempt).unwrap());
                let (leaf, _) = read_leaf(&root, NAME).unwrap();
                if replacement {
                    let name = fixture.leaf().join("admission.json"); let bytes = std::fs::read(&name).unwrap();
                    std::fs::rename(&name, fixture.directory.path().join("retained-admission")).unwrap();
                    std::fs::write(&name, bytes).unwrap();
                    std::fs::set_permissions(&name, std::fs::Permissions::from_mode(0o440)).unwrap();
                    assert!(leaf.revalidate().is_err());
                } else {
                    std::fs::create_dir(fixture.path.join(".pending-1-1")).unwrap();
                    assert!(capture_names(&root, 4).is_err());
                }
                drop(leaf); drop(root);
                // Rejection and close evidence are separate; all FDs retire.
                let _ = attempt.finish(Err::<(), ()>(()));
            }
        }
        #[test]
        fn archive_descriptor_mode_refuses_wrong_original_owner_and_mode() {
            let fixture = Fixture::new(); let _ordinary = admitted_metadata_fixture(&fixture);
            let attempt = crate::checked_close::Attempt::default();
            assert!(checked::archive_root(&fixture.path, fixture.root.owner.wrapping_add(1), fixture.root.group,
                Layout::Admitted, &attempt).is_err());
            let attempt = crate::checked_close::Attempt::default();
            let root = Arc::new(checked::archive_root(&fixture.path, fixture.root.owner, fixture.root.group,
                Layout::Admitted, &attempt).unwrap());
            let path = fixture.leaf().join("admission.json");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(read_leaf(&root, NAME).is_err()); drop(root);
            let failure = attempt.finish(Err::<(), ()>(())).err().unwrap();
            assert_eq!(failure.report().live(), 0);
        }

        #[test]
        fn archive_descriptor_late_close_failure_keeps_every_original_provisional() {
            struct BadClose;
            impl crate::checked_close::Close for BadClose {
                fn close(self) -> Result<(), crate::checked_close::Errno> { Err(crate::checked_close::Errno::Io) }
            }
            let fixture = Fixture::new(); let _ordinary = admitted_metadata_fixture(&fixture);
            let attempt = crate::checked_close::Attempt::default();
            let root = Arc::new(checked::archive_root(&fixture.path, fixture.root.owner, fixture.root.group,
                Layout::Admitted, &attempt).unwrap());
            let (leaf, bytes) = read_leaf(&root, NAME).unwrap();
            assert_eq!(bytes["admission.json"], b"navigation bytes only");
            assert!(!attempt.adopt(BadClose, crate::checked_close::Role::Source).finish());
            assert!(leaf.revalidate().is_err()); assert!(root.revalidate().is_err());
            drop(leaf); drop(root);
            let failure = attempt.finish(Ok::<(), ()>(())).err().unwrap();
            assert_eq!(failure.report().live(), 0); assert_eq!(failure.report().unknown(), 1);
        }
        #[test]
        fn archive_descriptor_root_cannot_publish_even_for_the_actual_owner() {
            let fixture = Fixture::new();
            let attempt = crate::checked_close::Attempt::default();
            let root = Arc::new(checked::archive_root(&fixture.path, fixture.root.owner, fixture.root.group,
                Layout::Candidate, &attempt).unwrap());
            assert_eq!(publish(&root, NAME, &BYTES), Err(Error::InvalidRoot));
            assert!(!fixture.leaf().exists()); drop(root);
            assert!(attempt.finish(Ok::<(), ()>(())).is_ok());
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

// C14 lower read APIs are deliberately not connected to replay/Context.
#[cfg(target_os = "linux")]
pub use linux::checked::{
    CheckedDailyAdmissionFiles, CheckedDailyAdmissionMetadata, CheckedDailyAdmissionRoot,
    CheckedDailyAdmissionView, CheckedDailyArtifactFailure, CheckedDailyCompletion,
    DailyArtifactFirstKind, DailyLocalCompletion, DailyReadPhase, DailyRetainedData,
    DailyReturnedStage, DailyUnitResult, DailyUseFailure,
};
