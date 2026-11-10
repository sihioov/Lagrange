//! Read-only snapshots of complete committed Raw storage scopes.
//!
//! A snapshot is storage evidence only. It preserves the committed metadata
//! and opaque provider bytes; it does not validate a provider response or
//! authorize normalization, copying, or admission.

use std::error::Error;
use std::fmt;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::contract::{FetchMode, MARKET_KR, ResponseKind, SourceFileReference};
use crate::storage::ManifestEntry;
use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};

const MAX_D4_SOURCE_FILE_BYTES: usize = 67_108_864;
const MAX_RAW_JSON_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawProvider {
    Kis,
    KisCalendar,
    #[cfg(target_os = "linux")]
    HistoricalRange,
    #[cfg(target_os = "linux")]
    HistoricalNormalized,
    #[cfg(target_os = "linux")]
    CuratedKrx,
    #[cfg(target_os = "linux")]
    CuratedNormalized,
    #[cfg(target_os = "linux")]
    CuratedCalendar,
}

impl RawProvider {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Kis => "kis",
            Self::KisCalendar => "kis-calendar",
            #[cfg(target_os = "linux")]
            Self::HistoricalRange => "kis-daily-range",
            #[cfg(target_os = "linux")]
            Self::HistoricalNormalized => "kis-daily-range-normalized",
            #[cfg(target_os = "linux")]
            Self::CuratedKrx => "krx",
            #[cfg(target_os = "linux")]
            Self::CuratedNormalized => "kis-normalized",
            #[cfg(target_os = "linux")]
            Self::CuratedCalendar => "kis-calendar-normalized",
        }
    }
}

/// Closed historical archive providers; never accepted by the Daily constructor.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub(crate) enum HistoricalRawProvider {
    Prices,
    Normalized,
    Actions,
    Calendar,
}
#[cfg(target_os = "linux")]
impl HistoricalRawProvider {
    pub(crate) fn scope(self) -> ApprovedRawScope {
        ApprovedRawScope {
            provider: match self {
                Self::Prices => RawProvider::HistoricalRange,
                Self::Normalized => RawProvider::HistoricalNormalized,
                Self::Actions => RawProvider::Kis,
                Self::Calendar => RawProvider::KisCalendar,
            },
            trusted_raw_owner_uid: 10001,
        }
    }
}

/// Only the existing PublicationBundle and CalendarPublicationBundle scopes.
/// This archive-only selector does not change ApprovedRawScope::new or any
/// live acquisition allowlist, and is distinct from the fixed historical four.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub(crate) enum PublicationRawProvider {
    Krx,
    Normalized,
    Calendar,
}
#[cfg(target_os = "linux")]
impl PublicationRawProvider {
    pub(crate) fn scope(self) -> ApprovedRawScope {
        ApprovedRawScope {
            provider: match self {
                Self::Krx => RawProvider::CuratedKrx,
                Self::Normalized => RawProvider::CuratedNormalized,
                Self::Calendar => RawProvider::CuratedCalendar,
            },
            trusted_raw_owner_uid: 10001,
        }
    }
}

/// A validated provider/market scope and an explicitly supplied Raw owner.
///
/// This value validates its representation only. It does not attest to an
/// installer's approval or infer the owner from the current process.
pub struct ApprovedRawScope {
    provider: RawProvider,
    trusted_raw_owner_uid: u32,
}

impl ApprovedRawScope {
    pub fn new(
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
    ) -> Result<Self, DailyEvidenceError> {
        let provider = match provider {
            "kis" => RawProvider::Kis,
            "kis-calendar" => RawProvider::KisCalendar,
            _ => return Err(DailyEvidenceError::InvalidScope),
        };
        if market != MARKET_KR || trusted_raw_owner_uid == u32::MAX {
            return Err(DailyEvidenceError::InvalidScope);
        }
        Ok(Self {
            provider,
            trusted_raw_owner_uid,
        })
    }

    pub fn provider(&self) -> &'static str {
        self.provider.as_str()
    }

    pub fn market(&self) -> &'static str {
        MARKET_KR
    }

    pub fn trusted_raw_owner_uid(&self) -> u32 {
        self.trusted_raw_owner_uid
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanLimitValues {
    pub max_manifest_bytes: usize,
    pub max_manifest_records: usize,
    pub max_batches: usize,
    pub max_files_per_batch: usize,
    pub max_files: usize,
    pub max_batch_metadata_bytes: usize,
    pub max_source_file_bytes: usize,
    pub max_retained_bytes: usize,
    pub max_open_descriptors: usize,
    pub max_data_root_bytes: usize,
    pub max_data_root_components: usize,
    pub max_metadata_json_depth: usize,
}

/// Independent finite caps for one complete-scope scan.
pub struct ScanLimits {
    values: ScanLimitValues,
}

impl ScanLimits {
    pub fn new(values: ScanLimitValues) -> Result<Self, DailyEvidenceError> {
        let positive = [
            values.max_manifest_bytes,
            values.max_manifest_records,
            values.max_batches,
            values.max_files_per_batch,
            values.max_files,
            values.max_batch_metadata_bytes,
            values.max_source_file_bytes,
            values.max_retained_bytes,
            values.max_open_descriptors,
            values.max_data_root_bytes,
            values.max_data_root_components,
            values.max_metadata_json_depth,
        ];
        if positive.contains(&0)
            || values.max_source_file_bytes > MAX_D4_SOURCE_FILE_BYTES
            || values.max_metadata_json_depth > MAX_RAW_JSON_DEPTH
        {
            return Err(DailyEvidenceError::InvalidLimits);
        }

        let byte_caps = [
            values.max_manifest_bytes,
            values.max_batch_metadata_bytes,
            values.max_source_file_bytes,
            values.max_retained_bytes,
            values.max_data_root_bytes,
        ];
        for cap in byte_caps {
            cap.checked_add(1)
                .filter(|next| *next <= isize::MAX as usize)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        }

        let count_caps = [
            values.max_manifest_records,
            values.max_batches,
            values.max_files_per_batch,
            values.max_files,
            values.max_open_descriptors,
            values.max_data_root_components,
        ];
        for cap in count_caps {
            cap.checked_add(1)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        }

        #[cfg(target_os = "linux")]
        checked_capacity(
            values.max_manifest_records,
            std::mem::size_of::<PendingBatch>(),
        )?;
        #[cfg(target_os = "linux")]
        checked_capacity(values.max_batches, std::mem::size_of::<PendingBatch>())?;
        checked_capacity(values.max_batches, std::mem::size_of::<(BatchId, usize)>())?;
        checked_capacity(values.max_batches, std::mem::size_of::<CommittedRawBatch>())?;
        checked_capacity(values.max_batches, std::mem::size_of::<TradingDate>())?;
        checked_capacity(
            values.max_files_per_batch,
            std::mem::size_of::<crate::storage::FileEntry>(),
        )?;
        checked_capacity(values.max_files, std::mem::size_of::<CapturedLeaf>())?;
        checked_capacity(values.max_files_per_batch, std::mem::size_of::<bool>())?;
        checked_capacity(
            values.max_data_root_components,
            std::mem::size_of::<Vec<u8>>(),
        )?;
        #[cfg(target_os = "linux")]
        linux::validate_limit_layout(&values)?;

        Ok(Self { values })
    }

    pub fn values(&self) -> ScanLimitValues {
        self.values
    }
}

fn checked_capacity(count: usize, element_size: usize) -> Result<usize, DailyEvidenceError> {
    let bytes = count
        .checked_mul(element_size)
        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
    if bytes > isize::MAX as usize {
        return Err(DailyEvidenceError::ArithmeticOverflow);
    }
    Ok(bytes)
}

pub struct CommittedRawReader<'root> {
    data_root: &'root Path,
}

impl<'root> CommittedRawReader<'root> {
    pub fn new(data_root: &'root Path) -> Self {
        Self { data_root }
    }

    pub fn snapshot(
        &self,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
        #[cfg(target_os = "linux")]
        {
            linux::snapshot(self.data_root, scope, limits)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self.data_root, scope, limits);
            Err(DailyEvidenceError::UnsupportedPlatform)
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{
    CheckedRawBatchView, CheckedRawFileView, CheckedRawPartialBatchView, CheckedRawSnapshot,
    CheckedRawSnapshotCompletion, CheckedRawSnapshotFailure, CheckedRawSnapshotUseFailure,
    CheckedRawSnapshotView, SnapshotFirstKind, SnapshotScopeResults,
};

#[cfg(target_os = "linux")]
pub(crate) use linux::with_checked_raw_originals;

#[cfg(target_os = "linux")]
impl CommittedRawReader<'_> {
    /// Archive-only captured originals for the existing publication readers.
    #[expect(
        clippy::result_large_err,
        reason = "Preserve actual checked scan failure custody."
    )]
    pub(crate) fn publication_snapshot_checked_in<'i, 'a>(
        &self,
        invocation: &'i crate::raw_custody::RawInvocation<'a>,
        expected: &crate::checked_close::Attempt,
        captured: &crate::checked_close::Attempt,
        coordination_root: &Path,
        provider: PublicationRawProvider,
        limits: ScanLimits,
    ) -> Result<CheckedRawSnapshot<'i, 'a>, CheckedRawSnapshotFailure> {
        linux::snapshot_checked_with_coordination(
            self.data_root,
            invocation,
            expected,
            captured,
            provider.scope(),
            limits,
            Some(coordination_root),
        )
    }
    /// Read restored originals using an existing verifier-local coordination
    /// directory outside the data tree. Nothing is created or copied here.
    /// This is descriptive source evidence, never a RESTORE_OFF capability.
    pub fn snapshot_for_archive(
        &self,
        coordination_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
        linux::snapshot_for_archive(self.data_root, coordination_root, scope, limits)
    }

    /// Historical archive capture uses the existing external coordination backend
    /// and the caller's real checked invocation. This creates no control file.
    #[expect(
        clippy::result_large_err,
        reason = "Preserve actual checked scan failure custody."
    )]
    pub(crate) fn historical_snapshot_checked_in<'i, 'a>(
        &self,
        invocation: &'i crate::raw_custody::RawInvocation<'a>,
        expected: &crate::checked_close::Attempt,
        captured: &crate::checked_close::Attempt,
        coordination_root: &Path,
        provider: HistoricalRawProvider,
        limits: ScanLimits,
    ) -> Result<CheckedRawSnapshot<'i, 'a>, CheckedRawSnapshotFailure> {
        linux::snapshot_checked_with_coordination(
            self.data_root,
            invocation,
            expected,
            captured,
            provider.scope(),
            limits,
            Some(coordination_root),
        )
    }

    /// Borrows the caller's actual custody attempt. Binding remains unavailable.
    #[expect(
        clippy::result_large_err,
        reason = "Keep whole diagnostics without added failure-path allocations."
    )]
    pub fn snapshot_checked_in<'i, 'a>(
        &self,
        invocation: &'i crate::raw_custody::RawInvocation<'a>,
        expected: &crate::checked_close::Attempt,
        captured: &crate::checked_close::Attempt,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CheckedRawSnapshot<'i, 'a>, CheckedRawSnapshotFailure> {
        linux::snapshot_checked(
            self.data_root,
            invocation,
            expected,
            captured,
            scope,
            limits,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotStats {
    pub manifest_bytes: usize,
    pub manifest_records: usize,
    pub batches: usize,
    pub files: usize,
    pub retained_evidence_bytes: usize,
    pub open_descriptors: usize,
    pub peak_open_descriptors: usize,
}

pub struct CommittedRawSnapshot {
    scope: ApprovedRawScope,
    limits: ScanLimits,
    stats: SnapshotStats,
    manifest_bytes: Arc<Vec<u8>>,
    manifest_sha256: ContentHash,
    batches: Vec<CommittedRawBatch>,
    batch_index: Vec<(BatchId, usize)>,
    #[cfg(target_os = "linux")]
    _resources: linux::ResourceGuard,
    #[cfg(target_os = "linux")]
    _data_root: std::path::PathBuf,
}

impl CommittedRawSnapshot {
    #[cfg(target_os = "linux")]
    pub(crate) fn original_data_root(&self) -> &Path {
        &self._data_root
    }
    /// Recheck the actual retained descriptors, names, membership and bytes.
    /// This is read-only evidence custody, not execution or backup authority.
    pub fn revalidate(&self) -> Result<(), DailyEvidenceError> {
        #[cfg(target_os = "linux")]
        {
            linux::revalidate_snapshot(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DailyEvidenceError::UnsupportedPlatform)
        }
    }

    pub fn scope(&self) -> &ApprovedRawScope {
        &self.scope
    }

    pub fn limits(&self) -> &ScanLimits {
        &self.limits
    }

    pub fn stats(&self) -> SnapshotStats {
        self.stats
    }

    pub fn manifest_bytes(&self) -> &[u8] {
        self.manifest_bytes.as_slice()
    }

    pub fn manifest_sha256(&self) -> &ContentHash {
        &self.manifest_sha256
    }

    pub fn batches(&self) -> &[CommittedRawBatch] {
        &self.batches
    }

    pub fn lookup_exact(
        &self,
        reference: &SourceFileReference,
    ) -> Result<CommittedRawFile<'_>, DailyEvidenceError> {
        let (batch, index) =
            lookup_reference_index(&self.scope, &self.batch_index, reference, |index| {
                self.batches.get(index).map(CommittedRawBatch::entry)
            })?;
        Ok(CommittedRawFile {
            batch: &self.batches[batch],
            index,
        })
    }
}

pub struct CommittedRawBatch {
    entry: ManifestEntry,
    manifest: Arc<Vec<u8>>,
    manifest_line: Range<usize>,
    manifest_line_sha256: ContentHash,
    manifest_line_number: usize,
    files: Vec<CapturedLeaf>,
    batch_json: CapturedLeaf,
}

impl CommittedRawBatch {
    pub fn batch_id(&self) -> BatchId {
        self.entry().batch_id
    }

    pub fn batch_date(&self) -> TradingDate {
        self.entry().date
    }

    pub fn retrieved_at(&self) -> UtcTimestamp {
        self.entry().retrieved_at
    }

    pub fn mode(&self) -> FetchMode {
        self.entry().mode
    }

    pub fn manifest_line_bytes(&self) -> &[u8] {
        &self.manifest[self.manifest_line.clone()]
    }

    pub fn manifest_line_sha256(&self) -> &ContentHash {
        &self.manifest_line_sha256
    }

    pub fn manifest_line_number(&self) -> usize {
        self.manifest_line_number
    }

    pub fn batch_json_bytes(&self) -> &[u8] {
        self.batch_json.bytes.as_slice()
    }

    pub fn batch_json_sha256(&self) -> &ContentHash {
        &self.batch_json.content_hash
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = CommittedRawFile<'_>> + '_ {
        (0..self.files.len()).map(move |index| CommittedRawFile { batch: self, index })
    }

    pub(crate) fn entry(&self) -> &ManifestEntry {
        &self.entry
    }
}

pub struct CommittedRawFile<'snapshot> {
    batch: &'snapshot CommittedRawBatch,
    index: usize,
}

impl<'snapshot> CommittedRawFile<'snapshot> {
    pub fn batch(&self) -> &'snapshot CommittedRawBatch {
        self.batch
    }

    pub fn file_name(&self) -> &'snapshot str {
        &self.entry().file_name
    }

    pub fn kind(&self) -> ResponseKind {
        self.entry().kind
    }

    pub fn content_hash(&self) -> &'snapshot ContentHash {
        &self.batch.files[self.index].content_hash
    }

    pub fn size_bytes(&self) -> u64 {
        self.entry().size_bytes
    }

    pub fn bytes(&self) -> &'snapshot [u8] {
        self.batch.files[self.index].bytes.as_slice()
    }

    pub(crate) fn entry(&self) -> &'snapshot crate::storage::FileEntry {
        &self.batch.entry.files[self.index]
    }
}

struct CapturedLeaf {
    bytes: Vec<u8>,
    content_hash: ContentHash,
    #[cfg(target_os = "linux")]
    file: linux::OwnedSourceFile,
    #[cfg(target_os = "linux")]
    fingerprint: linux::Fingerprint,
    #[cfg(target_os = "linux")]
    parent_index: usize,
    #[cfg(target_os = "linux")]
    name: linux::NameRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyEvidenceError {
    UnsupportedPlatform,
    InvalidScope,
    InvalidLimits,
    ArithmeticOverflow,
    SourceScanLimit,
    AllocationFailed,
    LockBusy,
    PermissionDenied,
    Io,
    MissingControl,
    MissingCommittedEvidence,
    UnsafePath,
    UntrustedOwner,
    WritableNode,
    NonRegularFile,
    HardLinkedFile,
    SourceChanged,
    InvalidManifest,
    ScopeMismatch,
    ManifestConflict,
    InvalidBatchMetadata,
    BatchMetadataMismatch,
    InvalidFileName,
    DuplicateFile,
    UnexpectedBatchEntry,
    SizeMismatch,
    HashMismatch,
    NotCommitted,
    ReferenceMismatch,
}

impl DailyEvidenceError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "UNSUPPORTED_PLATFORM",
            Self::InvalidScope => "INVALID_SCOPE",
            Self::InvalidLimits => "INVALID_LIMITS",
            Self::ArithmeticOverflow => "ARITHMETIC_OVERFLOW",
            Self::SourceScanLimit => "SOURCE_SCAN_LIMIT",
            Self::AllocationFailed => "ALLOCATION_FAILED",
            Self::LockBusy => "LOCK_BUSY",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Io => "IO",
            Self::MissingControl => "MISSING_CONTROL",
            Self::MissingCommittedEvidence => "MISSING_COMMITTED_EVIDENCE",
            Self::UnsafePath => "UNSAFE_PATH",
            Self::UntrustedOwner => "UNTRUSTED_OWNER",
            Self::WritableNode => "WRITABLE_NODE",
            Self::NonRegularFile => "NON_REGULAR_FILE",
            Self::HardLinkedFile => "HARD_LINKED_FILE",
            Self::SourceChanged => "SOURCE_CHANGED",
            Self::InvalidManifest => "INVALID_MANIFEST",
            Self::ScopeMismatch => "SCOPE_MISMATCH",
            Self::ManifestConflict => "MANIFEST_CONFLICT",
            Self::InvalidBatchMetadata => "INVALID_BATCH_METADATA",
            Self::BatchMetadataMismatch => "BATCH_METADATA_MISMATCH",
            Self::InvalidFileName => "INVALID_FILE_NAME",
            Self::DuplicateFile => "DUPLICATE_FILE",
            Self::UnexpectedBatchEntry => "UNEXPECTED_BATCH_ENTRY",
            Self::SizeMismatch => "SIZE_MISMATCH",
            Self::HashMismatch => "HASH_MISMATCH",
            Self::NotCommitted => "NOT_COMMITTED",
            Self::ReferenceMismatch => "REFERENCE_MISMATCH",
        }
    }
}

impl fmt::Display for DailyEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for DailyEvidenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

fn lookup_reference_index<'s>(
    scope: &ApprovedRawScope,
    batch_index: &[(BatchId, usize)],
    reference: &SourceFileReference,
    entry_at: impl FnOnce(usize) -> Option<&'s ManifestEntry>,
) -> Result<(usize, usize), DailyEvidenceError> {
    if reference.file_name == "batch.json" || validate_file_name(&reference.file_name).is_err() {
        return Err(DailyEvidenceError::InvalidFileName);
    }
    if reference.provider != scope.provider() || reference.market != scope.market() {
        return Err(DailyEvidenceError::ScopeMismatch);
    }
    let index_position = batch_index
        .binary_search_by_key(&reference.batch_id, |(batch_id, _)| *batch_id)
        .map_err(|_| DailyEvidenceError::NotCommitted)?;
    let batch_index = batch_index[index_position].1;
    let entry = entry_at(batch_index).ok_or(DailyEvidenceError::NotCommitted)?;
    let file_index = entry
        .files
        .iter()
        .position(|file| file.file_name == reference.file_name)
        .ok_or(DailyEvidenceError::NotCommitted)?;
    if entry.files[file_index].content_hash != reference.content_hash
        || entry.retrieved_at != reference.retrieved_at
    {
        return Err(DailyEvidenceError::ReferenceMismatch);
    }
    Ok((batch_index, file_index))
}

fn validate_file_name(name: &str) -> Result<(), DailyEvidenceError> {
    if name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.bytes().any(|byte| byte.is_ascii_control())
        || name == "batch.json"
    {
        return Err(DailyEvidenceError::InvalidFileName);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct PendingBatch {
    entry: ManifestEntry,
    line: Range<usize>,
    line_hash: ContentHash,
    line_number: usize,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    #[cfg(test)]
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};
    use std::mem::MaybeUninit;
    use std::os::fd::AsFd;
    #[cfg(test)]
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    #[cfg(test)]
    use std::rc::Rc;
    #[cfg(test)]
    use std::sync::Mutex;

    use fs2::FileExt;
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, SeekFrom as RustixSeekFrom, fstat, open, openat,
        seek, statat,
    };
    use rustix::io::Errno;
    use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

    const DIRECTORY_BUFFER_BYTES: usize = 4096;
    const READ_BUFFER_BYTES: usize = 8192;

    use crate::checked_close::{Attempt, Failure, Gap, Owner, Report, Role};
    use crate::raw_custody::RawInvocation;
    use std::cell::RefCell as CustodyCell;
    use std::convert::Infallible;

    type LocalResult = Result<(), Failure<Infallible>>;

    /// Actual local comparisons, never a terminal completion or native-close receipt.
    pub struct SnapshotScopeResults {
        expected: LocalResult,
        captured: LocalResult,
        own: LocalResult,
        owners: Vec<LocalResult>,
    }

    impl SnapshotScopeResults {
        pub fn expected(&self) -> &Result<(), Failure<Infallible>> {
            &self.expected
        }
        pub fn captured(&self) -> &Result<(), Failure<Infallible>> {
            &self.captured
        }
        pub fn own(&self) -> &Result<(), Failure<Infallible>> {
            &self.own
        }
        pub fn owners(&self) -> &[Result<(), Failure<Infallible>>] {
            &self.owners
        }
        pub fn is_clean(&self) -> bool {
            self.expected.is_ok()
                & self.captured.is_ok()
                & self.own.is_ok()
                & self.owners.iter().all(Result::is_ok)
        }
        fn first_report(&self) -> Option<&Report> {
            self.expected
                .as_ref()
                .err()
                .or(self.captured.as_ref().err())
                .or(self.own.as_ref().err())
                .or_else(|| self.owners.iter().find_map(|result| result.as_ref().err()))
                .map(Failure::report)
        }
    }

    impl fmt::Debug for SnapshotScopeResults {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("SnapshotScopeResults")
                .field("clean", &self.is_clean())
                .field("owner_checks", &self.owners.len())
                .finish_non_exhaustive()
        }
    }

    fn compare_snapshot_scopes(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        captured: &Attempt,
        own: &Attempt,
        owners: &[Attempt],
    ) -> SnapshotScopeResults {
        // Each real comparison runs even when any earlier ledger is dirty.
        let expected = invocation.settle_local_in(expected, Ok::<(), Infallible>(()));
        let captured = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
        let own = invocation.settle_local_in(own, Ok::<(), Infallible>(()));
        let owners = owners
            .iter()
            .map(|owner| invocation.settle_local_in(owner, Ok::<(), Infallible>(())))
            .collect();
        SnapshotScopeResults {
            expected,
            captured,
            own,
            owners,
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SnapshotFirstKind {
        Scope,
        Binding,
        Policy,
        Execution,
    }

    enum SnapshotFirst {
        Scope(SnapshotScopeResults),
        Binding(Failure<Infallible>),
        Local(Failure<Infallible>),
        Policy(Failure<DailyEvidenceError>),
        Execution {
            original: Option<Box<SnapshotFirst>>,
            failure: crate::ExecutionGateFailure,
        },
    }

    impl SnapshotFirst {
        fn kind(&self) -> SnapshotFirstKind {
            match self {
                Self::Scope(_) | Self::Local(_) => SnapshotFirstKind::Scope,
                Self::Binding(_) => SnapshotFirstKind::Binding,
                Self::Policy(_) => SnapshotFirstKind::Policy,
                Self::Execution { original, .. } => original
                    .as_ref()
                    .map_or(SnapshotFirstKind::Execution, |first| first.kind()),
            }
        }
        fn report(&self) -> Option<&Report> {
            match self {
                Self::Scope(scope) => scope.first_report(),
                Self::Binding(failure) | Self::Local(failure) => Some(failure.report()),
                Self::Policy(failure) => Some(failure.report()),
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
        fn policy(&self) -> Option<&Failure<DailyEvidenceError>> {
            match self.primary() {
                Self::Policy(failure) => Some(failure),
                _ => None,
            }
        }
    }

    enum CheckedScanError {
        Policy(DailyEvidenceError),
        First(Box<SnapshotFirst>),
    }
    impl From<DailyEvidenceError> for CheckedScanError {
        fn from(error: DailyEvidenceError) -> Self {
            Self::Policy(error)
        }
    }

    /// A temporary returned read only; errors from the underlying reader expose no bytes.
    struct ReturnedLeafData {
        bytes: Option<Vec<u8>>,
        content_hash: Option<ContentHash>,
    }

    struct CheckedControl<'i, 'a> {
        invocation: &'i RawInvocation<'a>,
        expected: Attempt,
        captured: Attempt,
        own: Attempt,
        owners: CustodyCell<Vec<Attempt>>,
        returned_leaf: CustodyCell<Option<ReturnedLeafData>>,
    }

    impl<'i, 'a> CheckedControl<'i, 'a> {
        fn new(invocation: &'i RawInvocation<'a>, expected: &Attempt, captured: &Attempt) -> Self {
            Self {
                invocation,
                expected: expected.clone(),
                captured: captured.clone(),
                own: invocation.attempt().clone(),
                owners: CustodyCell::new(Vec::new()),
                returned_leaf: CustodyCell::new(None),
            }
        }
        fn compare(&self, expected: &Attempt, captured: &Attempt) -> SnapshotScopeResults {
            compare_snapshot_scopes(
                self.invocation,
                expected,
                captured,
                &self.own,
                &self.owners.borrow(),
            )
        }
        fn before(&self) -> Result<(), CheckedScanError> {
            let results = self.compare(&self.expected, &self.captured);
            if results.is_clean() {
                Ok(())
            } else {
                Err(CheckedScanError::First(Box::new(SnapshotFirst::Scope(
                    results,
                ))))
            }
        }
        fn normalize(&self, error: CheckedScanError) -> CheckedScanError {
            match error {
                CheckedScanError::Policy(error) => {
                    let result = self
                        .invocation
                        .settle_local_in(&self.expected, Err::<(), _>(error));
                    match result {
                        Err(failure) => {
                            CheckedScanError::First(Box::new(SnapshotFirst::Policy(failure)))
                        }
                        Ok(()) => unreachable!("an original policy error cannot become success"),
                    }
                }
                first => first,
            }
        }
        fn local(&self) -> LocalResult {
            self.invocation
                .settle_local_in(&self.own, Ok::<(), Infallible>(()))
        }
    }

    /// On unwind this records a gap only; real owners still make their own once-only close.
    struct SnapshotUnwind<'a> {
        attempt: &'a Attempt,
        armed: bool,
    }
    impl Drop for SnapshotUnwind<'_> {
        fn drop(&mut self) {
            if self.armed {
                self.attempt.gap(Gap::Abandoned);
            }
        }
    }

    pub(super) trait NativeFileHandle {
        fn file(&self) -> &File;
        fn retire(&mut self);
        #[cfg(test)]
        fn observe_identity(&self, stat: &rustix::fs::Stat);
    }

    impl NativeFileHandle for OwnedSourceFile {
        fn file(&self) -> &File {
            OwnedSourceFile::file(self)
        }
        fn retire(&mut self) {
            OwnedSourceFile::retire(self);
        }
        #[cfg(test)]
        fn observe_identity(&self, stat: &rustix::fs::Stat) {
            OwnedSourceFile::observe_identity(self, stat);
        }
    }

    // Both capture modes own the same immutable payload representation. The
    // checked variant still owns its original ledger-bound native descriptor.
    type CheckedHandle = OwnedSourceFile;

    struct LeafParts<H> {
        bytes: Vec<u8>,
        content_hash: ContentHash,
        file: H,
        fingerprint: Fingerprint,
        parent_index: usize,
        name: NameRange,
    }
    trait LeafAccess {
        type Handle: NativeFileHandle;
        fn bytes(&self) -> &Vec<u8>;
        fn hash(&self) -> &ContentHash;
        fn handle(&self) -> &Self::Handle;
        fn handle_mut(&mut self) -> &mut Self::Handle;
        fn fingerprint(&self) -> Fingerprint;
        fn parent_index(&self) -> usize;
        fn name(&self) -> NameRange;
    }
    impl LeafAccess for CapturedLeaf {
        type Handle = OwnedSourceFile;
        fn bytes(&self) -> &Vec<u8> {
            &self.bytes
        }
        fn hash(&self) -> &ContentHash {
            &self.content_hash
        }
        fn handle(&self) -> &OwnedSourceFile {
            &self.file
        }
        fn handle_mut(&mut self) -> &mut OwnedSourceFile {
            &mut self.file
        }
        fn fingerprint(&self) -> Fingerprint {
            self.fingerprint
        }
        fn parent_index(&self) -> usize {
            self.parent_index
        }
        fn name(&self) -> NameRange {
            self.name
        }
    }
    type CheckedLeaf = CapturedLeaf;

    struct BatchParts<L> {
        entry: ManifestEntry,
        manifest: Arc<Vec<u8>>,
        manifest_line: Range<usize>,
        manifest_line_sha256: ContentHash,
        manifest_line_number: usize,
        files: Vec<L>,
        batch_json: L,
    }
    trait BatchAccess {
        type Leaf: LeafAccess;
        fn entry(&self) -> &ManifestEntry;
        fn manifest(&self) -> &Arc<Vec<u8>>;
        fn line(&self) -> Range<usize>;
        fn line_hash(&self) -> &ContentHash;
        fn line_number(&self) -> usize;
        fn files(&self) -> &[Self::Leaf];
        fn files_mut(&mut self) -> &mut [Self::Leaf];
        fn metadata(&self) -> &Self::Leaf;
        fn metadata_mut(&mut self) -> &mut Self::Leaf;
    }
    impl BatchAccess for CommittedRawBatch {
        type Leaf = CapturedLeaf;
        fn entry(&self) -> &ManifestEntry {
            &self.entry
        }
        fn manifest(&self) -> &Arc<Vec<u8>> {
            &self.manifest
        }
        fn line(&self) -> Range<usize> {
            self.manifest_line.clone()
        }
        fn line_hash(&self) -> &ContentHash {
            &self.manifest_line_sha256
        }
        fn line_number(&self) -> usize {
            self.manifest_line_number
        }
        fn files(&self) -> &[CapturedLeaf] {
            &self.files
        }
        fn files_mut(&mut self) -> &mut [CapturedLeaf] {
            &mut self.files
        }
        fn metadata(&self) -> &CapturedLeaf {
            &self.batch_json
        }
        fn metadata_mut(&mut self) -> &mut CapturedLeaf {
            &mut self.batch_json
        }
    }
    type CheckedBatch = CommittedRawBatch;

    // This trait is private and has exactly the two selected implementations.
    trait SnapshotMode: Sized {
        type Handle: NativeFileHandle;
        type Leaf: LeafAccess<Handle = Self::Handle>;
        type Batch: BatchAccess<Leaf = Self::Leaf>;
        type Output;
        type Error: From<DailyEvidenceError>;
        fn before(&self) -> Result<(), Self::Error>;
        fn normalize(&self, error: Self::Error) -> Self::Error;
        fn adopt(&self, file: File, #[cfg(test)] role: DescriptorRole) -> Self::Handle;
        fn leaf(parts: LeafParts<Self::Handle>) -> Self::Leaf;
        fn batch(parts: BatchParts<Self::Leaf>) -> Self::Batch;
        fn output(parts: OutputParts<Self::Handle, Self::Batch>) -> Self::Output;
        fn returned_leaf(&self, data: ReturnedLeafData);
        fn capacity(&self, limits: &ScanLimits) -> Result<(), Self::Error>;
    }
    struct Legacy {
        attempt: Option<Attempt>,
    }
    impl Legacy {
        fn ordinary() -> Self {
            Self { attempt: None }
        }
        fn for_snapshot(snapshot: &CommittedRawSnapshot) -> Self {
            Self {
                attempt: snapshot
                    ._resources
                    ._manifest_file
                    .file
                    .checked
                    .as_ref()
                    .map(|owner| owner.attempt().clone()),
            }
        }
    }
    impl SnapshotMode for Legacy {
        type Handle = OwnedSourceFile;
        type Leaf = CapturedLeaf;
        type Batch = CommittedRawBatch;
        type Output = CommittedRawSnapshot;
        type Error = DailyEvidenceError;
        fn before(&self) -> Result<(), Self::Error> {
            if self.attempt.as_ref().is_none_or(Attempt::check) {
                Ok(())
            } else {
                Err(DailyEvidenceError::SourceChanged)
            }
        }
        fn normalize(&self, error: Self::Error) -> Self::Error {
            if let Some(attempt) = &self.attempt {
                attempt.gap(Gap::Abandoned);
            }
            error
        }
        fn adopt(&self, file: File, #[cfg(test)] role: DescriptorRole) -> Self::Handle {
            if let Some(attempt) = &self.attempt {
                let owner = attempt.adopt(file, Role::Source);
                #[cfg(test)]
                let observation = OpenObservation::register(owner.get(), role);
                OwnedSourceFile {
                    file: None,
                    checked: Some(owner),
                    #[cfg(test)]
                    observation,
                }
            } else {
                OwnedSourceFile::new(
                    file,
                    #[cfg(test)]
                    role,
                )
            }
        }
        fn leaf(p: LeafParts<OwnedSourceFile>) -> CapturedLeaf {
            CapturedLeaf {
                bytes: p.bytes,
                content_hash: p.content_hash,
                file: p.file,
                fingerprint: p.fingerprint,
                parent_index: p.parent_index,
                name: p.name,
            }
        }
        fn batch(p: BatchParts<CapturedLeaf>) -> CommittedRawBatch {
            CommittedRawBatch {
                entry: p.entry,
                manifest: p.manifest,
                manifest_line: p.manifest_line,
                manifest_line_sha256: p.manifest_line_sha256,
                manifest_line_number: p.manifest_line_number,
                files: p.files,
                batch_json: p.batch_json,
            }
        }
        fn output(p: OutputParts<OwnedSourceFile, CommittedRawBatch>) -> CommittedRawSnapshot {
            CommittedRawSnapshot {
                scope: p.scope,
                limits: p.limits,
                stats: p.stats,
                manifest_bytes: p.manifest_bytes,
                manifest_sha256: p.manifest_sha256,
                batches: p.batches,
                batch_index: p.batch_index,
                _resources: p.resources,
                _data_root: p.data_root,
            }
        }
        fn returned_leaf(&self, _data: ReturnedLeafData) {}
        fn capacity(&self, _limits: &ScanLimits) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    struct Checked<'c, 'i, 'a> {
        control: &'c CheckedControl<'i, 'a>,
    }
    impl SnapshotMode for Checked<'_, '_, '_> {
        type Handle = CheckedHandle;
        type Leaf = CheckedLeaf;
        type Batch = CheckedBatch;
        type Output = CheckedData;
        type Error = CheckedScanError;
        fn before(&self) -> Result<(), Self::Error> {
            self.control.before()
        }
        fn normalize(&self, error: Self::Error) -> Self::Error {
            self.control.normalize(error)
        }
        fn adopt(&self, file: File, #[cfg(test)] role: DescriptorRole) -> Self::Handle {
            // The native File is already adopted before any observation/registry callback.
            let owner = adopt_resource(&self.control.own, file, Role::Source);
            self.control
                .owners
                .borrow_mut()
                .push(owner.attempt().clone());
            #[cfg(test)]
            let observation = OpenObservation::register(owner.get(), role);
            OwnedSourceFile {
                file: None,
                checked: Some(owner),
                #[cfg(test)]
                observation,
            }
        }
        fn leaf(parts: LeafParts<CheckedHandle>) -> CheckedLeaf {
            Legacy::leaf(parts)
        }
        fn batch(parts: BatchParts<CheckedLeaf>) -> CheckedBatch {
            Legacy::batch(parts)
        }
        fn output(parts: OutputParts<CheckedHandle, CheckedBatch>) -> CheckedData {
            CheckedData::Complete(Legacy::output(parts))
        }
        fn returned_leaf(&self, data: ReturnedLeafData) {
            *self.control.returned_leaf.borrow_mut() = Some(data);
        }
        fn capacity(&self, limits: &ScanLimits) -> Result<(), Self::Error> {
            let values = limits.values;
            checked_capacity(
                values.max_open_descriptors,
                std::mem::size_of::<DirectoryNode<CheckedHandle>>(),
            )?;
            checked_capacity(values.max_files, std::mem::size_of::<CheckedLeaf>())?;
            checked_capacity(values.max_batches, std::mem::size_of::<CheckedBatch>())?;
            checked_capacity(values.max_open_descriptors, std::mem::size_of::<Attempt>())?;
            checked_capacity(
                values.max_open_descriptors,
                std::mem::size_of::<LocalResult>(),
            )?;
            Ok(())
        }
    }

    fn effect<M: SnapshotMode, T>(
        mode: &M,
        operation: impl FnOnce() -> Result<T, DailyEvidenceError>,
    ) -> Result<T, M::Error> {
        mode.before()?;
        operation().map_err(Into::into)
    }

    fn preserve_before_retire<M: SnapshotMode, T>(
        mode: &M,
        result: Result<T, M::Error>,
        retire: impl FnOnce(),
    ) -> Result<T, M::Error> {
        let failed = result.is_err();
        retain_before_cleanup(
            result,
            |error| mode.normalize(error),
            || {
                if failed {
                    retire();
                }
            },
        )
    }

    /// The original result is owned before cleanup, including errors with no Clone/Debug bound.
    fn retain_before_cleanup<T, E, F>(
        result: Result<T, E>,
        normalize: impl FnOnce(E) -> F,
        cleanup: impl FnOnce(),
    ) -> Result<T, F> {
        let result = result.map_err(normalize);
        cleanup();
        result
    }

    fn capture_returned<T, E>(
        slot: &mut Option<T>,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<(), E> {
        let result = operation();
        match result {
            Ok(value) => {
                *slot = Some(value);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn append_completed<T>(values: &mut Vec<T>, value: T) {
        values.push(value);
    }

    fn adopt_resource<R: crate::checked_close::Close>(
        attempt: &Attempt,
        resource: R,
        role: Role,
    ) -> Owner<R> {
        attempt.adopt(resource, role)
    }

    fn install_pending<T>(slot: &mut Option<T>, value: T) {
        *slot = Some(value);
    }

    struct OriginalOutcome<T, E>(Result<T, E>);
    impl<T, E> fmt::Debug for OriginalOutcome<T, E> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("OriginalOutcome")
                .field("ok", &self.0.is_ok())
                .finish_non_exhaustive()
        }
    }

    fn retire_batch<B: BatchAccess>(batch: &mut B) {
        for leaf in batch.files_mut() {
            leaf.handle_mut().retire();
        }
        batch.metadata_mut().handle_mut().retire();
    }

    fn retire_construction<T, B, D, N, L>(
        temporary: impl FnOnce() -> T,
        batches: impl FnOnce() -> B,
        manifest: impl FnOnce() -> D,
        directories: impl FnOnce() -> N,
        names: impl FnOnce(),
        failure_hook: impl FnOnce(),
        lock: impl FnOnce() -> L,
    ) {
        temporary();
        batches();
        manifest();
        directories();
        names();
        failure_hook();
        lock();
    }

    struct OutputParts<H: NativeFileHandle, B> {
        data_root: std::path::PathBuf,
        scope: ApprovedRawScope,
        limits: ScanLimits,
        stats: SnapshotStats,
        manifest_bytes: Arc<Vec<u8>>,
        manifest_sha256: ContentHash,
        batches: Vec<B>,
        batch_index: Vec<(BatchId, usize)>,
        resources: ResourceGuard<H>,
    }

    struct CaptureBatchState<H: NativeFileHandle, L: LeafAccess<Handle = H>> {
        pending: Option<PendingBatch>,
        metadata: Option<HeldFile<H>>,
        metadata_bytes: Option<Vec<u8>>,
        batch_json: Option<L>,
        files: Vec<L>,
    }
    impl<H: NativeFileHandle, L: LeafAccess<Handle = H>> CaptureBatchState<H, L> {
        fn new(pending: PendingBatch) -> Self {
            Self {
                pending: Some(pending),
                metadata: None,
                metadata_bytes: None,
                batch_json: None,
                files: Vec::new(),
            }
        }
        fn retire(&mut self) {
            for leaf in &mut self.files {
                leaf.handle_mut().retire();
            }
            if let Some(metadata) = &mut self.batch_json {
                metadata.handle_mut().retire();
            }
            if let Some(metadata) = &mut self.metadata {
                metadata.file.retire();
            }
        }
    }

    struct SnapshotBuildState<
        H: NativeFileHandle,
        L: LeafAccess<Handle = H>,
        B: BatchAccess<Leaf = L>,
    > {
        scan: Option<ScanContext<H>>,
        manifest_captured: Option<Vec<u8>>,
        manifest_bytes: Option<Arc<Vec<u8>>>,
        manifest_hash: Option<ContentHash>,
        parsed: Option<ParsedManifest>,
        remaining: Option<std::vec::IntoIter<PendingBatch>>,
        current: Option<CaptureBatchState<H, L>>,
        batches: Vec<B>,
        stats: Option<SnapshotStats>,
    }
    impl<H: NativeFileHandle, L: LeafAccess<Handle = H>, B: BatchAccess<Leaf = L>> Default
        for SnapshotBuildState<H, L, B>
    {
        fn default() -> Self {
            Self {
                scan: None,
                manifest_captured: None,
                manifest_bytes: None,
                manifest_hash: None,
                parsed: None,
                remaining: None,
                current: None,
                batches: Vec::new(),
                stats: None,
            }
        }
    }
    impl<H: NativeFileHandle, L: LeafAccess<Handle = H>, B: BatchAccess<Leaf = L>>
        SnapshotBuildState<H, L, B>
    {
        fn retire(&mut self) {
            // A failed current helper retires first; previous batches then precede the controls.
            let current = &mut self.current;
            let batches = &mut self.batches;
            if let Some(scan) = &mut self.scan {
                retire_resources(
                    &mut scan.resources,
                    || {
                        if let Some(current) = current {
                            current.retire();
                        }
                    },
                    || {
                        for batch in batches {
                            retire_batch(batch);
                        }
                    },
                );
            }
        }
    }

    #[expect(
        clippy::large_enum_variant,
        reason = "Avoid capture-time allocation while retaining complete and partial owned graphs."
    )]
    enum CheckedData {
        Complete(CommittedRawSnapshot),
        Partial(SnapshotBuildState<CheckedHandle, CheckedLeaf, CheckedBatch>),
    }
    impl CheckedData {
        fn retire(&mut self) {
            match self {
                Self::Complete(parts) => {
                    for batch in &mut parts.batches {
                        retire_batch(batch);
                    }
                    parts._resources.retire();
                }
                Self::Partial(state) => state.retire(),
            }
        }
    }

    // The same class-ordered retirement seam is used by construction and its ordinary models.
    fn retire_resources<H: NativeFileHandle>(
        resources: &mut ConstructionResources<H>,
        temporary: impl FnOnce(),
        batches: impl FnOnce(),
    ) {
        let ConstructionResources {
            manifest_file,
            directories,
            names,
            transferred,
            lock_file,
            ..
        } = resources;
        retire_construction(
            temporary,
            batches,
            || {
                if let Some(held) = manifest_file {
                    held.file.retire();
                }
                drop(manifest_file.take());
            },
            || {
                for directory in directories.iter_mut() {
                    directory.file.retire();
                }
                directories.clear();
            },
            || names.bytes.clear(),
            || {
                if !*transferred {
                    #[cfg(test)]
                    observe_resource(ObservationEvent::ConstructionCleanupBeforeLockRelease);
                    let _ = hit_phase(CapturePhase::BeforeLockRelease);
                }
            },
            || {
                if let Some(held) = lock_file {
                    held.file.retire();
                }
                drop(lock_file.take());
            },
        );
        *transferred = true;
    }
    impl<H: NativeFileHandle> ResourceGuard<H> {
        fn retire(&mut self) {
            let Self {
                _manifest_file,
                _directories,
                _names,
                _lock_file,
            } = self;
            retire_construction(
                || {},
                || {},
                || _manifest_file.file.retire(),
                || {
                    for directory in _directories {
                        directory.file.retire();
                    }
                },
                || _names.bytes.clear(),
                || {},
                || _lock_file.file.retire(),
            );
        }
    }
    impl<H: NativeFileHandle> Drop for ResourceGuard<H> {
        fn drop(&mut self) {
            self.retire();
        }
    }
    impl<H: NativeFileHandle, L: LeafAccess<Handle = H>> Drop for CaptureBatchState<H, L> {
        fn drop(&mut self) {
            self.retire();
            // Match the original lexical owner destruction, not a universal reverse rule.
            self.files.clear();
            drop(self.batch_json.take());
            drop(self.metadata.take());
        }
    }
    impl<H: NativeFileHandle, L: LeafAccess<Handle = H>, B: BatchAccess<Leaf = L>> Drop
        for SnapshotBuildState<H, L, B>
    {
        fn drop(&mut self) {
            drop(self.current.take());
            self.batches.clear();
            drop(self.scan.take());
        }
    }
    struct PendingLeaf<H> {
        file: Option<H>,
        bytes: Option<Vec<u8>>,
        content_hash: Option<ContentHash>,
        fingerprint: Option<Fingerprint>,
        name: Option<NameRange>,
    }
    impl<H> Default for PendingLeaf<H> {
        fn default() -> Self {
            Self {
                file: None,
                bytes: None,
                content_hash: None,
                fingerprint: None,
                name: None,
            }
        }
    }

    fn shared_lock_result(result: std::io::Result<()>) -> Result<(), DailyEvidenceError> {
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                Err(DailyEvidenceError::LockBusy)
            }
            Err(error) => Err(io_error(error)),
        }
    }

    pub(super) fn snapshot(
        data_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
        let mut state = SnapshotBuildState::default();
        build_snapshot(&Legacy::ordinary(), data_root, scope, limits, &mut state)
    }

    pub(super) fn snapshot_for_archive(
        data_root: &Path,
        coordination_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
        if coordination_root.starts_with(data_root) || data_root.starts_with(coordination_root) {
            return Err(DailyEvidenceError::UnsafePath);
        }
        for path in [data_root, coordination_root] {
            let bytes = path.as_os_str().as_bytes();
            if !bytes.starts_with(b"/")
                || bytes[1..]
                    .split(|byte| *byte == b'/')
                    .any(|part| part.is_empty() || part == b"." || part == b"..")
            {
                return Err(DailyEvidenceError::UnsafePath);
            }
        }
        let mut state = SnapshotBuildState::default();
        let snapshot = build_snapshot_with_coordination(
            &Legacy::ordinary(),
            data_root,
            scope,
            limits,
            &mut state,
            Some(coordination_root),
        )?;
        revalidate_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub(super) fn revalidate_snapshot(
        snapshot: &CommittedRawSnapshot,
    ) -> Result<(), DailyEvidenceError> {
        let mode = Legacy::for_snapshot(snapshot);
        mode.before()?;
        let held = &snapshot._resources;
        let uid = snapshot.scope.trusted_raw_owner_uid;
        revalidate_archive_metadata(snapshot)?;
        revalidate_directories(&mode, &held._directories, &held._names, uid)?;
        revalidate_held_lock(
            &mode,
            &held._lock_file,
            &held._directories,
            &held._names,
            uid,
        )?;
        for batch in &snapshot.batches {
            let mut seen = Vec::new();
            seen.try_reserve_exact(batch.entry.files.len())
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            seen.resize(batch.entry.files.len(), false);
            verify_batch_membership(
                &mode,
                &held._directories[batch.batch_json.parent_index],
                &batch.entry,
                &mut seen,
                DailyEvidenceError::SourceChanged,
            )?;
        }
        final_byte_checks(
            &mode,
            &held._manifest_file,
            snapshot.manifest_bytes(),
            &snapshot.batches,
            &held._directories,
            &held._names,
            uid,
        )?;
        revalidate_all_batch_files(
            &mode,
            &snapshot.batches,
            &held._directories,
            &held._names,
            uid,
        )?;
        revalidate_held_lock(
            &mode,
            &held._lock_file,
            &held._directories,
            &held._names,
            uid,
        )?;
        revalidate_directories(&mode, &held._directories, &held._names, uid)?;
        revalidate_archive_metadata(snapshot)
    }

    fn revalidate_archive_metadata(
        snapshot: &CommittedRawSnapshot,
    ) -> Result<(), DailyEvidenceError> {
        let mode = Legacy::for_snapshot(snapshot);
        mode.before()?;
        let held = &snapshot._resources;
        let uid = snapshot.scope.trusted_raw_owner_uid;
        if held._names.get(held._lock_file.name)?.as_bytes() != b"commit.lock" {
            revalidate_archive_coordination(&mode, held, uid)?;
            for directory in &held._directories {
                no_archive_acl(&mode, directory.file.file())?;
            }
            no_archive_acl(&mode, held._manifest_file.file.file())?;
            for batch in &snapshot.batches {
                no_archive_acl(&mode, batch.batch_json.file.file())?;
                for file in &batch.files {
                    no_archive_acl(&mode, file.file.file())?;
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn read_exact_captured(
        reader: impl Read,
        expected: usize,
        byte_cap: usize,
    ) -> Result<Vec<u8>, DailyEvidenceError> {
        read_exact_captured_in(&Legacy::ordinary(), reader, expected, byte_cap)
    }

    /// Whole first source, returned scan data, and real later local cleanup stay separate.
    pub struct CheckedRawSnapshotFailure {
        first: SnapshotFirst,
        data: CheckedData,
        returned_leaf: Option<ReturnedLeafData>,
        completion: CheckedRawSnapshotCompletion,
    }

    impl CheckedRawSnapshotFailure {
        pub fn kind(&self) -> SnapshotFirstKind {
            self.first.kind()
        }
        pub fn source_report(&self) -> Option<&Report> {
            self.first.report()
        }
        pub fn policy(&self) -> Option<&Failure<DailyEvidenceError>> {
            self.first.policy()
        }
        pub fn scope_failure(&self) -> Option<&SnapshotScopeResults> {
            match self.first.primary() {
                SnapshotFirst::Scope(scope) => Some(scope),
                _ => None,
            }
        }
        pub fn binding_failure(&self) -> Option<&Failure<Infallible>> {
            match self.first.primary() {
                SnapshotFirst::Binding(failure) => Some(failure),
                _ => None,
            }
        }
        pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
            self.first.execution()
        }
        pub fn retained(&self) -> CheckedRawSnapshotView<'_> {
            CheckedRawSnapshotView {
                data: &self.data,
                returned_leaf: self.returned_leaf.as_ref(),
            }
        }
        pub fn completion(&self) -> &CheckedRawSnapshotCompletion {
            &self.completion
        }
    }
    impl fmt::Debug for CheckedRawSnapshotFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedRawSnapshotFailure")
                .field("kind", &self.kind())
                .field("complete", &self.retained().is_complete())
                .finish_non_exhaustive()
        }
    }

    pub struct CheckedRawSnapshotUseFailure {
        scope: SnapshotScopeResults,
    }
    impl CheckedRawSnapshotUseFailure {
        pub fn scope(&self) -> &SnapshotScopeResults {
            &self.scope
        }
        pub fn source_report(&self) -> Option<&Report> {
            self.scope.first_report()
        }
    }
    impl fmt::Debug for CheckedRawSnapshotUseFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedRawSnapshotUseFailure")
                .finish_non_exhaustive()
        }
    }

    /// Contains only real local results; the borrowed caller still owns terminal completion.
    pub struct CheckedRawSnapshotCompletion {
        before_close: SnapshotScopeResults,
        after_close: SnapshotScopeResults,
        local: LocalResult,
        data: Option<CheckedData>,
        returned_leaf: Option<ReturnedLeafData>,
    }
    impl CheckedRawSnapshotCompletion {
        pub fn before_close(&self) -> &SnapshotScopeResults {
            &self.before_close
        }
        pub fn after_close(&self) -> &SnapshotScopeResults {
            &self.after_close
        }
        pub fn local(&self) -> &Result<(), Failure<Infallible>> {
            &self.local
        }
        pub fn is_clean(&self) -> bool {
            self.before_close.is_clean() & self.after_close.is_clean() & self.local.is_ok()
        }
        /// Successful scan plus late cleanup uncertainty has no invented first source report.
        pub fn source_report(&self) -> Option<&Report> {
            None
        }
        pub fn retained(&self) -> Option<CheckedRawSnapshotView<'_>> {
            self.data.as_ref().map(|data| CheckedRawSnapshotView {
                data,
                returned_leaf: self.returned_leaf.as_ref(),
            })
        }
    }
    impl fmt::Debug for CheckedRawSnapshotCompletion {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedRawSnapshotCompletion")
                .field("clean", &self.is_clean())
                .field("data_present", &self.data.is_some())
                .finish_non_exhaustive()
        }
    }

    pub struct CheckedRawSnapshot<'i, 'a> {
        control: CheckedControl<'i, 'a>,
        data: CheckedData,
        armed: bool,
    }

    /// These are the actual checks on both sides of one lexical borrow. They
    /// are retained on failure together with the original callback error.
    pub(crate) struct RawBorrowChecks {
        pub(crate) scope: SnapshotScopeResults,
        pub(crate) native: Result<(), crate::close_gate::NativeAdmissionFailure>,
        pub(crate) originals: Result<(), DailyEvidenceError>,
    }
    impl RawBorrowChecks {
        fn clean(&self) -> bool {
            self.scope.is_clean() & self.native.is_ok() & self.originals.is_ok()
        }
    }
    pub(crate) struct CheckedRawBorrowFailure<E> {
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "Retain pre-check failure evidence for callers.")
        )]
        pub(crate) before: Vec<RawBorrowChecks>,
        pub(crate) callback: Option<Result<(), E>>,
        #[cfg_attr(
            not(test),
            expect(dead_code, reason = "Retain post-check failure evidence for callers.")
        )]
        pub(crate) after: Vec<RawBorrowChecks>,
    }

    /// Lend the actual complete capture, including the original checked File
    /// owners. No snapshot, manifest, byte buffer or descriptor is reconstructed
    /// or cloned. The HRTB callback cannot export the borrowed snapshot in its
    /// result. Every holder stays alive and every post-check runs on error.
    pub(crate) fn with_checked_raw_originals<E>(
        sources: &[CheckedRawSnapshot<'_, '_>],
        expected: &Attempt,
        captured: &Attempt,
        use_originals: impl for<'s> FnOnce(&[&'s CommittedRawSnapshot]) -> Result<(), E>,
    ) -> Result<(), CheckedRawBorrowFailure<E>> {
        let mut unwind = SnapshotUnwind {
            attempt: expected,
            armed: true,
        };
        let mut captured_unwind = SnapshotUnwind {
            attempt: captured,
            armed: true,
        };
        let check = |source: &CheckedRawSnapshot<'_, '_>| RawBorrowChecks {
            scope: source.control.compare(expected, captured),
            native: crate::close_gate::require_native(source.control.invocation, expected),
            originals: source.revalidate_originals_in(expected, captured),
        };
        // Refuse an unbounded caller slice before allocating result vectors.
        // Still visit every supplied owner on both sides; none is silently
        // omitted from custody settlement merely because the bound failed.
        if sources.len() > 7 {
            for source in sources {
                let _ = check(source);
            }
            for source in sources {
                let _ = check(source);
            }
            expected.gap(Gap::Abandoned);
            captured.gap(Gap::Abandoned);
            return Err(CheckedRawBorrowFailure {
                before: Vec::new(),
                callback: None,
                after: Vec::new(),
            });
        }
        let before: Vec<_> = sources.iter().map(check).collect();
        let valid = !sources.is_empty()
            && before.iter().all(RawBorrowChecks::clean)
            && sources
                .iter()
                .all(|s| matches!(s.data, CheckedData::Complete(_)));
        let callback = if valid {
            let originals: Vec<_> = sources
                .iter()
                .map(|source| match &source.data {
                    CheckedData::Complete(original) => original,
                    CheckedData::Partial(_) => unreachable!("complete capture checked above"),
                })
                .collect();
            Some(use_originals(&originals))
        } else {
            None
        };
        let after: Vec<_> = sources.iter().map(check).collect();
        if !valid
            || callback.as_ref().is_none_or(Result::is_err)
            || !after.iter().all(RawBorrowChecks::clean)
        {
            expected.gap(Gap::Abandoned);
            captured.gap(Gap::Abandoned);
            return Err(CheckedRawBorrowFailure {
                before,
                callback,
                after,
            });
        }
        unwind.armed = false;
        captured_unwind.armed = false;
        Ok(())
    }
    impl CheckedRawSnapshot<'_, '_> {
        /// Revalidate every retained original, not just the caller's ledger.
        /// Failure is recorded on the real attempt before any view can be reused.
        pub(crate) fn revalidate_originals_in(
            &self,
            expected: &Attempt,
            captured: &Attempt,
        ) -> Result<(), DailyEvidenceError> {
            if !self.control.compare(expected, captured).is_clean() {
                return Err(DailyEvidenceError::SourceChanged);
            }
            let mode = Checked {
                control: &self.control,
            };
            let result = (|| -> Result<(), CheckedScanError> {
                let CheckedData::Complete(parts) = &self.data else {
                    return Err(DailyEvidenceError::NotCommitted.into());
                };
                let held = &parts._resources;
                let uid = parts.scope.trusted_raw_owner_uid;
                let archive = held._names.get(held._lock_file.name)?.as_bytes() != b"commit.lock";
                if !archive || uid != 10001 {
                    return Err(DailyEvidenceError::UntrustedOwner.into());
                }
                if archive {
                    revalidate_archive_coordination(&mode, held, uid)?;
                }
                revalidate_directories(&mode, &held._directories, &held._names, uid)?;
                revalidate_held_lock(
                    &mode,
                    &held._lock_file,
                    &held._directories,
                    &held._names,
                    uid,
                )?;
                for batch in &parts.batches {
                    let mut seen = Vec::new();
                    seen.try_reserve_exact(batch.entry().files.len())
                        .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                    seen.resize(batch.entry().files.len(), false);
                    verify_batch_membership(
                        &mode,
                        &held._directories[batch.metadata().parent_index()],
                        batch.entry(),
                        &mut seen,
                        DailyEvidenceError::SourceChanged,
                    )?;
                    if archive {
                        historical_original_mode(&mode, batch.metadata().handle().file(), 0o600)?;
                        no_archive_acl(&mode, batch.metadata().handle().file())?;
                        for file in BatchAccess::files(batch) {
                            historical_original_mode(&mode, file.handle().file(), 0o600)?;
                            no_archive_acl(&mode, file.handle().file())?;
                        }
                    }
                }
                if archive {
                    for directory in &held._directories {
                        if directory.raw_protected {
                            historical_original_mode(&mode, directory.file.file(), 0o700)?;
                        }
                        no_archive_acl(&mode, directory.file.file())?;
                    }
                    historical_original_mode(&mode, held._manifest_file.file.file(), 0o600)?;
                    no_archive_acl(&mode, held._manifest_file.file.file())?;
                }
                final_byte_checks(
                    &mode,
                    &held._manifest_file,
                    &parts.manifest_bytes,
                    &parts.batches,
                    &held._directories,
                    &held._names,
                    uid,
                )?;
                revalidate_all_batch_files(
                    &mode,
                    &parts.batches,
                    &held._directories,
                    &held._names,
                    uid,
                )?;
                revalidate_held_lock(
                    &mode,
                    &held._lock_file,
                    &held._directories,
                    &held._names,
                    uid,
                )?;
                revalidate_directories(&mode, &held._directories, &held._names, uid)?;
                if archive {
                    revalidate_archive_coordination(&mode, held, uid)?;
                }
                Ok(())
            })();
            if result.is_err() {
                self.control.own.gap(Gap::Abandoned);
                return Err(DailyEvidenceError::SourceChanged);
            }
            Ok(())
        }
        #[expect(
            clippy::result_large_err,
            reason = "Keep all actual scope results without added error-path allocations."
        )]
        pub fn view_in(
            &self,
            expected: &Attempt,
            captured: &Attempt,
        ) -> Result<CheckedRawSnapshotView<'_>, CheckedRawSnapshotUseFailure> {
            let scope = self.control.compare(expected, captured);
            if !scope.is_clean() {
                return Err(CheckedRawSnapshotUseFailure { scope });
            }
            Ok(CheckedRawSnapshotView {
                data: &self.data,
                returned_leaf: None,
            })
        }
        pub fn finish_local(mut self) -> CheckedRawSnapshotCompletion {
            let before_close = self
                .control
                .compare(&self.control.expected, &self.control.captured);
            self.data.retire();
            let after_close = self
                .control
                .compare(&self.control.expected, &self.control.captured);
            let local = self.control.local();
            self.armed = false;
            // Moving through replace is data transport only; it creates no evidence.
            let data = std::mem::replace(
                &mut self.data,
                CheckedData::Partial(SnapshotBuildState::default()),
            );
            let returned_leaf = self.control.returned_leaf.borrow_mut().take();
            CheckedRawSnapshotCompletion {
                before_close,
                after_close,
                local,
                data: Some(data),
                returned_leaf,
            }
        }
    }

    // Installed Raw class policy, confined to the historical archive adapter.
    // Ordinary RawStore and Daily scope semantics remain unchanged.
    fn historical_original_mode<M: SnapshotMode>(
        mode: &M,
        file: &File,
        permissions: u32,
    ) -> Result<(), M::Error> {
        let stat = effect(mode, || fstat(file).map_err(errno_io))?;
        if stat.st_uid != 10001 || stat.st_gid != 10001 {
            return Err(DailyEvidenceError::UntrustedOwner.into());
        }
        if stat.st_mode & 0o7777 != permissions {
            return Err(DailyEvidenceError::WritableNode.into());
        }
        Ok(())
    }
    impl Drop for CheckedRawSnapshot<'_, '_> {
        fn drop(&mut self) {
            if self.armed {
                self.control.own.gap(Gap::Abandoned);
            }
            self.data.retire();
        }
    }
    impl fmt::Debug for CheckedRawSnapshot<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedRawSnapshot")
                .field("complete", &true)
                .finish_non_exhaustive()
        }
    }

    pub struct CheckedRawSnapshotView<'s> {
        data: &'s CheckedData,
        returned_leaf: Option<&'s ReturnedLeafData>,
    }
    impl<'s> CheckedRawSnapshotView<'s> {
        pub(crate) fn data_root(&self) -> Option<&'s Path> {
            match self.data {
                CheckedData::Complete(parts) => Some(&parts._data_root),
                _ => None,
            }
        }
        /// Completeness of the original scan data, independently of custody or acceptance.
        pub fn is_complete(&self) -> bool {
            matches!(self.data, CheckedData::Complete(_))
        }
        pub fn scope(&self) -> Option<&'s ApprovedRawScope> {
            match self.data {
                CheckedData::Complete(parts) => Some(&parts.scope),
                CheckedData::Partial(state) => state.scan.as_ref().map(|scan| &scan.scope),
            }
        }
        pub fn limits(&self) -> Option<&'s ScanLimits> {
            match self.data {
                CheckedData::Complete(parts) => Some(&parts.limits),
                CheckedData::Partial(state) => state.scan.as_ref().map(|scan| &scan.limits),
            }
        }
        pub fn stats(&self) -> Option<SnapshotStats> {
            match self.data {
                CheckedData::Complete(parts) => Some(parts.stats),
                CheckedData::Partial(state) => state.stats,
            }
        }
        pub fn manifest_bytes(&self) -> Option<&'s [u8]> {
            match self.data {
                CheckedData::Complete(parts) => Some(parts.manifest_bytes.as_slice()),
                CheckedData::Partial(state) => state
                    .manifest_bytes
                    .as_ref()
                    .map(|bytes| bytes.as_slice())
                    .or(state.manifest_captured.as_deref()),
            }
        }
        pub fn manifest_sha256(&self) -> Option<&'s ContentHash> {
            match self.data {
                CheckedData::Complete(parts) => Some(&parts.manifest_sha256),
                CheckedData::Partial(state) => state.manifest_hash.as_ref(),
            }
        }
        /// None means the original parser did not return. Some(0) is a returned empty parse.
        pub fn parsed_batch_count(&self) -> Option<usize> {
            match self.data {
                CheckedData::Complete(parts) => Some(parts.batches.len()),
                CheckedData::Partial(state) => {
                    state.parsed.as_ref().map(|parsed| parsed.batch_index.len())
                }
            }
        }
        pub fn parsed_entries(&self) -> impl Iterator<Item = &'s ManifestEntry> + '_ {
            let complete = self.batches_raw().iter().map(|batch| batch.entry());
            let (current, remaining) = match self.data {
                CheckedData::Complete(_) => (None, None),
                CheckedData::Partial(state) => (
                    state
                        .current
                        .as_ref()
                        .and_then(|current| current.pending.as_ref()),
                    state.remaining.as_ref(),
                ),
            };
            // Before iteration begins, the original ParsedManifest still owns all pending rows.
            let pending = match self.data {
                CheckedData::Partial(state) => state
                    .parsed
                    .as_ref()
                    .map(|parsed| parsed.pending.as_slice()),
                _ => None,
            };
            complete
                .chain(current.into_iter().map(|pending| &pending.entry))
                .chain(
                    remaining
                        .into_iter()
                        .flat_map(|rows| rows.as_slice().iter().map(|row| &row.entry)),
                )
                .chain(pending.into_iter().flatten().map(|row| &row.entry))
        }
        fn batches_raw(&self) -> &'s [CheckedBatch] {
            match self.data {
                CheckedData::Complete(parts) => &parts.batches,
                CheckedData::Partial(state) => &state.batches,
            }
        }
        pub fn batches(&self) -> impl ExactSizeIterator<Item = CheckedRawBatchView<'s>> + '_ {
            self.batches_raw()
                .iter()
                .map(|batch| CheckedRawBatchView { batch })
        }
        pub fn partial_batch(&self) -> Option<CheckedRawPartialBatchView<'s>> {
            match self.data {
                CheckedData::Partial(state) => state
                    .current
                    .as_ref()
                    .map(|state| CheckedRawPartialBatchView { state }),
                _ => None,
            }
        }
        pub fn returned_temporary_bytes(&self) -> Option<&'s [u8]> {
            self.returned_leaf.and_then(|data| data.bytes.as_deref())
        }
        pub fn returned_temporary_hash(&self) -> Option<&'s ContentHash> {
            self.returned_leaf
                .and_then(|data| data.content_hash.as_ref())
        }
        pub fn lookup_exact(
            &self,
            reference: &SourceFileReference,
        ) -> Result<CheckedRawFileView<'s>, DailyEvidenceError> {
            let CheckedData::Complete(parts) = self.data else {
                return Err(DailyEvidenceError::NotCommitted);
            };
            let (batch, index) =
                lookup_reference_index(&parts.scope, &parts.batch_index, reference, |index| {
                    parts.batches.get(index).map(|batch| batch.entry())
                })?;
            Ok(CheckedRawFileView {
                batch: &parts.batches[batch],
                index,
            })
        }
    }

    pub struct CheckedRawBatchView<'s> {
        batch: &'s CheckedBatch,
    }
    impl<'s> CheckedRawBatchView<'s> {
        pub(crate) fn entry(&self) -> &'s ManifestEntry {
            self.batch.entry()
        }

        pub fn batch_id(&self) -> BatchId {
            self.batch.entry().batch_id
        }
        pub fn batch_date(&self) -> TradingDate {
            self.batch.entry().date
        }
        pub fn retrieved_at(&self) -> UtcTimestamp {
            self.batch.entry().retrieved_at
        }
        pub fn mode(&self) -> FetchMode {
            self.batch.entry().mode
        }
        pub fn manifest_line_bytes(&self) -> &'s [u8] {
            &self.batch.manifest()[self.batch.line()]
        }
        pub fn manifest_line_sha256(&self) -> &'s ContentHash {
            self.batch.line_hash()
        }
        pub fn manifest_line_number(&self) -> usize {
            self.batch.line_number()
        }
        pub fn batch_json_bytes(&self) -> &'s [u8] {
            self.batch.metadata().bytes()
        }
        pub fn batch_json_sha256(&self) -> &'s ContentHash {
            self.batch.metadata().hash()
        }
        pub fn files(&self) -> impl ExactSizeIterator<Item = CheckedRawFileView<'s>> + '_ {
            (0..self.batch.files().len()).map(move |index| CheckedRawFileView {
                batch: self.batch,
                index,
            })
        }
    }
    pub struct CheckedRawFileView<'s> {
        batch: &'s CheckedBatch,
        index: usize,
    }
    impl<'s> CheckedRawFileView<'s> {
        pub(crate) fn entry(&self) -> &'s crate::storage::FileEntry {
            &self.batch.entry().files[self.index]
        }

        pub fn batch(&self) -> CheckedRawBatchView<'s> {
            CheckedRawBatchView { batch: self.batch }
        }
        pub fn file_name(&self) -> &'s str {
            &self.batch.entry().files[self.index].file_name
        }
        pub fn kind(&self) -> ResponseKind {
            self.batch.entry().files[self.index].kind
        }
        pub fn content_hash(&self) -> &'s ContentHash {
            BatchAccess::files(self.batch)[self.index].hash()
        }
        pub fn size_bytes(&self) -> u64 {
            self.batch.entry().files[self.index].size_bytes
        }
        pub fn bytes(&self) -> &'s [u8] {
            BatchAccess::files(self.batch)[self.index].bytes()
        }
    }
    pub struct CheckedRawPartialBatchView<'s> {
        state: &'s CaptureBatchState<CheckedHandle, CheckedLeaf>,
    }
    impl<'s> CheckedRawPartialBatchView<'s> {
        pub fn is_complete(&self) -> bool {
            false
        }
        pub fn entry(&self) -> Option<&'s ManifestEntry> {
            self.state.pending.as_ref().map(|pending| &pending.entry)
        }
        pub fn batch_json_bytes(&self) -> Option<&'s [u8]> {
            self.state
                .batch_json
                .as_ref()
                .map(|leaf| leaf.bytes().as_slice())
                .or(self.state.metadata_bytes.as_deref())
        }
        pub fn returned_file_count(&self) -> usize {
            self.state.files.len()
        }
        pub fn returned_file_bytes(&self, index: usize) -> Option<&'s [u8]> {
            self.state
                .files
                .get(index)
                .map(|leaf| leaf.bytes().as_slice())
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "Keep whole diagnostics without added failure-path allocations."
    )]
    pub(super) fn snapshot_checked<'i, 'a>(
        data_root: &Path,
        invocation: &'i RawInvocation<'a>,
        expected: &Attempt,
        captured: &Attempt,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CheckedRawSnapshot<'i, 'a>, CheckedRawSnapshotFailure> {
        snapshot_checked_with_coordination(
            data_root, invocation, expected, captured, scope, limits, None,
        )
    }

    #[expect(
        clippy::result_large_err,
        reason = "Preserve actual checked scan failure custody."
    )]
    pub(super) fn snapshot_checked_with_coordination<'i, 'a>(
        data_root: &Path,
        invocation: &'i RawInvocation<'a>,
        expected: &Attempt,
        captured: &Attempt,
        scope: ApprovedRawScope,
        limits: ScanLimits,
        coordination_root: Option<&Path>,
    ) -> Result<CheckedRawSnapshot<'i, 'a>, CheckedRawSnapshotFailure> {
        let control = CheckedControl::new(invocation, expected, captured);
        let mut unwind = SnapshotUnwind {
            attempt: &control.own,
            armed: true,
        };
        let mode = Checked { control: &control };
        let mut state = SnapshotBuildState::default();
        let result = OriginalOutcome(
            (|| {
                let original = mode.before().map_err(|error| match error {
                    CheckedScanError::First(first) => first,
                    CheckedScanError::Policy(_) => unreachable!("scope-only preflight"),
                });
                let execution = invocation.compare_native_context_scopes(expected);
                crate::raw_custody::join_native_checks(original, execution, |original, failure| {
                    Box::new(SnapshotFirst::Execution { original, failure })
                })
                .map_err(CheckedScanError::First)?;
                match crate::close_gate::require_native(invocation, expected) {
                    Ok(()) => {}
                    Err(crate::close_gate::NativeAdmissionFailure::Unbound) => {
                        return Err(CheckedScanError::First(Box::new(SnapshotFirst::Binding(
                            invocation.refused(Gap::Unbound),
                        ))));
                    }
                    Err(crate::close_gate::NativeAdmissionFailure::Execution(failure)) => {
                        return Err(CheckedScanError::First(Box::new(
                            SnapshotFirst::Execution {
                                original: None,
                                failure,
                            },
                        )));
                    }
                }
                if let Some(coordination) = coordination_root {
                    if coordination.starts_with(data_root) || data_root.starts_with(coordination) {
                        return Err(DailyEvidenceError::UnsafePath.into());
                    }
                    for path in [data_root, coordination] {
                        let bytes = path.as_os_str().as_bytes();
                        if !bytes.starts_with(b"/")
                            || bytes[1..]
                                .split(|byte| *byte == b'/')
                                .any(|part| part.is_empty() || part == b"." || part == b"..")
                        {
                            return Err(DailyEvidenceError::UnsafePath.into());
                        }
                    }
                }
                build_snapshot_with_coordination(
                    &mode,
                    data_root,
                    scope,
                    limits,
                    &mut state,
                    coordination_root,
                )
            })()
            .map_err(|error| mode.normalize(error)),
        );
        match result.0 {
            Ok(data) => {
                // The final comparison/settlement operates on unit; the actual data stays outside T.
                let publication = mode.before().and_then(|()| {
                    control.local().map_err(|failure| {
                        CheckedScanError::First(Box::new(SnapshotFirst::Local(failure)))
                    })
                });
                match publication {
                    Ok(()) => {
                        unwind.armed = false;
                        drop(unwind);
                        Ok(CheckedRawSnapshot {
                            control,
                            data,
                            armed: true,
                        })
                    }
                    Err(error) => {
                        let first = checked_first(error);
                        let mut data = data;
                        let completion = complete_data(&control, &mut data);
                        let returned_leaf = control.returned_leaf.borrow_mut().take();
                        unwind.armed = false;
                        Err(CheckedRawSnapshotFailure {
                            first,
                            data,
                            returned_leaf,
                            completion,
                        })
                    }
                }
            }
            Err(error) => {
                let first = checked_first(error);
                let mut data = CheckedData::Partial(state);
                let completion = complete_data(&control, &mut data);
                let returned_leaf = control.returned_leaf.borrow_mut().take();
                unwind.armed = false;
                Err(CheckedRawSnapshotFailure {
                    first,
                    data,
                    returned_leaf,
                    completion,
                })
            }
        }
    }

    fn checked_first(error: CheckedScanError) -> SnapshotFirst {
        match error {
            CheckedScanError::First(first) => *first,
            CheckedScanError::Policy(_) => unreachable!("normalized before resource retirement"),
        }
    }

    fn complete_data(
        control: &CheckedControl<'_, '_>,
        data: &mut CheckedData,
    ) -> CheckedRawSnapshotCompletion {
        let before_close = control.compare(&control.expected, &control.captured);
        data.retire();
        let after_close = control.compare(&control.expected, &control.captured);
        let local = control.local();
        CheckedRawSnapshotCompletion {
            before_close,
            after_close,
            local,
            data: None,
            returned_leaf: None,
        }
    }

    /// Owns exactly one native file from open through its final destruction.
    /// Moves transfer this owner without opening or closing a descriptor.
    pub(super) struct OwnedSourceFile {
        file: Option<File>,
        checked: Option<Owner<File>>,
        #[cfg(test)]
        observation: Option<OpenObservation>,
    }

    impl OwnedSourceFile {
        fn new(file: File, #[cfg(test)] role: DescriptorRole) -> Self {
            #[cfg(test)]
            let observation = OpenObservation::register(&file, role);
            Self {
                file: Some(file),
                checked: None,
                #[cfg(test)]
                observation,
            }
        }

        fn file(&self) -> &File {
            self.file
                .as_ref()
                .or_else(|| self.checked.as_ref().map(Owner::get))
                .expect("source file is owned until its destructor")
        }

        fn retire(&mut self) {
            if let Some(owner) = self.checked.take() {
                let _ = owner.finish();
                #[cfg(test)]
                if let Some(observation) = self.observation.take() {
                    observation.record_closed();
                }
            }
        }

        #[cfg(test)]
        fn observe_identity(&self, stat: &rustix::fs::Stat) {
            if let Some(observation) = &self.observation {
                observation.record_identity(stat.st_dev, stat.st_ino);
            }
        }
    }

    impl Drop for OwnedSourceFile {
        fn drop(&mut self) {
            if let Some(owner) = &self.checked {
                owner.attempt().gap(Gap::Abandoned);
            }
            self.retire();
            // The production close happens first. Test state follows that same
            // destruction, even after the thread-local collection scope ends.
            drop(self.file.take());
            #[cfg(test)]
            if let Some(observation) = &self.observation {
                observation.record_closed();
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum NodeType {
        Directory,
        RegularFile,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct Fingerprint {
        device: u64,
        inode: u64,
        mode: u32,
        uid: u32,
        gid: u32,
        links: Option<u64>,
        size: Option<i64>,
        mtime: Option<(i64, u64)>,
        ctime: Option<(i64, u64)>,
        node_type: NodeType,
    }

    impl Fingerprint {
        fn from_stat(
            stat: &rustix::fs::Stat,
            expected: NodeType,
            raw_uid: Option<u32>,
        ) -> Result<Self, DailyEvidenceError> {
            let file_type = FileType::from_raw_mode(stat.st_mode);
            let expected_type = match expected {
                NodeType::Directory => FileType::Directory,
                NodeType::RegularFile => FileType::RegularFile,
            };
            if file_type == FileType::Symlink {
                return Err(DailyEvidenceError::UnsafePath);
            }
            if file_type != expected_type {
                return Err(match expected {
                    NodeType::Directory => DailyEvidenceError::UnsafePath,
                    NodeType::RegularFile => DailyEvidenceError::NonRegularFile,
                });
            }
            validate_raw_stat_owner_mode(stat.st_mode, stat.st_uid, raw_uid)?;
            match expected {
                NodeType::Directory => Ok(Self {
                    device: stat.st_dev,
                    inode: stat.st_ino,
                    mode: stat.st_mode,
                    uid: stat.st_uid,
                    gid: stat.st_gid,
                    links: None,
                    size: None,
                    mtime: None,
                    ctime: None,
                    node_type: expected,
                }),
                NodeType::RegularFile => {
                    let links = checked_u64(stat.st_nlink)?;
                    validate_single_link_count(links)?;
                    if stat.st_size < 0 {
                        return Err(DailyEvidenceError::SizeMismatch);
                    }
                    Ok(Self {
                        device: stat.st_dev,
                        inode: stat.st_ino,
                        mode: stat.st_mode,
                        uid: stat.st_uid,
                        gid: stat.st_gid,
                        links: Some(links),
                        size: Some(stat.st_size),
                        mtime: Some((stat.st_mtime, checked_u64(stat.st_mtime_nsec)?)),
                        ctime: Some((stat.st_ctime, checked_u64(stat.st_ctime_nsec)?)),
                        node_type: expected,
                    })
                }
            }
        }
    }

    fn checked_u64<T>(value: T) -> Result<u64, DailyEvidenceError>
    where
        u64: TryFrom<T>,
    {
        u64::try_from(value).map_err(|_| DailyEvidenceError::ArithmeticOverflow)
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct LockFingerprint {
        device: u64,
        inode: u64,
        mode: u32,
        uid: u32,
        gid: u32,
        links: u64,
    }

    impl LockFingerprint {
        fn from_stat(stat: &rustix::fs::Stat, raw_uid: u32) -> Result<Self, DailyEvidenceError> {
            let file_type = FileType::from_raw_mode(stat.st_mode);
            if file_type == FileType::Symlink {
                return Err(DailyEvidenceError::UnsafePath);
            }
            if file_type != FileType::RegularFile {
                return Err(DailyEvidenceError::NonRegularFile);
            }
            validate_raw_stat_owner_mode(stat.st_mode, stat.st_uid, Some(raw_uid))?;
            let links = checked_u64(stat.st_nlink)?;
            validate_single_link_count(links)?;
            Ok(Self {
                device: stat.st_dev,
                inode: stat.st_ino,
                mode: stat.st_mode,
                uid: stat.st_uid,
                gid: stat.st_gid,
                links,
            })
        }
    }

    fn validate_raw_stat_owner_mode(
        mode: u32,
        uid: u32,
        raw_uid: Option<u32>,
    ) -> Result<(), DailyEvidenceError> {
        if let Some(owner) = raw_uid {
            if uid != owner {
                return Err(DailyEvidenceError::UntrustedOwner);
            }
            if mode & 0o022 != 0 {
                return Err(DailyEvidenceError::WritableNode);
            }
        }
        Ok(())
    }

    fn validate_single_link_count(link_count: u64) -> Result<(), DailyEvidenceError> {
        if link_count == 1 {
            Ok(())
        } else {
            Err(DailyEvidenceError::HardLinkedFile)
        }
    }

    #[derive(Clone, Copy)]
    pub(super) struct NameRange {
        start: usize,
        length: usize,
    }

    #[derive(Default)]
    struct NamePool {
        bytes: Vec<u8>,
    }

    impl NamePool {
        fn append(&mut self, name: &[u8]) -> Result<NameRange, DailyEvidenceError> {
            let start = self.bytes.len();
            let end = start
                .checked_add(name.len())
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            #[cfg(test)]
            observe_resource(ObservationEvent::NamePoolReserve(name.len()));
            self.bytes
                .try_reserve_exact(name.len())
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            self.bytes.extend_from_slice(name);
            #[cfg(test)]
            observe_resource(ObservationEvent::NameCopied(name.len()));
            Ok(NameRange {
                start,
                length: end - start,
            })
        }

        fn get(&self, range: NameRange) -> Result<&OsStr, DailyEvidenceError> {
            let end = range
                .start
                .checked_add(range.length)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            let bytes = self
                .bytes
                .get(range.start..end)
                .ok_or(DailyEvidenceError::SourceChanged)?;
            Ok(OsStr::from_bytes(bytes))
        }
    }

    struct DirectoryNode<H: NativeFileHandle = OwnedSourceFile> {
        file: H,
        parent: Option<usize>,
        name: Option<NameRange>,
        fingerprint: Fingerprint,
        raw_protected: bool,
    }

    pub(super) fn validate_limit_layout(
        values: &ScanLimitValues,
    ) -> Result<(), DailyEvidenceError> {
        super::checked_capacity(
            values.max_open_descriptors,
            std::mem::size_of::<DirectoryNode>(),
        )?;
        Ok(())
    }

    struct HeldFile<H: NativeFileHandle = OwnedSourceFile> {
        file: H,
        parent: usize,
        name: NameRange,
        fingerprint: Fingerprint,
    }

    struct HeldLock<H: NativeFileHandle = OwnedSourceFile> {
        file: H,
        parent: usize,
        name: NameRange,
        fingerprint: LockFingerprint,
    }

    struct ConstructionResources<H: NativeFileHandle = OwnedSourceFile> {
        manifest_file: Option<HeldFile<H>>,
        directories: Vec<DirectoryNode<H>>,
        names: NamePool,
        descriptors: DescriptorBudget,
        transferred: bool,
        lock_file: Option<HeldLock<H>>,
    }

    impl<H: NativeFileHandle> ConstructionResources<H> {
        fn into_resource_guard(mut self) -> Result<ResourceGuard<H>, DailyEvidenceError> {
            let manifest_file = self.manifest_file.take().ok_or(DailyEvidenceError::Io)?;
            let lock_file = self.lock_file.take().ok_or(DailyEvidenceError::Io)?;
            let guard = ResourceGuard {
                _manifest_file: manifest_file,
                _directories: std::mem::take(&mut self.directories),
                _names: std::mem::take(&mut self.names),
                _lock_file: lock_file,
            };
            self.transferred = true;
            Ok(guard)
        }

        fn retire(&mut self) {
            retire_resources(self, || {}, || {});
        }
    }

    impl<H: NativeFileHandle> Drop for ConstructionResources<H> {
        fn drop(&mut self) {
            self.retire();
        }
    }

    struct ScanContext<H: NativeFileHandle = OwnedSourceFile> {
        scope: ApprovedRawScope,
        limits: ScanLimits,
        archive_read: bool,
        resources: ConstructionResources<H>,
    }

    /// Field order is intentional: metadata, directories, then the one lock FD.
    pub(super) struct ResourceGuard<H: NativeFileHandle = OwnedSourceFile> {
        _manifest_file: HeldFile<H>,
        _directories: Vec<DirectoryNode<H>>,
        _names: NamePool,
        _lock_file: HeldLock<H>,
    }

    struct DescriptorBudget {
        limit: usize,
        current: usize,
        peak: usize,
    }

    impl DescriptorBudget {
        fn reserve_before_open(&mut self) -> Result<(), DailyEvidenceError> {
            let next = self
                .current
                .checked_add(1)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            if next > self.limit {
                return Err(DailyEvidenceError::SourceScanLimit);
            }
            self.current = next;
            self.peak = self.peak.max(next);
            #[cfg(test)]
            observe_resource(ObservationEvent::DescriptorReserved);
            Ok(())
        }

        fn open<T, E: From<DailyEvidenceError>>(
            &mut self,
            operation: impl FnOnce() -> Result<T, E>,
        ) -> Result<T, E> {
            self.reserve_before_open()?;
            #[cfg(test)]
            observe_resource(ObservationEvent::DescriptorOpenAttempt);
            match operation() {
                Ok(value) => Ok(value),
                Err(error) => {
                    self.current = self
                        .current
                        .checked_sub(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                    Err(error)
                }
            }
        }
    }

    struct RetainedBytes {
        total: usize,
        limit: usize,
    }

    impl RetainedBytes {
        fn reserve(&mut self, amount: usize) -> Result<(), DailyEvidenceError> {
            let next = self
                .total
                .checked_add(amount)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            if next > self.limit {
                return Err(DailyEvidenceError::SourceScanLimit);
            }
            self.total = next;
            #[cfg(test)]
            observe_resource(ObservationEvent::EvidenceBytesReserved {
                amount,
                total: self.total,
            });
            Ok(())
        }
    }

    fn errno_error(error: Errno, missing: DailyEvidenceError) -> DailyEvidenceError {
        if error == Errno::NOENT {
            missing
        } else if error == Errno::LOOP || error == Errno::NOTDIR {
            DailyEvidenceError::UnsafePath
        } else if error == Errno::ACCESS || error == Errno::PERM {
            DailyEvidenceError::PermissionDenied
        } else {
            DailyEvidenceError::Io
        }
    }

    fn errno_io(error: Errno) -> DailyEvidenceError {
        if error == Errno::ACCESS || error == Errno::PERM {
            DailyEvidenceError::PermissionDenied
        } else {
            DailyEvidenceError::Io
        }
    }

    fn io_error(error: std::io::Error) -> DailyEvidenceError {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            DailyEvidenceError::PermissionDenied
        } else {
            DailyEvidenceError::Io
        }
    }

    fn stat_at<M: SnapshotMode>(
        mode: &M,
        parent: &File,
        name: &OsStr,
        missing: DailyEvidenceError,
    ) -> Result<rustix::fs::Stat, M::Error> {
        effect(mode, || {
            statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| errno_error(error, missing))
        })
    }

    fn verify_path_fingerprint(
        stat: &rustix::fs::Stat,
        expected: Fingerprint,
        owner: Option<u32>,
    ) -> Result<(), DailyEvidenceError> {
        let actual = Fingerprint::from_stat(stat, expected.node_type, owner)?;
        if actual != expected {
            return Err(DailyEvidenceError::SourceChanged);
        }
        Ok(())
    }

    fn open_child_directory<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        parent_index: usize,
        name: &[u8],
        raw_owner: Option<u32>,
        missing: DailyEvidenceError,
    ) -> Result<usize, M::Error> {
        let mut pending: Option<M::Handle> = None;
        let result = (|| {
            mode.before()?;
            let resources = &mut scan.resources;
            let range = resources.names.append(name)?;
            let name_os = resources.names.get(range)?;
            let pre_stat = stat_at(
                mode,
                resources.directories[parent_index].file.file(),
                name_os,
                missing,
            )?;
            let pre_fingerprint =
                Fingerprint::from_stat(&pre_stat, NodeType::Directory, raw_owner)?;
            pending = Some(resources.descriptors.open::<_, M::Error>(|| {
                mode.before()?;
                let descriptor = openat(
                    resources.directories[parent_index].file.file(),
                    name_os,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| M::Error::from(errno_error(error, missing)))?;
                Ok(mode.adopt(
                    File::from(descriptor),
                    #[cfg(test)]
                    DescriptorRole::Directory,
                ))
            })?);
            let file = pending.as_ref().expect("actual opened pending owner");
            let opened_stat = effect(mode, || fstat(file.file()).map_err(errno_io))?;
            if scan.archive_read {
                no_archive_acl(mode, file.file())?;
            }
            #[cfg(test)]
            file.observe_identity(&opened_stat);
            let opened = Fingerprint::from_stat(&opened_stat, NodeType::Directory, raw_owner)?;
            if opened != pre_fingerprint {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            let named = stat_at(
                mode,
                resources.directories[parent_index].file.file(),
                resources.names.get(range)?,
                DailyEvidenceError::SourceChanged,
            )?;
            verify_path_fingerprint(&named, pre_fingerprint, raw_owner)?;
            let index = resources.directories.len();
            resources
                .directories
                .try_reserve(1)
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            resources.directories.push(DirectoryNode {
                file: pending.take().expect("validated pending owner"),
                parent: Some(parent_index),
                name: Some(range),
                fingerprint: pre_fingerprint,
                raw_protected: raw_owner.is_some(),
            });
            Ok(index)
        })();
        preserve_before_retire(mode, result, || {
            if let Some(file) = &mut pending {
                file.retire();
            }
        })
    }

    fn root_components(
        root: &Path,
        limits: &ScanLimits,
    ) -> Result<Vec<Vec<u8>>, DailyEvidenceError> {
        if !root.is_absolute() {
            return Err(DailyEvidenceError::UnsafePath);
        }
        let raw = root.as_os_str().as_bytes();
        if raw.len() > limits.values.max_data_root_bytes || raw.contains(&0) {
            return Err(if raw.len() > limits.values.max_data_root_bytes {
                DailyEvidenceError::SourceScanLimit
            } else {
                DailyEvidenceError::UnsafePath
            });
        }
        let mut component_count = 0usize;
        for component in root.components() {
            match component {
                std::path::Component::RootDir | std::path::Component::CurDir => {}
                std::path::Component::Normal(_) => {
                    component_count = component_count
                        .checked_add(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                }
                std::path::Component::ParentDir | std::path::Component::Prefix(_) => {
                    return Err(DailyEvidenceError::UnsafePath);
                }
            }
        }
        if component_count > limits.values.max_data_root_components {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        let mut components = Vec::new();
        #[cfg(test)]
        observe_resource(ObservationEvent::RootComponentsReserve(component_count));
        components
            .try_reserve_exact(component_count)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        for component in root.components() {
            match component {
                std::path::Component::RootDir | std::path::Component::CurDir => {}
                std::path::Component::Normal(name) => {
                    let mut component_bytes = Vec::new();
                    #[cfg(test)]
                    observe_resource(ObservationEvent::RootComponentReserve(
                        name.as_bytes().len(),
                    ));
                    component_bytes
                        .try_reserve_exact(name.as_bytes().len())
                        .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                    component_bytes.extend_from_slice(name.as_bytes());
                    #[cfg(test)]
                    observe_resource(ObservationEvent::RootComponentCopied);
                    components.push(component_bytes);
                }
                std::path::Component::ParentDir | std::path::Component::Prefix(_) => {
                    return Err(DailyEvidenceError::UnsafePath);
                }
            }
        }
        if components.len() > limits.values.max_data_root_components {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        Ok(components)
    }

    fn preflight_descriptor_count(
        components: usize,
        minimum: usize,
        limit: usize,
    ) -> Result<(), DailyEvidenceError> {
        let required = components
            .checked_add(minimum)
            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        if required > limit {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        Ok(())
    }

    fn build_snapshot<M: SnapshotMode>(
        mode: &M,
        data_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
        state: &mut SnapshotBuildState<M::Handle, M::Leaf, M::Batch>,
    ) -> Result<M::Output, M::Error> {
        build_snapshot_with_coordination(mode, data_root, scope, limits, state, None)
    }

    fn build_snapshot_with_coordination<M: SnapshotMode>(
        mode: &M,
        data_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
        state: &mut SnapshotBuildState<M::Handle, M::Leaf, M::Batch>,
        coordination_root: Option<&Path>,
    ) -> Result<M::Output, M::Error> {
        mode.before()?;
        mode.capacity(&limits)?;
        let components = root_components(data_root, &limits)?;
        let coordination = coordination_root
            .map(|path| root_components(path, &limits))
            .transpose()?;
        let descriptor_limit = limits.values.max_open_descriptors;
        let manifest_limit = limits.values.max_manifest_bytes;
        // `/`, the data-root ancestors, `raw`, three manifest directories and
        // both existing controls are the minimum opened descriptor graph.
        preflight_descriptor_count(components.len(), 7, limits.values.max_open_descriptors)?;

        let mut directories = Vec::new();
        let initial_directory_capacity = components
            .len()
            .checked_add(5)
            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        directories
            .try_reserve_exact(initial_directory_capacity)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        state.scan = Some(ScanContext {
            scope,
            limits,
            archive_read: coordination.is_some(),
            resources: ConstructionResources {
                manifest_file: None,
                directories,
                names: NamePool::default(),
                descriptors: DescriptorBudget {
                    limit: descriptor_limit,
                    current: 0,
                    peak: 0,
                },
                transferred: false,
                lock_file: None,
            },
        });
        let scan = state.scan.as_mut().expect("created construction context");
        let result: Result<(), M::Error> = (|| {
            let root_stat = effect(mode, || {
                statat(rustix::fs::CWD, "/", AtFlags::empty())
                    .map_err(|error| errno_error(error, DailyEvidenceError::MissingControl))
            })?;
            let root_fingerprint = Fingerprint::from_stat(&root_stat, NodeType::Directory, None)?;
            let mut pending_root: Option<M::Handle> = None;
            let root_result = (|| {
                pending_root = Some(scan.resources.descriptors.open(|| {
                    mode.before()?;
                    open(
                        "/",
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map(|descriptor| {
                        mode.adopt(
                            File::from(descriptor),
                            #[cfg(test)]
                            DescriptorRole::Root,
                        )
                    })
                    .map_err(|error| {
                        M::Error::from(errno_error(error, DailyEvidenceError::MissingControl))
                    })
                })?);
                let root_file = pending_root.as_ref().expect("actual opened root");
                let opened_root_stat = effect(mode, || fstat(root_file.file()).map_err(errno_io))?;
                if scan.archive_read {
                    no_archive_acl(mode, root_file.file())?;
                }
                #[cfg(test)]
                root_file.observe_identity(&opened_root_stat);
                let opened_root =
                    Fingerprint::from_stat(&opened_root_stat, NodeType::Directory, None)?;
                if opened_root != root_fingerprint {
                    return Err(DailyEvidenceError::SourceChanged.into());
                }
                scan.resources.directories.push(DirectoryNode {
                    file: pending_root.take().expect("validated root owner"),
                    parent: None,
                    name: None,
                    fingerprint: root_fingerprint,
                    raw_protected: false,
                });
                Ok(())
            })();
            preserve_before_retire(mode, root_result, || {
                if let Some(root) = &mut pending_root {
                    root.retire();
                }
            })?;
            let root_index = 0;
            let raw_uid = scan.scope.trusted_raw_owner_uid;
            let mut cursor = root_index;
            for component in &components {
                cursor = open_child_directory(
                    mode,
                    scan,
                    cursor,
                    component,
                    None,
                    DailyEvidenceError::MissingControl,
                )?;
            }
            let raw_index = open_child_directory(
                mode,
                scan,
                cursor,
                b"raw",
                Some(raw_uid),
                DailyEvidenceError::MissingControl,
            )?;
            let manifest_root = open_child_directory(
                mode,
                scan,
                raw_index,
                b"manifests",
                Some(raw_uid),
                DailyEvidenceError::MissingControl,
            )?;
            let manifest_provider_name = format!("provider={}", scan.scope.provider());
            let manifest_provider = open_child_directory(
                mode,
                scan,
                manifest_root,
                manifest_provider_name.as_bytes(),
                Some(raw_uid),
                DailyEvidenceError::MissingControl,
            )?;
            let manifest_market_name = format!("market={}", scan.scope.market());
            let manifest_market = open_child_directory(
                mode,
                scan,
                manifest_provider,
                manifest_market_name.as_bytes(),
                Some(raw_uid),
                DailyEvidenceError::MissingControl,
            )?;

            let (lock_parent, lock_name) = match coordination.as_ref() {
                None => (
                    manifest_market,
                    scan.resources.names.append(b"commit.lock")?,
                ),
                Some(parts) => {
                    let mut parent = root_index;
                    for part in parts {
                        parent = open_child_directory(
                            mode,
                            scan,
                            parent,
                            part,
                            None,
                            DailyEvidenceError::MissingControl,
                        )?;
                        let file = scan.resources.directories[parent].file.file();
                        let info = effect(mode, || fstat(file).map_err(errno_io))?;
                        if info.st_uid != 0 || info.st_gid != 0 || info.st_mode & 0o7022 != 0 {
                            return Err(DailyEvidenceError::UntrustedOwner.into());
                        }
                        no_archive_acl(mode, file)?;
                    }
                    let file = scan.resources.directories[parent].file.file();
                    if effect(mode, || fstat(file).map_err(errno_io))?.st_mode & 0o7777 != 0o700 {
                        return Err(DailyEvidenceError::WritableNode.into());
                    }
                    let name = format!("{}.lock", scan.scope.provider());
                    (parent, scan.resources.names.append(name.as_bytes())?)
                }
            };
            open_lock_file(mode, scan, lock_parent, lock_name)?;
            {
                let lock_file = scan
                    .resources
                    .lock_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?;
                effect(mode, || {
                    shared_lock_result(FileExt::try_lock_shared(lock_file.file.file()))
                })?;
                if coordination.is_some() {
                    let info = effect(mode, || fstat(lock_file.file.file()).map_err(errno_io))?;
                    if info.st_mode & 0o7777 != 0o600 || info.st_gid != 0 || info.st_size != 0 {
                        return Err(DailyEvidenceError::WritableNode.into());
                    }
                    no_archive_acl(mode, lock_file.file.file())?;
                }
            }
            effect(mode, || hit_phase(CapturePhase::AfterSharedLock))?;
            revalidate_directories(
                mode,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;
            revalidate_held_lock(
                mode,
                scan.resources
                    .lock_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;

            let manifest_name = scan.resources.names.append(b"manifest.jsonl")?;
            let manifest_file = open_leaf_control(
                mode,
                scan,
                manifest_market,
                manifest_name,
                DailyEvidenceError::MissingControl,
                Some(raw_uid),
                Some(manifest_limit),
            )?;
            scan.resources.manifest_file = Some(manifest_file);
            let manifest_length = checked_stat_size(
                scan.resources
                    .manifest_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?
                    .fingerprint,
            )?;
            let mut retained = RetainedBytes {
                total: 0,
                limit: scan.limits.values.max_retained_bytes,
            };
            retained.reserve(manifest_length)?;
            capture_returned(&mut state.manifest_captured, || {
                let manifest_file = scan
                    .resources
                    .manifest_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?;
                if scan.archive_read {
                    no_archive_acl(mode, manifest_file.file.file())?;
                }
                let result = read_exact_captured_in(
                    mode,
                    manifest_file.file.file(),
                    manifest_length,
                    manifest_limit,
                )?;
                if scan.archive_read {
                    no_archive_acl(mode, manifest_file.file.file())?;
                }
                Ok::<_, M::Error>(result)
            })?;
            revalidate_held_file(
                mode,
                scan.resources
                    .manifest_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?,
                &scan.resources.directories,
                &scan.resources.names,
                Some(scan.scope.trusted_raw_owner_uid),
            )?;
            effect(mode, || hit_phase(CapturePhase::AfterManifestCapture))?;
            state.manifest_hash = Some(ContentHash::from_bytes(
                state
                    .manifest_captured
                    .as_ref()
                    .expect("returned manifest bytes"),
            ));
            state.manifest_bytes = Some(Arc::new(
                state
                    .manifest_captured
                    .take()
                    .expect("returned manifest bytes"),
            ));
            let manifest_bytes = state
                .manifest_bytes
                .as_ref()
                .expect("captured manifest Arc");
            capture_returned(&mut state.parsed, || {
                parse_manifest(&scan.scope, &scan.limits, manifest_bytes.as_slice())
            })?;
            let parsed = state.parsed.as_ref().expect("actual returned parse");
            let pending = &parsed.pending;
            let records = parsed.records;
            let total_files = parsed.total_files;
            validate_recorded_file_sizes(pending, &scan.limits)?;

            let distinct_dates = distinct_date_count(pending)?;
            let expected_descriptors = if pending.is_empty() {
                components
                    .len()
                    .checked_add(7)
                    .ok_or(DailyEvidenceError::ArithmeticOverflow)?
            } else {
                components
                    .len()
                    .checked_add(9)
                    .and_then(|value| value.checked_add(distinct_dates))
                    .and_then(|value| value.checked_add(pending.len().checked_mul(2)?))
                    .and_then(|value| value.checked_add(total_files))
                    .ok_or(DailyEvidenceError::ArithmeticOverflow)?
            };
            if expected_descriptors > scan.limits.values.max_open_descriptors {
                return Err(DailyEvidenceError::SourceScanLimit.into());
            }
            let directory_capacity = expected_descriptors
                .checked_sub(scan.resources.directories.len())
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            scan.resources
                .directories
                .try_reserve_exact(directory_capacity)
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;

            state
                .batches
                .try_reserve_exact(pending.len())
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            let batches = &mut state.batches;
            state.remaining = Some(
                std::mem::take(
                    &mut state
                        .parsed
                        .as_mut()
                        .expect("actual returned parse")
                        .pending,
                )
                .into_iter(),
            );
            while let Some(pending_batch) =
                state.remaining.as_mut().expect("pending iterator").next()
            {
                state.current = Some(CaptureBatchState::new(pending_batch));
                let batch = capture_batch(
                    mode,
                    scan,
                    state.current.as_mut().expect("actual current batch"),
                    batches,
                    &mut retained,
                    raw_index,
                    manifest_bytes,
                )?;
                append_completed(batches, batch);
                state.current.take();
            }
            effect(mode, || hit_phase(CapturePhase::BeforeFinalBytes))?;
            final_byte_checks(
                mode,
                scan.resources
                    .manifest_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?,
                manifest_bytes,
                batches,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;
            effect(mode, || hit_phase(CapturePhase::BeforeFinalNames))?;
            for batch in batches.iter() {
                let batch_directory = batch.metadata().parent_index();
                let mut final_seen = Vec::new();
                final_seen
                    .try_reserve_exact(batch.entry().files.len())
                    .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                final_seen.resize(batch.entry().files.len(), false);
                verify_batch_membership(
                    mode,
                    &scan.resources.directories[batch_directory],
                    batch.entry(),
                    &mut final_seen,
                    DailyEvidenceError::SourceChanged,
                )?;
            }
            revalidate_directories(
                mode,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;
            revalidate_held_file(
                mode,
                scan.resources
                    .manifest_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?,
                &scan.resources.directories,
                &scan.resources.names,
                Some(scan.scope.trusted_raw_owner_uid),
            )?;
            revalidate_held_lock(
                mode,
                scan.resources
                    .lock_file
                    .as_ref()
                    .ok_or(DailyEvidenceError::Io)?,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;
            revalidate_all_batch_files(
                mode,
                batches,
                &scan.resources.directories,
                &scan.resources.names,
                scan.scope.trusted_raw_owner_uid,
            )?;

            state.stats = Some(SnapshotStats {
                manifest_bytes: manifest_bytes.len(),
                manifest_records: records,
                batches: batches.len(),
                files: total_files,
                retained_evidence_bytes: retained.total,
                open_descriptors: scan.resources.descriptors.current,
                peak_open_descriptors: scan.resources.descriptors.peak,
            });
            Ok(())
        })();
        result.map_err(|error| mode.normalize(error))?;

        let scan = state.scan.take().expect("complete scan");
        let resources = scan.resources.into_resource_guard()?;
        Ok(M::output(OutputParts {
            data_root: data_root.to_path_buf(),
            scope: scan.scope,
            limits: scan.limits,
            stats: state.stats.take().expect("computed snapshot stats"),
            manifest_bytes: state.manifest_bytes.take().expect("complete manifest"),
            manifest_sha256: state.manifest_hash.take().expect("computed manifest hash"),
            batches: std::mem::take(&mut state.batches),
            batch_index: std::mem::take(
                &mut state.parsed.as_mut().expect("actual parse").batch_index,
            ),
            resources,
        }))
    }

    fn distinct_date_count(pending: &[PendingBatch]) -> Result<usize, DailyEvidenceError> {
        let mut dates = Vec::new();
        for batch in pending {
            if dates.contains(&batch.entry.date) {
                continue;
            }
            dates
                .try_reserve(1)
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            dates.push(batch.entry.date);
        }
        Ok(dates.len())
    }

    fn checked_stat_size_value(size: i64) -> Result<usize, DailyEvidenceError> {
        if size < 0 {
            return Err(DailyEvidenceError::SizeMismatch);
        }
        usize::try_from(size).map_err(|_| DailyEvidenceError::SizeMismatch)
    }

    fn checked_stat_size(fingerprint: Fingerprint) -> Result<usize, DailyEvidenceError> {
        checked_stat_size_value(fingerprint.size.ok_or(DailyEvidenceError::SizeMismatch)?)
    }

    fn validate_recorded_file_sizes(
        pending: &[PendingBatch],
        limits: &ScanLimits,
    ) -> Result<(), DailyEvidenceError> {
        let maximum = checked_u64(limits.values.max_source_file_bytes)?;
        for batch in pending {
            for file in &batch.entry.files {
                if file.size_bytes == 0 {
                    return Err(DailyEvidenceError::SizeMismatch);
                }
                if file.size_bytes > maximum {
                    return Err(DailyEvidenceError::SourceScanLimit);
                }
            }
        }
        Ok(())
    }

    fn open_leaf_control<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        parent: usize,
        name: NameRange,
        missing: DailyEvidenceError,
        raw_owner: Option<u32>,
        byte_cap: Option<usize>,
    ) -> Result<HeldFile<M::Handle>, M::Error> {
        let mut pending: Option<M::Handle> = None;
        let result = (|| {
            mode.before()?;
            let resources = &mut scan.resources;
            let name_os = resources.names.get(name)?;
            let parent_file = resources
                .directories
                .get(parent)
                .ok_or(DailyEvidenceError::Io)?
                .file
                .file();
            let pre_stat = stat_at(mode, parent_file, name_os, missing)?;
            let fingerprint = Fingerprint::from_stat(&pre_stat, NodeType::RegularFile, raw_owner)?;
            if let Some(maximum) = byte_cap {
                let size = checked_stat_size(fingerprint)?;
                if size > maximum {
                    return Err(DailyEvidenceError::SourceScanLimit.into());
                }
            }
            let name_os = resources.names.get(name)?;
            pending = Some(resources.descriptors.open::<_, M::Error>(|| {
                mode.before()?;
                let descriptor = openat(
                    parent_file,
                    name_os,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|error| M::Error::from(errno_error(error, missing)))?;
                Ok(mode.adopt(
                    File::from(descriptor),
                    #[cfg(test)]
                    if name_os.as_bytes() == b"manifest.jsonl" {
                        DescriptorRole::Manifest
                    } else {
                        DescriptorRole::Metadata
                    },
                ))
            })?);
            let file = pending.as_ref().expect("actual opened pending owner");
            let opened_stat = effect(mode, || fstat(file.file()).map_err(errno_io))?;
            #[cfg(test)]
            file.observe_identity(&opened_stat);
            let opened = Fingerprint::from_stat(&opened_stat, NodeType::RegularFile, raw_owner)?;
            if scan.archive_read {
                no_archive_acl(mode, file.file())?;
            }
            if opened != fingerprint {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            let named_stat = stat_at(
                mode,
                parent_file,
                resources.names.get(name)?,
                DailyEvidenceError::SourceChanged,
            )?;
            verify_path_fingerprint(&named_stat, fingerprint, raw_owner)?;
            Ok(HeldFile {
                file: pending.take().expect("validated pending owner"),
                parent,
                name,
                fingerprint,
            })
        })();
        preserve_before_retire(mode, result, || {
            if let Some(file) = &mut pending {
                file.retire();
            }
        })
    }

    fn verify_lock_path_fingerprint(
        stat: &rustix::fs::Stat,
        expected: LockFingerprint,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        let actual = LockFingerprint::from_stat(stat, raw_uid)?;
        if actual != expected {
            return Err(DailyEvidenceError::SourceChanged);
        }
        Ok(())
    }

    fn open_lock_file<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        parent: usize,
        name: NameRange,
    ) -> Result<(), M::Error> {
        mode.before()?;
        let resources = &mut scan.resources;
        let raw_uid = scan.scope.trusted_raw_owner_uid;
        let name_os = resources.names.get(name)?;
        let parent_file = resources
            .directories
            .get(parent)
            .ok_or(DailyEvidenceError::Io)?
            .file
            .file();
        let pre_stat = stat_at(
            mode,
            parent_file,
            name_os,
            DailyEvidenceError::MissingControl,
        )?;
        let fingerprint = LockFingerprint::from_stat(&pre_stat, raw_uid)?;
        let file = resources.descriptors.open::<_, M::Error>(|| {
            mode.before()?;
            let descriptor = openat(
                parent_file,
                name_os,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| {
                M::Error::from(errno_error(error, DailyEvidenceError::MissingControl))
            })?;
            Ok(mode.adopt(
                File::from(descriptor),
                #[cfg(test)]
                DescriptorRole::Lock,
            ))
        })?;
        // Retain the real lock owner in the final construction field before
        // any post-open check can fail; even pre-acquisition failure closes
        // the directory graph before this sole lock descriptor.
        install_pending(
            &mut resources.lock_file,
            HeldLock {
                file,
                parent,
                name,
                fingerprint,
            },
        );
        let held = resources.lock_file.as_ref().ok_or(DailyEvidenceError::Io)?;
        let opened_stat = effect(mode, || fstat(held.file.file()).map_err(errno_io))?;
        #[cfg(test)]
        held.file.observe_identity(&opened_stat);
        effect(mode, || hit_phase(CapturePhase::AfterLockOpen))?;
        let opened = LockFingerprint::from_stat(&opened_stat, raw_uid)?;
        if opened != fingerprint {
            return Err(DailyEvidenceError::SourceChanged.into());
        }
        let named_stat = stat_at(
            mode,
            parent_file,
            resources.names.get(name)?,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_lock_path_fingerprint(&named_stat, fingerprint, raw_uid)?;
        Ok(())
    }

    fn revalidate_held_lock<M: SnapshotMode>(
        mode: &M,
        held: &HeldLock<M::Handle>,
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        let parent = directories
            .get(held.parent)
            .ok_or(DailyEvidenceError::SourceChanged)?;
        let descriptor_stat = effect(mode, || fstat(held.file.file()).map_err(errno_io))?;
        verify_lock_path_fingerprint(&descriptor_stat, held.fingerprint, raw_uid)?;
        let named = stat_at(
            mode,
            parent.file.file(),
            names.get(held.name)?,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_lock_path_fingerprint(&named, held.fingerprint, raw_uid).map_err(Into::into)
    }

    fn no_archive_acl<M: SnapshotMode>(mode: &M, file: &File) -> Result<(), M::Error> {
        for name in ["system.posix_acl_access", "system.posix_acl_default"] {
            let mut byte = [0u8; 1];
            mode.before()?;
            match rustix::fs::fgetxattr(file, name, byte.as_mut_slice()) {
                Err(Errno::NODATA) => {}
                _ => return Err(DailyEvidenceError::UnsafePath.into()),
            }
        }
        Ok(())
    }

    fn revalidate_archive_coordination<M: SnapshotMode>(
        mode: &M,
        held: &ResourceGuard<M::Handle>,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        let lock = &held._lock_file;
        let info = effect(mode, || fstat(lock.file.file()).map_err(errno_io))?;
        if info.st_uid != raw_uid
            || info.st_gid != 0
            || info.st_mode & 0o7777 != 0o600
            || info.st_size != 0
        {
            return Err(DailyEvidenceError::SourceChanged.into());
        }
        no_archive_acl(mode, lock.file.file())?;
        let mut cursor = Some(lock.parent);
        while let Some(index) = cursor {
            let directory = &held._directories[index];
            let info = effect(mode, || fstat(directory.file.file()).map_err(errno_io))?;
            if info.st_uid != 0
                || info.st_gid != 0
                || info.st_mode & 0o7022 != 0
                || (index == lock.parent && info.st_mode & 0o7777 != 0o700)
            {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            no_archive_acl(mode, directory.file.file())?;
            cursor = directory.parent;
        }
        Ok(())
    }

    fn open_provider_file<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        parent: usize,
        file_entry: &crate::storage::FileEntry,
        retained: &mut RetainedBytes,
    ) -> Result<M::Leaf, M::Error> {
        let mut state: PendingLeaf<M::Handle> = PendingLeaf::default();
        let result: Result<(), M::Error> = (|| {
            mode.before()?;
            validate_file_name(&file_entry.file_name)?;
            if file_entry.size_bytes == 0 {
                return Err(DailyEvidenceError::SizeMismatch.into());
            }
            let resources = &mut scan.resources;
            let raw_uid = scan.scope.trusted_raw_owner_uid;
            let limits = &scan.limits;
            let name_range = resources.names.append(file_entry.file_name.as_bytes())?;
            let parent_file = resources
                .directories
                .get(parent)
                .ok_or(DailyEvidenceError::Io)?
                .file
                .file();
            let name = resources.names.get(name_range)?;
            let pre_stat = stat_at(
                mode,
                parent_file,
                resources.names.get(name_range)?,
                DailyEvidenceError::MissingCommittedEvidence,
            )?;
            let fingerprint =
                Fingerprint::from_stat(&pre_stat, NodeType::RegularFile, Some(raw_uid))?;
            let actual_size = checked_stat_size(fingerprint)?;
            let expected_size = usize::try_from(file_entry.size_bytes)
                .map_err(|_| DailyEvidenceError::SizeMismatch)?;
            if expected_size > limits.values.max_source_file_bytes
                || actual_size > limits.values.max_source_file_bytes
            {
                return Err(DailyEvidenceError::SourceScanLimit.into());
            }
            if actual_size != expected_size {
                return Err(DailyEvidenceError::SizeMismatch.into());
            }
            retained.reserve(actual_size)?;
            state.fingerprint = Some(fingerprint);
            state.name = Some(name_range);
            state.file = Some(resources.descriptors.open::<_, M::Error>(|| {
                mode.before()?;
                let descriptor = openat(
                    parent_file,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|error| {
                    M::Error::from(errno_error(
                        error,
                        DailyEvidenceError::MissingCommittedEvidence,
                    ))
                })?;
                Ok(mode.adopt(
                    File::from(descriptor),
                    #[cfg(test)]
                    DescriptorRole::Provider,
                ))
            })?);
            let file = state.file.as_ref().expect("actual provider owner");
            let opened_stat = effect(mode, || fstat(file.file()).map_err(errno_io))?;
            #[cfg(test)]
            file.observe_identity(&opened_stat);
            let opened =
                Fingerprint::from_stat(&opened_stat, NodeType::RegularFile, Some(raw_uid))?;
            if opened != fingerprint {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            let named_stat = stat_at(
                mode,
                parent_file,
                resources.names.get(name_range)?,
                DailyEvidenceError::SourceChanged,
            )?;
            verify_path_fingerprint(&named_stat, fingerprint, Some(raw_uid))?;
            if scan.archive_read {
                no_archive_acl(mode, file.file())?;
            }
            capture_returned(&mut state.bytes, || {
                read_exact_captured_in(
                    mode,
                    file.file(),
                    actual_size,
                    limits.values.max_source_file_bytes,
                )
            })?;
            if scan.archive_read {
                no_archive_acl(mode, file.file())?;
            }
            let named_after = stat_at(mode, parent_file, name, DailyEvidenceError::SourceChanged)?;
            verify_path_fingerprint(
                &effect(mode, || fstat(file.file()).map_err(errno_io))?,
                fingerprint,
                Some(raw_uid),
            )?;
            verify_path_fingerprint(&named_after, fingerprint, Some(raw_uid))?;
            let hash = ContentHash::from_bytes(state.bytes.as_ref().expect("actual returned read"));
            state.content_hash = Some(hash);
            if state.content_hash.as_ref() != Some(&file_entry.content_hash) {
                return Err(DailyEvidenceError::HashMismatch.into());
            }
            effect(mode, || hit_phase(CapturePhase::AfterBodyCapture))?;
            Ok(())
        })();
        let result = preserve_before_retire(mode, result, || {
            if let Some(file) = &mut state.file {
                file.retire();
            }
        });
        match result {
            Ok(()) => Ok(M::leaf(LeafParts {
                bytes: state.bytes.take().expect("returned provider bytes"),
                content_hash: state.content_hash.take().expect("computed provider hash"),
                file: state.file.take().expect("validated provider owner"),
                fingerprint: state.fingerprint.take().expect("provider fingerprint"),
                parent_index: parent,
                name: state.name.take().expect("provider name"),
            })),
            Err(error) => {
                mode.returned_leaf(ReturnedLeafData {
                    bytes: state.bytes.take(),
                    content_hash: state.content_hash.take(),
                });
                Err(error)
            }
        }
    }

    fn capture_batch<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        current: &mut CaptureBatchState<M::Handle, M::Leaf>,
        batches: &[M::Batch],
        retained: &mut RetainedBytes,
        raw_index: usize,
        manifest: &Arc<Vec<u8>>,
    ) -> Result<M::Batch, M::Error> {
        mode.before()?;
        let entry = &current
            .pending
            .as_ref()
            .expect("actual pending batch")
            .entry;
        effect(mode, || hit_phase(CapturePhase::BeforeBatchTreeOpen))?;
        #[cfg(test)]
        observe_resource(ObservationEvent::BatchTreeOpen);
        let raw_uid = scan.scope.trusted_raw_owner_uid;
        let limits = scan.limits.values;
        let provider_name = format!("provider={}", scan.scope.provider());
        let provider_directory = find_or_open_child(
            mode,
            scan,
            raw_index,
            provider_name.as_bytes(),
            Some(raw_uid),
        )?;
        let market_name = format!("market={}", scan.scope.market());
        let market_directory = find_or_open_child(
            mode,
            scan,
            provider_directory,
            market_name.as_bytes(),
            Some(raw_uid),
        )?;
        let date_name = format!("date={}", entry.date);
        let date_directory = find_or_open_child(
            mode,
            scan,
            market_directory,
            date_name.as_bytes(),
            Some(raw_uid),
        )?;
        let batch_name = format!("batch={}", entry.batch_id);
        let batch_directory = open_child_directory(
            mode,
            scan,
            date_directory,
            batch_name.as_bytes(),
            Some(raw_uid),
            DailyEvidenceError::MissingCommittedEvidence,
        )?;

        let mut seen = Vec::new();
        seen.try_reserve_exact(entry.files.len())
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        seen.resize(entry.files.len(), false);
        verify_batch_membership(
            mode,
            &scan.resources.directories[batch_directory],
            entry,
            &mut seen,
            DailyEvidenceError::MissingCommittedEvidence,
        )?;

        let metadata_name = scan.resources.names.append(b"batch.json")?;
        current.metadata = Some(open_leaf_control(
            mode,
            scan,
            batch_directory,
            metadata_name,
            DailyEvidenceError::MissingCommittedEvidence,
            Some(raw_uid),
            Some(limits.max_batch_metadata_bytes),
        )?);
        let metadata = current.metadata.as_ref().expect("actual metadata owner");
        let metadata_size = checked_stat_size(metadata.fingerprint)?;
        retained.reserve(metadata_size)?;
        if scan.archive_read {
            no_archive_acl(mode, metadata.file.file())?;
        }
        capture_returned(&mut current.metadata_bytes, || {
            read_exact_captured_in(
                mode,
                metadata.file.file(),
                metadata_size,
                limits.max_batch_metadata_bytes,
            )
        })?;
        if scan.archive_read {
            no_archive_acl(mode, metadata.file.file())?;
        }
        let metadata_bytes = current
            .metadata_bytes
            .as_ref()
            .expect("returned metadata bytes");
        revalidate_held_file(
            mode,
            metadata,
            &scan.resources.directories,
            &scan.resources.names,
            Some(raw_uid),
        )?;
        {
            let stored = parse_metadata(metadata_bytes, &scan.limits)?;
            if &stored != entry {
                return Err(DailyEvidenceError::BatchMetadataMismatch.into());
            }
        }
        // `stored` and all of its owned metadata are destroyed at the scope
        // boundary above, before any body-bearing batch or buffer is built.
        let metadata_hash = ContentHash::from_bytes(metadata_bytes);
        let metadata = current.metadata.take().expect("validated metadata owner");
        current.batch_json = Some(M::leaf(LeafParts {
            bytes: current
                .metadata_bytes
                .take()
                .expect("returned metadata bytes"),
            content_hash: metadata_hash,
            file: metadata.file,
            fingerprint: metadata.fingerprint,
            parent_index: metadata.parent,
            name: metadata.name,
        }));
        effect(mode, || hit_phase(CapturePhase::AfterMetadataCapture))?;

        let next_file_count = batches
            .iter()
            .try_fold(entry.files.len(), |sum, batch| {
                sum.checked_add(batch.entry().files.len())
            })
            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        if next_file_count > limits.max_files {
            return Err(DailyEvidenceError::SourceScanLimit.into());
        }

        current
            .files
            .try_reserve_exact(entry.files.len())
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        for file_entry in &entry.files {
            append_completed(
                &mut current.files,
                open_provider_file(mode, scan, batch_directory, file_entry, retained)?,
            );
        }
        verify_batch_membership(
            mode,
            &scan.resources.directories[batch_directory],
            entry,
            &mut seen,
            DailyEvidenceError::SourceChanged,
        )?;
        let pending = current.pending.take().expect("completed original batch");
        let batch = M::batch(BatchParts {
            entry: pending.entry,
            manifest: Arc::clone(manifest),
            manifest_line: pending.line,
            manifest_line_sha256: pending.line_hash,
            manifest_line_number: pending.line_number,
            files: std::mem::take(&mut current.files),
            batch_json: current.batch_json.take().expect("completed metadata leaf"),
        });
        Ok(batch)
    }

    fn find_or_open_child<M: SnapshotMode>(
        mode: &M,
        scan: &mut ScanContext<M::Handle>,
        parent: usize,
        name: &[u8],
        raw_owner: Option<u32>,
    ) -> Result<usize, M::Error> {
        mode.before()?;
        if let Some(index) = scan.resources.directories.iter().position(|candidate| {
            candidate.parent == Some(parent)
                && candidate
                    .name
                    .and_then(|range| scan.resources.names.get(range).ok())
                    .is_some_and(|candidate_name| candidate_name.as_bytes() == name)
        }) {
            return Ok(index);
        }
        open_child_directory(
            mode,
            scan,
            parent,
            name,
            raw_owner,
            DailyEvidenceError::MissingCommittedEvidence,
        )
    }

    fn next_borrowed<'entry, 'buffer, M: SnapshotMode, F: AsFd>(
        mode: &M,
        directory: &'entry mut RawDir<'buffer, F>,
    ) -> Result<Option<rustix::fs::RawDirEntry<'entry>>, M::Error> {
        mode.before()?;
        directory
            .next()
            .transpose()
            .map_err(|error| M::Error::from(errno_io(error)))
    }

    fn verify_batch_membership<M: SnapshotMode>(
        mode: &M,
        batch_directory: &DirectoryNode<M::Handle>,
        entry: &ManifestEntry,
        seen: &mut [bool],
        missing: DailyEvidenceError,
    ) -> Result<(), M::Error> {
        mode.before()?;
        if seen.len() != entry.files.len() {
            return Err(DailyEvidenceError::ArithmeticOverflow.into());
        }
        seen.fill(false);
        let mut metadata_seen = false;
        let mut buffer = [MaybeUninit::<u8>::uninit(); DIRECTORY_BUFFER_BYTES];
        effect(mode, || {
            seek(
                batch_directory.file.file().as_fd(),
                RustixSeekFrom::Start(0),
            )
            .map_err(errno_io)
        })?;
        let mut directory = RawDir::new(batch_directory.file.file().as_fd(), &mut buffer);
        while let Some(item) = next_borrowed(mode, &mut directory)? {
            let name = item.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if name == b"batch.json" {
                if metadata_seen {
                    return Err(DailyEvidenceError::UnexpectedBatchEntry.into());
                }
                metadata_seen = true;
                continue;
            }
            let Some(index) = entry
                .files
                .iter()
                .position(|file| file.file_name.as_bytes() == name)
            else {
                return Err(DailyEvidenceError::UnexpectedBatchEntry.into());
            };
            let Some(slot) = seen.get_mut(index) else {
                return Err(DailyEvidenceError::UnexpectedBatchEntry.into());
            };
            if *slot {
                return Err(DailyEvidenceError::UnexpectedBatchEntry.into());
            }
            *slot = true;
        }
        if !metadata_seen || seen.iter().any(|seen_file| !seen_file) {
            return Err(missing.into());
        }
        Ok(())
    }

    fn revalidate_held_file<M: SnapshotMode>(
        mode: &M,
        held: &HeldFile<M::Handle>,
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_owner: Option<u32>,
    ) -> Result<(), M::Error> {
        mode.before()?;
        let parent = directories
            .get(held.parent)
            .ok_or(DailyEvidenceError::SourceChanged)?;
        let name = names.get(held.name)?;
        let descriptor_stat = effect(mode, || fstat(held.file.file()).map_err(errno_io))?;
        verify_path_fingerprint(&descriptor_stat, held.fingerprint, raw_owner)?;
        let named = stat_at(
            mode,
            parent.file.file(),
            name,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_path_fingerprint(&named, held.fingerprint, raw_owner).map_err(Into::into)
    }

    fn revalidate_directories<M: SnapshotMode>(
        mode: &M,
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        for directory in directories {
            let descriptor_stat = effect(mode, || fstat(directory.file.file()).map_err(errno_io))?;
            verify_path_fingerprint(&descriptor_stat, directory.fingerprint, None)?;
            if let (Some(parent_index), Some(name_range)) = (directory.parent, directory.name) {
                let parent = directories
                    .get(parent_index)
                    .ok_or(DailyEvidenceError::SourceChanged)?;
                let name = names.get(name_range)?;
                let named = stat_at(
                    mode,
                    parent.file.file(),
                    name,
                    DailyEvidenceError::SourceChanged,
                )?;
                let raw_owner = if directory.raw_protected {
                    Some(raw_uid)
                } else {
                    None
                };
                verify_path_fingerprint(&named, directory.fingerprint, raw_owner)?;
            }
        }
        Ok(())
    }

    fn revalidate_all_batch_files<M: SnapshotMode>(
        mode: &M,
        batches: &[M::Batch],
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        for batch in batches {
            revalidate_capture(mode, batch.metadata(), directories, names, raw_uid)?;
            for captured in batch.files() {
                revalidate_capture(mode, captured, directories, names, raw_uid)?;
            }
        }
        Ok(())
    }

    fn revalidate_capture<M: SnapshotMode>(
        mode: &M,
        capture: &M::Leaf,
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        let parent = directories
            .get(capture.parent_index())
            .ok_or(DailyEvidenceError::SourceChanged)?;
        let parent_stat = parent.file.file();
        let name = names.get(capture.name())?;
        let descriptor_stat = effect(mode, || fstat(capture.handle().file()).map_err(errno_io))?;
        verify_path_fingerprint(&descriptor_stat, capture.fingerprint(), Some(raw_uid))?;
        let named = stat_at(mode, parent_stat, name, DailyEvidenceError::SourceChanged)?;
        verify_path_fingerprint(&named, capture.fingerprint(), Some(raw_uid)).map_err(Into::into)
    }

    fn final_byte_checks<M: SnapshotMode>(
        mode: &M,
        manifest: &HeldFile<M::Handle>,
        manifest_bytes: &[u8],
        batches: &[M::Batch],
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        compare_held_bytes(mode, manifest, manifest_bytes, directories, names, raw_uid)?;
        for batch in batches {
            compare_captured_bytes(
                mode,
                batch.metadata(),
                batch.metadata().bytes(),
                directories,
                names,
                raw_uid,
            )?;
            for captured in batch.files() {
                compare_captured_bytes(
                    mode,
                    captured,
                    captured.bytes(),
                    directories,
                    names,
                    raw_uid,
                )?;
            }
        }
        Ok(())
    }

    fn compare_held_bytes<M: SnapshotMode>(
        mode: &M,
        held: &HeldFile<M::Handle>,
        expected: &[u8],
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        revalidate_held_file(mode, held, directories, names, Some(raw_uid))?;
        compare_descriptor_bytes(mode, held.file.file(), expected)?;
        revalidate_held_file(mode, held, directories, names, Some(raw_uid))
    }

    fn compare_captured_bytes<M: SnapshotMode>(
        mode: &M,
        capture: &M::Leaf,
        expected: &[u8],
        directories: &[DirectoryNode<M::Handle>],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), M::Error> {
        mode.before()?;
        revalidate_capture(mode, capture, directories, names, raw_uid)?;
        compare_descriptor_bytes(mode, capture.handle().file(), expected)?;
        revalidate_capture(mode, capture, directories, names, raw_uid)
    }

    fn compare_descriptor_bytes<M: SnapshotMode>(
        mode: &M,
        file: &File,
        expected: &[u8],
    ) -> Result<(), M::Error> {
        mode.before()?;
        let mut reader = file;
        effect(mode, || reader.seek(SeekFrom::Start(0)).map_err(io_error))?;
        let mut scratch = [0u8; READ_BUFFER_BYTES];
        let mut offset = 0usize;
        while offset < expected.len() {
            let wanted = (expected.len() - offset).min(scratch.len());
            let count = effect(mode, || {
                reader.read(&mut scratch[..wanted]).map_err(io_error)
            })?;
            if count == 0 {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            let next = offset
                .checked_add(count)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            if expected.get(offset..next) != Some(&scratch[..count]) {
                return Err(DailyEvidenceError::SourceChanged.into());
            }
            offset = next;
        }
        let mut probe = [0u8; 1];
        if effect(mode, || reader.read(&mut probe).map_err(io_error))? != 0 {
            return Err(DailyEvidenceError::SourceChanged.into());
        }
        Ok(())
    }

    fn read_exact_captured_in<M: SnapshotMode>(
        mode: &M,
        mut reader: impl Read,
        expected: usize,
        byte_cap: usize,
    ) -> Result<Vec<u8>, M::Error> {
        mode.before()?;
        if expected > byte_cap {
            return Err(DailyEvidenceError::SourceScanLimit.into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        #[cfg(test)]
        observe_resource(ObservationEvent::EvidenceBufferReady {
            requested: expected,
            capacity: bytes.capacity(),
        });
        let mut scratch = [0u8; READ_BUFFER_BYTES];
        while bytes.len() < expected {
            let wanted = (expected - bytes.len()).min(scratch.len());
            let count = effect(mode, || {
                reader.read(&mut scratch[..wanted]).map_err(io_error)
            })?;
            #[cfg(test)]
            observe_resource(ObservationEvent::ReadBytes(count));
            if count == 0 {
                return Err(DailyEvidenceError::SizeMismatch.into());
            }
            let next = bytes
                .len()
                .checked_add(count)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            bytes.extend_from_slice(&scratch[..count]);
            if next > byte_cap {
                return Err(DailyEvidenceError::SourceScanLimit.into());
            }
        }
        let mut probe = [0u8; 1];
        let probe_count = effect(mode, || reader.read(&mut probe).map_err(io_error))?;
        #[cfg(test)]
        observe_resource(ObservationEvent::ReadBytes(probe_count));
        if probe_count != 0 {
            return Err((if expected >= byte_cap {
                DailyEvidenceError::SourceScanLimit
            } else {
                DailyEvidenceError::SourceChanged
            })
            .into());
        }
        Ok(bytes)
    }

    struct ParsedManifest {
        pending: Vec<PendingBatch>,
        batch_index: Vec<(BatchId, usize)>,
        records: usize,
        total_files: usize,
    }

    fn parse_manifest(
        scope: &ApprovedRawScope,
        limits: &ScanLimits,
        bytes: &[u8],
    ) -> Result<ParsedManifest, DailyEvidenceError> {
        let mut pending: Vec<PendingBatch> = Vec::new();
        let mut index = Vec::<(BatchId, usize)>::new();
        let mut records = 0usize;
        let mut total_files = 0usize;
        let mut physical_line = 1usize;
        let mut start = 0usize;
        while start < bytes.len() {
            let newline = bytes[start..].iter().position(|byte| *byte == b'\n');
            let (end, terminated) = match newline {
                Some(relative) => (
                    start
                        .checked_add(relative)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?,
                    true,
                ),
                None => (bytes.len(), false),
            };
            let line = &bytes[start..end];
            if !line.iter().all(u8::is_ascii_whitespace) {
                if terminated {
                    records = records
                        .checked_add(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                    if records > limits.values.max_manifest_records {
                        return Err(DailyEvidenceError::SourceScanLimit);
                    }
                }
                let depth = json_depth(line)?;
                if depth > limits.values.max_metadata_json_depth {
                    return Err(DailyEvidenceError::SourceScanLimit);
                }
                let entry = match parse_bounded_entry(line, limits.values.max_files_per_batch) {
                    Ok(entry) => entry,
                    Err(BoundedParseError::Resource(error)) => return Err(error),
                    Err(BoundedParseError::Json(error)) if !terminated && error.is_eof() => break,
                    Err(BoundedParseError::Json(_)) => {
                        return Err(DailyEvidenceError::InvalidManifest);
                    }
                };
                if !terminated {
                    records = records
                        .checked_add(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                    if records > limits.values.max_manifest_records {
                        return Err(DailyEvidenceError::SourceScanLimit);
                    }
                }
                if entry.provider != scope.provider() || entry.market != scope.market() {
                    return Err(DailyEvidenceError::ScopeMismatch);
                }
                let mut names = Vec::<&str>::new();
                names
                    .try_reserve_exact(entry.files.len())
                    .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                for file in &entry.files {
                    validate_file_name(&file.file_name)?;
                    if names.contains(&file.file_name.as_str()) {
                        return Err(DailyEvidenceError::DuplicateFile);
                    }
                    names.push(file.file_name.as_str());
                }

                match index.binary_search_by_key(&entry.batch_id, |(batch_id, _)| *batch_id) {
                    Ok(existing_index) => {
                        let first = pending
                            .get(index[existing_index].1)
                            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                        if first.entry != entry {
                            return Err(DailyEvidenceError::ManifestConflict);
                        }
                    }
                    Err(insert_at) => {
                        let next_batch_count = pending
                            .len()
                            .checked_add(1)
                            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                        if next_batch_count > limits.values.max_batches {
                            return Err(DailyEvidenceError::SourceScanLimit);
                        }
                        let next_files = total_files
                            .checked_add(entry.files.len())
                            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                        if next_files > limits.values.max_files {
                            return Err(DailyEvidenceError::SourceScanLimit);
                        }
                        pending
                            .try_reserve(1)
                            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                        index
                            .try_reserve(1)
                            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                        let batch_index = pending.len();
                        pending.push(PendingBatch {
                            entry,
                            line: start..end,
                            line_hash: ContentHash::from_bytes(line),
                            line_number: physical_line,
                        });
                        index.insert(
                            insert_at,
                            (pending[batch_index].entry.batch_id, batch_index),
                        );
                        total_files = next_files;
                    }
                }
            }
            if terminated {
                start = end
                    .checked_add(1)
                    .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                if start < bytes.len() {
                    physical_line = physical_line
                        .checked_add(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                }
            } else {
                break;
            }
        }
        Ok(ParsedManifest {
            pending,
            batch_index: index,
            records,
            total_files,
        })
    }

    fn json_depth(bytes: &[u8]) -> Result<usize, DailyEvidenceError> {
        let mut depth = 0usize;
        let mut maximum = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for byte in bytes {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match *byte {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth = depth
                        .checked_add(1)
                        .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
                    maximum = maximum.max(depth);
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        Ok(maximum)
    }

    enum BoundedParseError {
        Json(serde_json::Error),
        Resource(DailyEvidenceError),
    }

    #[derive(Clone, Copy)]
    enum ParseResource {
        FileLimit,
        Allocation,
    }

    struct ParseState {
        resource: std::cell::Cell<Option<ParseResource>>,
        max_files: usize,
    }

    struct EntrySeed<'a> {
        state: &'a ParseState,
    }

    impl<'de> DeserializeSeed<'de> for EntrySeed<'_> {
        type Value = ManifestEntry;

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_struct(
                "ManifestEntry",
                &[
                    "batch_id",
                    "provider",
                    "market",
                    "date",
                    "retrieved_at",
                    "mode",
                    "entitlement_reference",
                    "files",
                ],
                EntryVisitor { state: self.state },
            )
        }
    }

    struct EntryVisitor<'a> {
        state: &'a ParseState,
    }

    impl<'de> Visitor<'de> for EntryVisitor<'_> {
        type Value = ManifestEntry;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a Raw manifest entry")
        }

        fn visit_map<A>(self, mut map: A) -> Result<ManifestEntry, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut batch_id = None;
            let mut provider = None;
            let mut market = None;
            let mut date = None;
            let mut retrieved_at = None;
            let mut mode = None;
            let mut entitlement_reference = None;
            let mut files = None;
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "batch_id" => {
                        reject_duplicate(&batch_id, "batch_id")?;
                        set_once(&mut batch_id, map.next_value()?, "batch_id")?;
                    }
                    "provider" => {
                        reject_duplicate(&provider, "provider")?;
                        set_once(&mut provider, map.next_value()?, "provider")?;
                    }
                    "market" => {
                        reject_duplicate(&market, "market")?;
                        set_once(&mut market, map.next_value()?, "market")?;
                    }
                    "date" => {
                        reject_duplicate(&date, "date")?;
                        set_once(&mut date, map.next_value()?, "date")?;
                    }
                    "retrieved_at" => {
                        reject_duplicate(&retrieved_at, "retrieved_at")?;
                        set_once(&mut retrieved_at, map.next_value()?, "retrieved_at")?
                    }
                    "mode" => {
                        reject_duplicate(&mode, "mode")?;
                        set_once(&mut mode, map.next_value()?, "mode")?;
                    }
                    "entitlement_reference" => {
                        reject_duplicate(&entitlement_reference, "entitlement_reference")?;
                        set_once(
                            &mut entitlement_reference,
                            map.next_value()?,
                            "entitlement_reference",
                        )?;
                    }
                    "files" => {
                        if files.is_some() {
                            return Err(de::Error::duplicate_field("files"));
                        }
                        files = Some(map.next_value_seed(FilesSeed { state: self.state })?);
                    }
                    _ => {
                        let _: IgnoredAny = map.next_value()?;
                    }
                }
            }
            Ok(ManifestEntry {
                batch_id: batch_id.ok_or_else(|| de::Error::missing_field("batch_id"))?,
                provider: provider.ok_or_else(|| de::Error::missing_field("provider"))?,
                market: market.ok_or_else(|| de::Error::missing_field("market"))?,
                date: date.ok_or_else(|| de::Error::missing_field("date"))?,
                retrieved_at: retrieved_at
                    .ok_or_else(|| de::Error::missing_field("retrieved_at"))?,
                mode: mode.ok_or_else(|| de::Error::missing_field("mode"))?,
                entitlement_reference: entitlement_reference.unwrap_or(None),
                files: files.ok_or_else(|| de::Error::missing_field("files"))?,
            })
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<ManifestEntry, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let batch_id = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(0, &self))?;
            let provider = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(1, &self))?;
            let market = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(2, &self))?;
            let date = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(3, &self))?;
            let retrieved_at = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(4, &self))?;
            let mode = sequence
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(5, &self))?;
            let entitlement_reference = sequence.next_element()?.unwrap_or(None);
            let files = sequence
                .next_element_seed(FilesSeed { state: self.state })?
                .ok_or_else(|| de::Error::invalid_length(7, &self))?;
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(de::Error::invalid_length(9, &self));
            }
            Ok(ManifestEntry {
                batch_id,
                provider,
                market,
                date,
                retrieved_at,
                mode,
                entitlement_reference,
                files,
            })
        }
    }

    fn set_once<T, E>(slot: &mut Option<T>, value: T, field: &'static str) -> Result<(), E>
    where
        E: de::Error,
    {
        if slot.is_some() {
            return Err(E::duplicate_field(field));
        }
        *slot = Some(value);
        Ok(())
    }

    fn reject_duplicate<T, E>(slot: &Option<T>, field: &'static str) -> Result<(), E>
    where
        E: de::Error,
    {
        if slot.is_some() {
            Err(E::duplicate_field(field))
        } else {
            Ok(())
        }
    }

    struct FilesSeed<'a> {
        state: &'a ParseState,
    }

    impl<'de> DeserializeSeed<'de> for FilesSeed<'_> {
        type Value = Vec<crate::storage::FileEntry>;

        fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_seq(FilesVisitor { state: self.state })
        }
    }

    struct FilesVisitor<'a> {
        state: &'a ParseState,
    }

    impl<'de> Visitor<'de> for FilesVisitor<'_> {
        type Value = Vec<crate::storage::FileEntry>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded Raw file-entry sequence")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut files = Vec::new();
            loop {
                if files.len() == self.state.max_files {
                    match sequence.next_element_seed(FileLimitSeed { state: self.state }) {
                        Ok(Some(())) => return Ok(files),
                        Ok(None) => return Ok(files),
                        Err(error) => return Err(error),
                    }
                }
                files.try_reserve(1).map_err(|_| {
                    self.state.resource.set(Some(ParseResource::Allocation));
                    de::Error::custom("bounded Raw allocation failed")
                })?;
                let Some(file) = sequence.next_element::<crate::storage::FileEntry>()? else {
                    break;
                };
                #[cfg(test)]
                observe_resource(ObservationEvent::FileEntryMaterialized);
                files.push(file);
            }
            Ok(files)
        }
    }

    struct FileLimitSeed<'a> {
        state: &'a ParseState,
    }

    impl<'de> DeserializeSeed<'de> for FileLimitSeed<'_> {
        type Value = ();

        fn deserialize<D>(self, _deserializer: D) -> Result<(), D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            self.state.resource.set(Some(ParseResource::FileLimit));
            Err(de::Error::custom("bounded Raw file count exceeded"))
        }
    }

    fn parse_bounded_entry(
        bytes: &[u8],
        max_files: usize,
    ) -> Result<ManifestEntry, BoundedParseError> {
        let state = ParseState {
            resource: std::cell::Cell::new(None),
            max_files,
        };
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let parsed = EntrySeed { state: &state }.deserialize(&mut deserializer);
        let parsed = match parsed {
            Ok(entry) => deserializer.end().map(|()| entry),
            Err(error) => Err(error),
        };
        match (parsed, state.resource.get()) {
            (_, Some(ParseResource::FileLimit)) => Err(BoundedParseError::Resource(
                DailyEvidenceError::SourceScanLimit,
            )),
            (_, Some(ParseResource::Allocation)) => Err(BoundedParseError::Resource(
                DailyEvidenceError::AllocationFailed,
            )),
            (Ok(entry), None) => Ok(entry),
            (Err(error), None) => Err(BoundedParseError::Json(error)),
        }
    }

    fn parse_metadata(
        bytes: &[u8],
        limits: &ScanLimits,
    ) -> Result<ManifestEntry, DailyEvidenceError> {
        if json_depth(bytes)? > limits.values.max_metadata_json_depth {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        match parse_bounded_entry(bytes, limits.values.max_files_per_batch) {
            Ok(entry) => Ok(entry),
            Err(BoundedParseError::Resource(error)) => Err(error),
            Err(BoundedParseError::Json(_)) => Err(DailyEvidenceError::InvalidBatchMetadata),
        }
    }

    #[cfg(test)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CapturePhase {
        AfterLockOpen,
        AfterSharedLock,
        AfterManifestCapture,
        AfterMetadataCapture,
        AfterBodyCapture,
        BeforeBatchTreeOpen,
        BeforeFinalBytes,
        BeforeFinalNames,
        BeforeLockRelease,
    }

    #[cfg(test)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DescriptorRole {
        Root,
        Directory,
        Lock,
        Manifest,
        Metadata,
        Provider,
    }

    #[cfg(test)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ObservedDescriptor {
        open_id: usize,
        raw_fd: i32,
        role: DescriptorRole,
    }

    #[cfg(test)]
    struct ObservedLifetime {
        descriptor: ObservedDescriptor,
        identity: Option<(u64, u64)>,
        closed: bool,
    }

    #[cfg(test)]
    struct OpenObservation {
        destination: Arc<Mutex<ResourceObservation>>,
        open_id: usize,
    }

    #[cfg(test)]
    impl OpenObservation {
        fn register(file: &File, role: DescriptorRole) -> Option<Self> {
            TEST_RESOURCE_OBSERVATION.with(|slot| {
                let destination = Arc::clone(slot.borrow().as_ref()?);
                let open_id = {
                    let mut observation = destination.lock().unwrap();
                    // This index is allocated once per open and never reused,
                    // regardless of native FD numbers or shared inode identity.
                    let open_id = observation.lifetimes.len();
                    let descriptor = ObservedDescriptor {
                        open_id,
                        raw_fd: file.as_raw_fd(),
                        role,
                    };
                    observation.lifetimes.push(ObservedLifetime {
                        descriptor,
                        identity: None,
                        closed: false,
                    });
                    observation
                        .events
                        .push(ObservationEvent::DescriptorOpened(descriptor));
                    open_id
                };
                Some(Self {
                    destination,
                    open_id,
                })
            })
        }

        fn record_identity(&self, device: u64, inode: u64) {
            let mut observation = self.destination.lock().unwrap();
            let lifetime = &mut observation.lifetimes[self.open_id];
            assert!(!lifetime.closed, "identity belongs to a live source owner");
            lifetime.identity = Some((device, inode));
        }

        fn record_closed(&self) {
            let mut observation = self.destination.lock().unwrap();
            let lifetime = &mut observation.lifetimes[self.open_id];
            assert!(!lifetime.closed, "each source owner is destroyed once");
            lifetime.closed = true;
            observation
                .events
                .push(ObservationEvent::DescriptorClosed(self.open_id));
        }
    }

    #[cfg(test)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ObservationEvent {
        DescriptorReserved,
        DescriptorOpenAttempt,
        DescriptorOpened(ObservedDescriptor),
        DescriptorClosed(usize),
        RootComponentsReserve(usize),
        RootComponentReserve(usize),
        EvidenceBytesReserved { amount: usize, total: usize },
        EvidenceBufferReady { requested: usize, capacity: usize },
        NamePoolReserve(usize),
        NameCopied(usize),
        ReadBytes(usize),
        RootComponentCopied,
        FileEntryMaterialized,
        BatchTreeOpen,
        ConstructionCleanupBeforeLockRelease,
    }

    #[cfg(test)]
    #[derive(Default)]
    struct ResourceObservation {
        events: Vec<ObservationEvent>,
        lifetimes: Vec<ObservedLifetime>,
    }

    #[cfg(test)]
    thread_local! {
        static TEST_RESOURCE_OBSERVATION: RefCell<Option<Arc<Mutex<ResourceObservation>>>> = const { RefCell::new(None) };
    }

    #[cfg(test)]
    fn observe_resource(event: ObservationEvent) {
        TEST_RESOURCE_OBSERVATION.with(|slot| {
            if let Some(observation) = slot.borrow().as_ref() {
                observation.lock().unwrap().events.push(event);
            }
        });
    }

    #[cfg(test)]
    fn with_resource_observation<T>(run: impl FnOnce(Arc<Mutex<ResourceObservation>>) -> T) -> T {
        struct Reset;

        impl Drop for Reset {
            fn drop(&mut self) {
                TEST_RESOURCE_OBSERVATION.with(|slot| *slot.borrow_mut() = None);
            }
        }

        let observation = Arc::new(Mutex::new(ResourceObservation::default()));
        TEST_RESOURCE_OBSERVATION.with(|slot| {
            *slot.borrow_mut() = Some(Arc::clone(&observation));
        });
        let _reset = Reset;
        run(observation)
    }

    #[cfg(test)]
    type TestPhaseHook = Box<dyn FnMut(CapturePhase) -> Result<(), DailyEvidenceError>>;

    #[cfg(test)]
    thread_local! {
        static TEST_PHASE_HOOK: RefCell<Option<TestPhaseHook>> = const { RefCell::new(None) };
    }

    #[cfg(test)]
    fn hit_phase(phase: CapturePhase) -> Result<(), DailyEvidenceError> {
        TEST_PHASE_HOOK.with(|hook| {
            let mut hook = hook.borrow_mut();
            if let Some(hook) = hook.as_mut() {
                hook(phase)
            } else {
                Ok(())
            }
        })
    }

    #[cfg(test)]
    fn with_phase_hook<T>(
        hook: impl FnMut(CapturePhase) -> Result<(), DailyEvidenceError> + 'static,
        run: impl FnOnce() -> T,
    ) -> T {
        struct Reset;

        impl Drop for Reset {
            fn drop(&mut self) {
                TEST_PHASE_HOOK.with(|hook| *hook.borrow_mut() = None);
            }
        }

        TEST_PHASE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        let _reset = Reset;
        run()
    }

    #[cfg(test)]
    mod checked_snapshot_tests {
        use super::*;
        use crate::checked_close::{Close, Errno, Issue};
        use std::cell::Cell;
        use std::panic::{AssertUnwindSafe, catch_unwind};

        struct Row(Box<str>);
        struct Body(Box<str>);
        struct Counted {
            name: &'static str,
            log: Rc<RefCell<Vec<&'static str>>>,
            result: Result<(), Errno>,
        }
        impl Close for Counted {
            fn close(self) -> Result<(), Errno> {
                self.log.borrow_mut().push(self.name);
                self.result
            }
        }
        fn counted(
            attempt: &Attempt,
            log: &Rc<RefCell<Vec<&'static str>>>,
            name: &'static str,
            result: Result<(), Errno>,
        ) -> Owner<Counted> {
            adopt_resource(
                attempt,
                Counted {
                    name,
                    log: Rc::clone(log),
                    result,
                },
                Role::Source,
            )
        }
        fn limits() -> ScanLimits {
            ScanLimits::new(ScanLimitValues {
                max_manifest_bytes: 1,
                max_manifest_records: 1,
                max_batches: 1,
                max_files_per_batch: 1,
                max_files: 1,
                max_batch_metadata_bytes: 1,
                max_source_file_bytes: 1,
                max_retained_bytes: 1,
                max_open_descriptors: 1,
                max_data_root_bytes: 1,
                max_data_root_components: 1,
                max_metadata_json_depth: 1,
            })
            .unwrap()
        }

        #[test]
        fn construction_cleanup_order() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let temporary = counted(&attempt, &log, "pending", Ok(()));
            let partial = counted(&attempt, &log, "partial-provider", Ok(()));
            let metadata = counted(&attempt, &log, "partial-metadata", Ok(()));
            let prior = counted(&attempt, &log, "prior-batch", Ok(()));
            let manifest = counted(&attempt, &log, "manifest", Ok(()));
            let directory = counted(&attempt, &log, "directory", Ok(()));
            let lock = RefCell::new(Some(counted(&attempt, &log, "lock", Ok(()))));
            retire_construction(
                || {
                    temporary.finish();
                },
                || {
                    partial.finish();
                    metadata.finish();
                    prior.finish();
                },
                || {
                    manifest.finish();
                },
                || {
                    directory.finish();
                },
                || log.borrow_mut().push("names"),
                || {
                    assert_eq!(attempt.report().live(), 1);
                    assert_eq!(lock.borrow().as_ref().unwrap().get().name, "lock");
                    log.borrow_mut().push("failure-hook");
                },
                || {
                    lock.borrow_mut().take().unwrap().finish();
                },
            );
            assert_eq!(
                &*log.borrow(),
                &[
                    "pending",
                    "partial-provider",
                    "partial-metadata",
                    "prior-batch",
                    "manifest",
                    "directory",
                    "names",
                    "failure-hook",
                    "lock"
                ]
            );
            assert_eq!(attempt.report().attempted(), 7);
            assert_eq!(attempt.report().closed(), 7);
            assert!(attempt.check());
        }

        #[test]
        fn gate_before_effects() {
            with_resource_observation(|observation| {
                let attempt = Attempt::default();
                let invocation = RawInvocation::new(&attempt);
                let result = CommittedRawReader::new(Path::new("/uninspected-checked-snapshot"))
                    .snapshot_checked_in(
                        &invocation,
                        &attempt,
                        &attempt,
                        ApprovedRawScope::new("kis", MARKET_KR, 0).unwrap(),
                        limits(),
                    );
                let failure = match result {
                    Err(failure) => failure,
                    Ok(_) => panic!("None gate"),
                };
                assert_eq!(failure.kind(), SnapshotFirstKind::Binding);
                assert!(failure.binding_failure().unwrap().body().is_none());
                assert_eq!(
                    failure.source_report().unwrap().first(),
                    Some(Issue::Gap(Gap::Unbound))
                );
                assert!(failure.retained().manifest_bytes().is_none());
                assert!(failure.retained().parsed_batch_count().is_none());
                assert!(observation.lock().unwrap().events.is_empty());
                assert!(observation.lock().unwrap().lifetimes.is_empty());
                assert_eq!(attempt.report().attempted(), 0);
            });
        }

        #[test]
        fn all_scope_tokens_even_dirty() {
            let actual = Attempt::default();
            let expected = Attempt::default();
            let captured = Attempt::default();
            let own = Attempt::default();
            let foreign_owner = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let held = counted(&foreign_owner, &log, "foreign", Ok(()));
            actual.gap(Gap::Unbound);
            let invocation = RawInvocation::new(&actual);
            let result = compare_snapshot_scopes(
                &invocation,
                &expected,
                &captured,
                &own,
                &[held.attempt().clone()],
            );
            assert!(!result.is_clean());
            assert!(
                result.expected().is_err() && result.captured().is_err() && result.own().is_err()
            );
            assert!(result.owners()[0].is_err());
            for scope in [&expected, &captured, &own, &foreign_owner] {
                assert_eq!(scope.report().first(), Some(Issue::Gap(Gap::ScopeMismatch)));
                assert_eq!(scope.report().attempted(), 0);
            }
            assert_eq!(foreign_owner.report().live(), 1);
            assert_eq!(actual.report().live(), 0);
            held.finish();
            assert_eq!(foreign_owner.report().closed(), 1);
            assert_eq!(actual.report().closed(), 0);
        }

        #[test]
        fn adopt_before_postopen() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let mode = Checked { control: &control };
            let log = Rc::new(RefCell::new(Vec::new()));
            let mut pending = Some(counted(&attempt, &log, "pending", Ok(())));
            let result = effect(&mode, || {
                assert_eq!(attempt.report().live(), 1);
                assert_eq!(attempt.report().attempted(), 0);
                Err::<(), _>(DailyEvidenceError::SourceChanged)
            });
            let result = preserve_before_retire(&mode, result, || {
                pending.take().unwrap().finish();
            });
            let first = checked_first(result.err().unwrap());
            assert_eq!(
                first.policy().unwrap().body(),
                Some(&DailyEvidenceError::SourceChanged)
            );
            assert_eq!(first.report().unwrap().attempted(), 0);
            assert_eq!(attempt.report().closed(), 1);
            assert_eq!(&*log.borrow(), &["pending"]);
        }

        #[test]
        fn lock_installed_early() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let mut lock = None;
            install_pending(&mut lock, counted(&attempt, &log, "lock", Ok(())));
            let lock = RefCell::new(lock);
            let directory = counted(&attempt, &log, "directory", Ok(()));
            let acquired = Cell::new(0);
            let result: Result<(), Body> = Err(Body("post-open-original".into()));
            let pointer = result.as_ref().err().unwrap().0.as_ptr();
            let result = retain_before_cleanup(
                result,
                |body| body,
                || {
                    retire_construction(
                        || {},
                        || {},
                        || {},
                        || {
                            directory.finish();
                        },
                        || {},
                        || {
                            assert!(lock.borrow().is_some());
                            assert_eq!(acquired.get(), 0);
                        },
                        || {
                            lock.borrow_mut().take().unwrap().finish();
                        },
                    );
                },
            );
            assert_eq!(result.err().unwrap().0.as_ptr(), pointer);
            assert_eq!(&*log.borrow(), &["directory", "lock"]);
        }

        #[test]
        fn try_lock_outcomes() {
            assert_eq!(shared_lock_result(Ok(())), Ok(()));
            assert_eq!(
                shared_lock_result(Err(std::io::ErrorKind::WouldBlock.into())),
                Err(DailyEvidenceError::LockBusy)
            );
            assert_eq!(
                shared_lock_result(Err(std::io::ErrorKind::PermissionDenied.into())),
                Err(DailyEvidenceError::PermissionDenied)
            );
            assert_eq!(
                shared_lock_result(Err(std::io::ErrorKind::Other.into())),
                Err(DailyEvidenceError::Io)
            );
        }

        #[test]
        fn temporary_policy_primary() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let log = Rc::new(RefCell::new(Vec::new()));
            let owner = counted(&attempt, &log, "unknown", Err(Errno::Interrupted));
            let body = Body("nonclone-original-error".into());
            let pointer = body.0.as_ptr();
            let result = retain_before_cleanup(
                Err::<(), _>(body),
                |body| {
                    invocation
                        .settle_local_in(&attempt, Err::<(), _>(body))
                        .err()
                        .unwrap()
                },
                || {
                    owner.finish();
                },
            );
            let failure = result.err().unwrap();
            assert_eq!(failure.body().unwrap().0.as_ptr(), pointer);
            assert_eq!(failure.report().attempted(), 0);
            assert_eq!(failure.report().live(), 1);
            assert_eq!(attempt.report().unknown(), 1);
            assert_eq!(&*log.borrow(), &["unknown"]);
            assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::Abandoned)));
            assert!(matches!(
                attempt.report().secondary().last(),
                Some(Issue::Close { .. })
            ));
        }

        #[test]
        fn success_late_close_only() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let log = Rc::new(RefCell::new(Vec::new()));
            let owner = counted(&attempt, &log, "unknown", Err(Errno::Io));
            let row = Row("actual-returned-value".into());
            let pointer = row.0.as_ptr();
            let result = OriginalOutcome(Ok::<_, Body>(row));
            let first: Option<SnapshotFirst> = None;
            owner.finish();
            let local = control.local();
            assert!(local.as_ref().err().unwrap().body().is_none());
            assert!(first.as_ref().and_then(SnapshotFirst::report).is_none());
            assert_eq!(result.0.ok().unwrap().0.as_ptr(), pointer);
            assert_eq!(attempt.report().unknown(), 1);
        }

        #[test]
        fn normal_guard_order() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let provider = counted(&attempt, &log, "provider", Ok(()));
            let metadata = counted(&attempt, &log, "metadata", Ok(()));
            let manifest = counted(&attempt, &log, "manifest", Ok(()));
            let directory = counted(&attempt, &log, "directory", Ok(()));
            let lock = counted(&attempt, &log, "lock", Ok(()));
            retire_construction(
                || {},
                || {
                    provider.finish();
                    metadata.finish();
                },
                || {
                    manifest.finish();
                },
                || {
                    directory.finish();
                },
                || log.borrow_mut().push("names"),
                || {},
                || {
                    lock.finish();
                },
            );
            assert_eq!(
                &*log.borrow(),
                &[
                    "provider",
                    "metadata",
                    "manifest",
                    "directory",
                    "names",
                    "lock"
                ]
            );
            assert!(attempt.check());
        }

        #[test]
        fn borrowed_cursor_no_owner() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let mode = Checked { control: &control };
            let log = Rc::new(RefCell::new(Vec::new()));
            let directory = counted(&attempt, &log, "directory", Ok(()));
            let calls = Cell::new(0);
            let mut cursor =
                [Ok("first"), Err(DailyEvidenceError::Io), Ok("unreached")].into_iter();
            {
                let borrowed = directory.get();
                assert_eq!(borrowed.name, "directory");
                let first = effect(&mode, || {
                    calls.set(calls.get() + 1);
                    cursor.next().unwrap()
                });
                assert_eq!(first.ok(), Some("first"));
                let second = effect(&mode, || {
                    calls.set(calls.get() + 1);
                    cursor.next().unwrap()
                });
                assert!(second.is_err());
                assert_eq!(attempt.report().live(), 1);
                assert_eq!(attempt.report().attempted(), 0);
            }
            directory.finish();
            assert_eq!(calls.get(), 2);
            assert_eq!(attempt.report().attempted(), 1);
        }

        #[test]
        fn unit_settlement_keeps_data() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let mode = Checked { control: &control };
            let mut data = None;
            capture_returned(&mut data, || Ok::<_, Body>(Row("kept".into())))
                .ok()
                .unwrap();
            let pointer = data.as_ref().unwrap().0.as_ptr();
            attempt.gap(Gap::TerminalPublication);
            let calls = Cell::new(0);
            assert!(
                effect(&mode, || {
                    calls.set(1);
                    Ok(())
                })
                .is_err()
            );
            assert!(control.local().as_ref().err().unwrap().body().is_none());
            assert_eq!(calls.get(), 0);
            assert_eq!(data.as_ref().unwrap().0.as_ptr(), pointer);
        }

        #[test]
        fn none_vs_empty() {
            let mut rows: Option<Vec<Row>> = None;
            assert!(
                capture_returned(&mut rows, || Err::<Vec<Row>, _>(Body(
                    "parse-failed".into()
                )))
                .is_err()
            );
            assert!(rows.is_none());
            capture_returned(&mut rows, || Ok::<_, Body>(Vec::new()))
                .ok()
                .unwrap();
            assert_eq!(rows.as_ref().map(Vec::len), Some(0));
            let scope = ApprovedRawScope::new("kis", MARKET_KR, 0).unwrap();
            let mut actual = None;
            capture_returned(&mut actual, || parse_manifest(&scope, &limits(), b"")).unwrap();
            assert!(actual.as_ref().unwrap().pending.is_empty());
            let mut rejected = None;
            assert!(
                capture_returned(&mut rejected, || parse_manifest(&scope, &limits(), b"!\n"))
                    .is_err()
            );
            assert!(rejected.is_none());
            let mut read = None;
            assert!(capture_returned(&mut read, || read_exact_captured(&b"x"[..], 2, 2)).is_err());
            assert!(read.is_none());
        }

        #[test]
        fn partial_batch_retention() {
            let mut batches = Vec::new();
            let prior = Row("prior-complete".into());
            let prior_pointer = prior.0.as_ptr();
            append_completed(&mut batches, prior);
            let mut leaves = Vec::new();
            let mut returned = None;
            capture_returned(&mut returned, || Ok::<_, Body>(Row("returned-leaf".into())))
                .ok()
                .unwrap();
            let leaf = returned.take().unwrap();
            let leaf_pointer = leaf.0.as_ptr();
            append_completed(&mut leaves, leaf);
            let result: Result<(), Body> = Err(Body("later-leaf-failed".into()));
            let result = retain_before_cleanup(result, |error| error, || {});
            assert!(result.is_err());
            assert_eq!(batches[0].0.as_ptr(), prior_pointer);
            assert_eq!(leaves[0].0.as_ptr(), leaf_pointer);
            assert!(returned.is_none());
            assert_eq!(batches.len(), 1);
        }

        #[test]
        fn outer_owner_live() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let log = Rc::new(RefCell::new(Vec::new()));
            let outer = counted(&attempt, &log, "outer", Ok(()));
            let snapshot = counted(&attempt, &log, "snapshot", Ok(()));
            snapshot.finish();
            assert!(control.local().is_ok());
            assert_eq!(attempt.report().live(), 1);
            assert_eq!(outer.get().name, "outer");
            assert!(attempt.check());
            assert!(control.before().is_ok());
            outer.finish();
            assert_eq!(&*log.borrow(), &["snapshot", "outer"]);
        }

        #[test]
        fn unwind_abandoned() {
            let attempt = Attempt::default();
            let log = Rc::new(RefCell::new(Vec::new()));
            let result = catch_unwind(AssertUnwindSafe(|| {
                let owner = counted(&attempt, &log, "fallback", Ok(()));
                let _armed = SnapshotUnwind {
                    attempt: &attempt,
                    armed: true,
                };
                let _keep = &owner;
                panic!("ordinary unwind");
            }));
            assert!(result.is_err());
            assert_eq!(attempt.report().first(), Some(Issue::Gap(Gap::Abandoned)));
            assert_eq!(attempt.report().attempted(), 1);
            assert_eq!(&*log.borrow(), &["fallback"]);
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            assert!(control.before().is_err());
        }

        #[test]
        fn refuse_after_dirty() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let mode = Checked { control: &control };
            let log = Rc::new(RefCell::new(Vec::new()));
            let owner = counted(&attempt, &log, "cleanup", Ok(()));
            attempt.gap(Gap::Unbound);
            let cache_hits = Cell::new(0);
            assert!(
                effect(&mode, || {
                    cache_hits.set(1);
                    Ok(())
                })
                .is_err()
            );
            owner.finish();
            assert_eq!(cache_hits.get(), 0);
            assert_eq!(attempt.report().attempted(), 1);
            assert_eq!(attempt.report().closed(), 1);
            let finished = Attempt::default();
            assert!(finished.clone().finish(Ok::<(), Infallible>(())).is_ok());
            let invocation = RawInvocation::new(&finished);
            let control = CheckedControl::new(&invocation, &finished, &finished);
            assert!(control.before().is_err());
        }

        #[test]
        fn redacted_debug() {
            let outcome = OriginalOutcome(Err::<Row, _>(Body("secret-body".into())));
            let display = format!("{outcome:?}");
            assert!(display.contains("ok: false"));
            assert!(!display.contains("secret-body"));
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let control = CheckedControl::new(&invocation, &attempt, &attempt);
            let display = format!("{:?}", control.compare(&attempt, &attempt));
            assert!(!display.contains("Report"));
            assert!(!display.contains("Attempt"));
        }
    }

    #[cfg(test)]
    mod private_tests {
        use super::*;
        use std::cell::Cell;
        use std::fs::{self, OpenOptions};
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::path::{Path, PathBuf};
        use std::rc::Rc;
        use std::time::UNIX_EPOCH;

        use crate::contract::{FetchMode, MARKET_KR, RequestMetadata, ResponseKind};
        use crate::storage::{FileEntry, ManifestEntry};
        use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
        use uuid::Uuid;

        const BODY: &[u8] = b"opaque body bytes";

        struct Fixture {
            _temp: tempfile::TempDir,
            data_root: PathBuf,
            uid: u32,
            entry: ManifestEntry,
        }

        struct CountingReader {
            bytes: Vec<u8>,
            cursor: usize,
            calls: usize,
            returned: usize,
        }

        impl CountingReader {
            fn new(bytes: &[u8]) -> Self {
                Self {
                    bytes: bytes.to_vec(),
                    cursor: 0,
                    calls: 0,
                    returned: 0,
                }
            }
        }

        impl Read for CountingReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.calls += 1;
                let remaining = self
                    .bytes
                    .len()
                    .checked_sub(self.cursor)
                    .expect("test reader cursor stays within its input");
                let count = remaining.min(buffer.len());
                let end = self
                    .cursor
                    .checked_add(count)
                    .expect("test reader count is bounded by its input");
                buffer[..count].copy_from_slice(&self.bytes[self.cursor..end]);
                self.cursor = end;
                self.returned += count;
                Ok(count)
            }
        }

        fn make_fixture() -> Fixture {
            let temp = tempfile::tempdir().unwrap();
            let data_root = temp.path().join("data");
            fs::create_dir(&data_root).unwrap();
            let raw = data_root.join("raw");
            let manifest_dir = raw.join("manifests").join("provider=kis").join("market=kr");
            let entry = ManifestEntry {
                batch_id: BatchId::from_uuid(Uuid::from_u128(1)),
                provider: "kis".to_owned(),
                market: MARKET_KR.to_owned(),
                date: TradingDate::parse("2026-10-13").unwrap(),
                retrieved_at: UtcTimestamp::parse_rfc3339("2026-10-14T01:02:03Z").unwrap(),
                mode: FetchMode::Synthetic,
                entitlement_reference: None,
                files: vec![FileEntry {
                    kind: ResponseKind::Bars,
                    file_name: "bars.json".to_owned(),
                    content_hash: ContentHash::from_bytes(BODY),
                    size_bytes: u64::try_from(BODY.len()).unwrap(),
                    request: RequestMetadata {
                        endpoint: "kis.fixture".to_owned(),
                        query: vec![("date".to_owned(), "2026-10-13".to_owned())],
                        headers: Vec::new(),
                        mode: FetchMode::Synthetic,
                    },
                    response_continuation: None,
                    copied_from: None,
                }],
            };
            let batch_dir = batch_path(&data_root, &entry);
            private_tree_dir(&data_root, &manifest_dir);
            private_tree_dir(&data_root, &batch_dir);
            private_file(&manifest_dir.join("commit.lock"), b"");
            private_file(
                &manifest_dir.join("manifest.jsonl"),
                &[serde_json::to_vec(&entry).unwrap().as_slice(), b"\n"].concat(),
            );
            private_file(
                &batch_dir.join("batch.json"),
                &serde_json::to_vec_pretty(&entry).unwrap(),
            );
            private_file(&batch_dir.join("bars.json"), BODY);
            let uid = fs::metadata(&raw).unwrap().uid();
            Fixture {
                _temp: temp,
                data_root,
                uid,
                entry,
            }
        }

        fn private_dir(path: &Path) {
            fs::create_dir_all(path).unwrap();
            set_mode(path, 0o700);
        }

        fn private_tree_dir(root: &Path, path: &Path) {
            let relative = path.strip_prefix(root).unwrap();
            let mut current = root.to_path_buf();
            for component in relative.components() {
                let std::path::Component::Normal(name) = component else {
                    panic!("Raw fixture path must stay below its data root");
                };
                current.push(name);
                match fs::create_dir(&current) {
                    Ok(()) => set_mode(&current, 0o700),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        assert!(fs::symlink_metadata(&current).unwrap().is_dir());
                    }
                    Err(error) => panic!("failed to create Raw fixture directory: {error}"),
                }
            }
        }

        fn private_file(path: &Path, bytes: &[u8]) {
            fs::write(path, bytes).unwrap();
            set_mode(path, 0o600);
        }

        fn set_mode(path: &Path, mode: u32) {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }

        fn raw_path(fixture: &Fixture) -> PathBuf {
            fixture.data_root.join("raw")
        }

        fn manifest_dir(fixture: &Fixture) -> PathBuf {
            raw_path(fixture)
                .join("manifests")
                .join("provider=kis")
                .join("market=kr")
        }

        fn manifest_path(fixture: &Fixture) -> PathBuf {
            manifest_dir(fixture).join("manifest.jsonl")
        }

        fn lock_path(fixture: &Fixture) -> PathBuf {
            manifest_dir(fixture).join("commit.lock")
        }

        fn batch_path(data_root: &Path, entry: &ManifestEntry) -> PathBuf {
            data_root
                .join("raw")
                .join("provider=kis")
                .join("market=kr")
                .join(format!("date={}", entry.date))
                .join(format!("batch={}", entry.batch_id))
        }

        fn batch_dir(fixture: &Fixture) -> PathBuf {
            batch_path(&fixture.data_root, &fixture.entry)
        }

        fn metadata_path(fixture: &Fixture) -> PathBuf {
            batch_dir(fixture).join("batch.json")
        }

        fn body_path(fixture: &Fixture) -> PathBuf {
            batch_dir(fixture).join("bars.json")
        }

        fn with_incomplete_over_limit_file(entry: &ManifestEntry) -> Vec<u8> {
            let bytes = serde_json::to_vec(entry).unwrap();
            let files_end = bytes
                .iter()
                .rposition(|byte| *byte == b']')
                .expect("serialized entry has a files array");
            let mut incomplete = Vec::new();
            incomplete.extend_from_slice(&bytes[..files_end]);
            incomplete.extend_from_slice(b",{\"sentinel\":");
            incomplete
        }

        fn with_malformed_over_limit_file(entry: &ManifestEntry) -> Vec<u8> {
            let bytes = serde_json::to_vec(entry).unwrap();
            let files_end = bytes.iter().rposition(|byte| *byte == b']').unwrap();
            let mut malformed = with_incomplete_over_limit_file(entry);
            malformed.extend_from_slice(&bytes[files_end..]);
            malformed
        }

        fn scan(fixture: &Fixture) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
            scan_at(fixture, &fixture.data_root)
        }

        fn scan_at(
            fixture: &Fixture,
            data_root: &Path,
        ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
            scan_with_limits(fixture, data_root, test_limits())
        }

        fn scan_with_limits(
            fixture: &Fixture,
            data_root: &Path,
            limits: ScanLimits,
        ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
            let scope = ApprovedRawScope::new("kis", "kr", fixture.uid)?;
            CommittedRawReader::new(data_root).snapshot(scope, limits)
        }

        fn test_limits() -> ScanLimits {
            ScanLimits::new(ScanLimitValues {
                max_manifest_bytes: 16_384,
                max_manifest_records: 8,
                max_batches: 4,
                max_files_per_batch: 4,
                max_files: 8,
                max_batch_metadata_bytes: 8_192,
                max_source_file_bytes: 8_192,
                max_retained_bytes: 32_768,
                max_open_descriptors: 128,
                max_data_root_bytes: 4_096,
                max_data_root_components: 32,
                max_metadata_json_depth: 32,
            })
            .unwrap()
        }

        fn replace_file_with_same_bytes(path: &Path) {
            let bytes = fs::read(path).unwrap();
            let saved = path.with_extension("saved");
            fs::rename(path, saved).unwrap();
            private_file(path, &bytes);
        }

        fn replace_file_with_same_bytes_outside(path: &Path, saved: &Path) {
            let bytes = fs::read(path).unwrap();
            fs::rename(path, saved).unwrap();
            private_file(path, &bytes);
        }

        fn replace_directory_with_empty(path: &Path) {
            let saved = path.with_extension("saved");
            fs::rename(path, saved).unwrap();
            private_dir(path);
        }

        fn mutate_in_place(path: &Path) {
            let mut bytes = fs::read(path).unwrap();
            bytes[0] ^= 1;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&bytes).unwrap();
            file.sync_all().unwrap();
        }

        fn expect_replacement_error(
            fixture: &Fixture,
            phase: CapturePhase,
            mutation: impl FnMut(CapturePhase) -> Result<(), DailyEvidenceError> + 'static,
            invoked: Rc<Cell<bool>>,
        ) {
            assert!(
                scan(fixture).is_ok(),
                "fixture must be valid before mutation"
            );
            let result = with_phase_hook(mutation, || scan(fixture));
            assert!(
                invoked.get(),
                "the deterministic capture phase was not reached"
            );
            assert_eq!(
                result.err(),
                Some(DailyEvidenceError::SourceChanged),
                "{phase:?}"
            );
        }

        fn mutation_at(
            phase: CapturePhase,
            mutation: impl FnOnce() + 'static,
            invoked: Rc<Cell<bool>>,
        ) -> impl FnMut(CapturePhase) -> Result<(), DailyEvidenceError> {
            let mut mutation = Some(mutation);
            move |current| {
                if current == phase {
                    mutation.take().expect("phase hook invoked more than once")();
                    invoked.set(true);
                }
                Ok(())
            }
        }

        #[derive(Debug, PartialEq, Eq)]
        struct TreeState {
            path: PathBuf,
            device: u64,
            inode: u64,
            mode: u32,
            uid: u32,
            links: u64,
            bytes: Option<Vec<u8>>,
        }

        fn tree_state(root: &Path) -> Vec<TreeState> {
            fn visit(root: &Path, current: &Path, found: &mut Vec<TreeState>) {
                let metadata = fs::symlink_metadata(current).unwrap();
                let bytes = if metadata.is_file() {
                    Some(fs::read(current).unwrap())
                } else if metadata.file_type().is_symlink() {
                    Some(
                        fs::read_link(current)
                            .unwrap()
                            .as_os_str()
                            .as_bytes()
                            .to_vec(),
                    )
                } else {
                    None
                };
                found.push(TreeState {
                    path: current.strip_prefix(root).unwrap().to_path_buf(),
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    mode: metadata.mode(),
                    uid: metadata.uid(),
                    links: metadata.nlink(),
                    bytes,
                });
                if metadata.is_dir() {
                    let mut children: Vec<_> = fs::read_dir(current)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .collect();
                    children.sort();
                    for child in children {
                        visit(root, &child, found);
                    }
                }
            }

            let mut state = Vec::new();
            visit(root, root, &mut state);
            state.sort_by(|left, right| left.path.cmp(&right.path));
            state
        }

        fn observed_descriptors(
            observation: &Arc<Mutex<ResourceObservation>>,
        ) -> Vec<ObservedDescriptor> {
            observation
                .lock()
                .unwrap()
                .events
                .iter()
                .filter_map(|event| match event {
                    ObservationEvent::DescriptorOpened(descriptor) => Some(*descriptor),
                    _ => None,
                })
                .collect()
        }

        fn native_descriptor_matches(descriptor: ObservedDescriptor, identity: (u64, u64)) -> bool {
            let path = format!("/proc/self/fd/{}", descriptor.raw_fd);
            fs::metadata(path).is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == identity)
        }

        fn descriptor_is_live(
            observation: &Arc<Mutex<ResourceObservation>>,
            descriptor: ObservedDescriptor,
        ) -> bool {
            let observation = observation.lock().unwrap();
            let lifetime = &observation.lifetimes[descriptor.open_id];
            assert_eq!(lifetime.descriptor, descriptor);
            assert!(
                !lifetime.closed,
                "native live checks require the owning handle"
            );
            native_descriptor_matches(
                descriptor,
                lifetime.identity.expect("live object was statted"),
            )
        }

        fn assert_descriptors_closed(
            observation: &Arc<Mutex<ResourceObservation>>,
            descriptors: &[ObservedDescriptor],
            fixture_root: &Path,
        ) {
            let fixture_objects = tree_state(fixture_root);
            let observation = observation.lock().unwrap();
            for descriptor in descriptors {
                let lifetime = &observation.lifetimes[descriptor.open_id];
                assert_eq!(lifetime.descriptor, *descriptor);
                assert!(
                    lifetime.closed,
                    "the native owning File must already be destroyed"
                );
                assert_eq!(
                    observation
                        .events
                        .iter()
                        .filter(|event| {
                            **event == ObservationEvent::DescriptorClosed(descriptor.open_id)
                        })
                        .count(),
                    1,
                    "every open lifetime must close exactly once"
                );
                // Only this test can reopen its unique temporary tree. A
                // shared ancestor's freed FD slot can belong to another test.
                let unique_identity = lifetime.identity.filter(|identity| {
                    fixture_objects
                        .iter()
                        .any(|object| (object.device, object.inode) == *identity)
                });
                if let Some(identity) = unique_identity {
                    assert!(!native_descriptor_matches(*descriptor, identity));
                }
            }
        }

        fn assert_lock_closed_last(observation: &Arc<Mutex<ResourceObservation>>) {
            let observation = observation.lock().unwrap();
            let lock_ids: Vec<_> = observation
                .lifetimes
                .iter()
                .filter(|lifetime| lifetime.descriptor.role == DescriptorRole::Lock)
                .map(|lifetime| lifetime.descriptor.open_id)
                .collect();
            assert_eq!(lock_ids.len(), 1);
            let last_close = observation
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    ObservationEvent::DescriptorClosed(open_id) => Some(*open_id),
                    _ => None,
                });
            assert_eq!(last_close, Some(lock_ids[0]));
        }

        fn assert_no_batch_tree_or_leaf_open(observation: &Arc<Mutex<ResourceObservation>>) {
            assert!(
                !observation
                    .lock()
                    .unwrap()
                    .events
                    .contains(&ObservationEvent::BatchTreeOpen)
            );
            assert!(
                observed_descriptors(observation)
                    .iter()
                    .all(|descriptor| !matches!(
                        descriptor.role,
                        DescriptorRole::Metadata | DescriptorRole::Provider
                    ))
            );
        }

        fn lock_exclusive(fixture: &Fixture) {
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path(fixture))
                .unwrap();
            FileExt::try_lock_exclusive(&lock).unwrap();
            FileExt::unlock(&lock).unwrap();
        }

        #[test]
        fn s2_t28_shared_inode_owners_keep_distinct_lifetimes_across_observation_scope() {
            let (second, third, observation) = with_resource_observation(|observation| {
                let first = OwnedSourceFile::new(File::open("/").unwrap(), DescriptorRole::Root);
                first.observe_identity(&fstat(first.file()).unwrap());
                let second = OwnedSourceFile::new(File::open("/").unwrap(), DescriptorRole::Root);
                second.observe_identity(&fstat(second.file()).unwrap());
                let initial = observed_descriptors(&observation);
                assert_eq!(initial.len(), 2);
                assert_ne!(initial[0].open_id, initial[1].open_id);
                let (first_identity, second_identity) = {
                    let observation = observation.lock().unwrap();
                    (
                        observation.lifetimes[0].identity,
                        observation.lifetimes[1].identity,
                    )
                };
                assert_eq!(first_identity, second_identity);
                assert!(
                    initial
                        .iter()
                        .all(|descriptor| descriptor_is_live(&observation, *descriptor))
                );

                drop(first);
                assert!(observation.lock().unwrap().lifetimes[initial[0].open_id].closed);
                assert!(descriptor_is_live(&observation, initial[1]));
                let third = OwnedSourceFile::new(File::open("/").unwrap(), DescriptorRole::Root);
                third.observe_identity(&fstat(third.file()).unwrap());
                let opened = observed_descriptors(&observation);
                assert_eq!(opened.len(), 3);
                assert_ne!(opened[2].open_id, initial[0].open_id);
                assert_ne!(opened[2].open_id, initial[1].open_id);
                let (opened_identity, initial_identity, initial_closed) = {
                    let observation = observation.lock().unwrap();
                    (
                        observation.lifetimes[opened[2].open_id].identity,
                        observation.lifetimes[initial[0].open_id].identity,
                        observation.lifetimes[initial[0].open_id].closed,
                    )
                };
                assert_eq!(opened_identity, initial_identity);
                assert!(initial_closed);
                assert!(descriptor_is_live(&observation, initial[1]));
                assert!(descriptor_is_live(&observation, opened[2]));
                (second, third, observation)
            });

            // Both real owners moved beyond the thread-local collection scope;
            // their captured observer still records destruction of each File.
            let opened = observed_descriptors(&observation);
            drop(second);
            assert!(observation.lock().unwrap().lifetimes[opened[1].open_id].closed);
            assert!(descriptor_is_live(&observation, opened[2]));
            drop(third);
            let observation = observation.lock().unwrap();
            assert!(observation.lifetimes.iter().all(|lifetime| lifetime.closed));
            for descriptor in &opened {
                assert_eq!(
                    observation
                        .events
                        .iter()
                        .filter(|event| {
                            **event == ObservationEvent::DescriptorClosed(descriptor.open_id)
                        })
                        .count(),
                    1
                );
            }
            // Never query a closed shared ancestor's old FD slot as ownership.
        }

        #[test]
        fn s2_t14_synthetic_raw_stat_owner_mode_and_link_guards_include_uid_zero() {
            assert_eq!(validate_raw_stat_owner_mode(0o700, 0, Some(0)), Ok(()));
            assert_eq!(
                validate_raw_stat_owner_mode(0o700, 1000, Some(0)),
                Err(DailyEvidenceError::UntrustedOwner)
            );
            assert_eq!(
                validate_raw_stat_owner_mode(0o700, 0, Some(4242)),
                Err(DailyEvidenceError::UntrustedOwner)
            );
            assert_eq!(
                validate_raw_stat_owner_mode(0o660, 4242, Some(4242)),
                Err(DailyEvidenceError::WritableNode)
            );
            assert_eq!(validate_raw_stat_owner_mode(0o777, 0, None), Ok(()));
            assert_eq!(validate_single_link_count(1), Ok(()));
            assert_eq!(
                validate_single_link_count(2),
                Err(DailyEvidenceError::HardLinkedFile)
            );
        }

        #[test]
        fn s2_t06_stat_size_conversion_uses_size_mismatch_for_nonrepresentable_values() {
            assert_eq!(
                checked_stat_size_value(-1),
                Err(DailyEvidenceError::SizeMismatch)
            );
            #[cfg(target_pointer_width = "32")]
            assert_eq!(
                checked_stat_size_value(i64::MAX),
                Err(DailyEvidenceError::SizeMismatch)
            );
            #[cfg(target_pointer_width = "64")]
            assert_eq!(
                checked_stat_size_value(i64::MAX),
                usize::try_from(i64::MAX).map_err(|_| DailyEvidenceError::SizeMismatch)
            );
        }

        #[test]
        fn s2_t07_bounded_reads_probe_once_and_reject_caps_before_allocation() {
            let mut exact_reader = CountingReader::new(b"abc");
            let (exact, exact_returned, exact_calls, exact_events) =
                with_resource_observation(|observation| {
                    let result = read_exact_captured(&mut exact_reader, 3, 3);
                    (
                        result,
                        exact_reader.returned,
                        exact_reader.calls,
                        observation.lock().unwrap().events.clone(),
                    )
                });
            assert_eq!(exact, Ok(b"abc".to_vec()));
            assert_eq!(exact_returned, 3);
            assert_eq!(exact_calls, 2);
            assert_eq!(exact_events.len(), 3);
            assert!(matches!(
                exact_events[0],
                ObservationEvent::EvidenceBufferReady {
                    requested: 3,
                    capacity
                } if capacity >= 3
            ));
            assert_eq!(exact_events[1], ObservationEvent::ReadBytes(3));
            assert_eq!(exact_events[2], ObservationEvent::ReadBytes(0));

            let mut short_reader = CountingReader::new(b"ab");
            assert_eq!(
                read_exact_captured(&mut short_reader, 3, 3),
                Err(DailyEvidenceError::SizeMismatch)
            );
            assert_eq!(short_reader.calls, 2);
            assert_eq!(short_reader.returned, 2);

            let mut growing_reader = CountingReader::new(b"abcdef");
            let (growing, returned, calls) = with_resource_observation(|_| {
                let result = read_exact_captured(&mut growing_reader, 3, 8);
                (result, growing_reader.returned, growing_reader.calls)
            });
            assert_eq!(growing, Err(DailyEvidenceError::SourceChanged));
            assert_eq!(returned, 4);
            assert_eq!(calls, 2);

            let mut capped_reader = CountingReader::new(b"abcd");
            assert_eq!(
                read_exact_captured(&mut capped_reader, 3, 3),
                Err(DailyEvidenceError::SourceScanLimit)
            );
            assert_eq!(capped_reader.calls, 2);
            assert_eq!(capped_reader.returned, 4);

            let mut rejected_reader = CountingReader::new(b"not read");
            let (rejected, events) = with_resource_observation(|observation| {
                let result = read_exact_captured(&mut rejected_reader, 4, 3);
                (result, observation.lock().unwrap().events.clone())
            });
            assert_eq!(rejected, Err(DailyEvidenceError::SourceScanLimit));
            assert_eq!(rejected_reader.calls, 0);
            assert_eq!(rejected_reader.returned, 0);
            assert!(events.is_empty());
        }

        #[test]
        fn s2_t08_file_seed_rejects_before_materialization_and_data_tree_open() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before count fault"
            );
            let mut bounded_values = test_limits().values();
            bounded_values.max_files_per_batch = fixture.entry.files.len();
            assert!(
                scan_with_limits(
                    &fixture,
                    &fixture.data_root,
                    ScanLimits::new(bounded_values).unwrap()
                )
                .is_ok()
            );

            let mut full_entry = serde_json::to_value(&fixture.entry).unwrap();
            full_entry["files"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"sentinel": "not a FileEntry"}));
            let over_limit = serde_json::to_vec(&full_entry).unwrap();
            let malformed = with_malformed_over_limit_file(&fixture.entry);
            let incomplete = with_incomplete_over_limit_file(&fixture.entry);
            assert!(incomplete.ends_with(b",{\"sentinel\":"));
            for (bytes, expect_eof) in [(&malformed, false), (&incomplete, true)] {
                match serde_json::from_slice::<ManifestEntry>(bytes) {
                    Err(error) => assert_eq!(error.is_eof(), expect_eof),
                    Ok(_) => panic!("the second file entry must be incomplete or malformed"),
                }
                match parse_bounded_entry(bytes, test_limits().values.max_files_per_batch) {
                    Err(BoundedParseError::Json(error)) => assert_eq!(error.is_eof(), expect_eof),
                    _ => panic!("the permissive parser must report the actual JSON class"),
                }
            }

            for faulty_row in [&over_limit, &malformed, &incomplete] {
                private_file(
                    &manifest_path(&fixture),
                    &[
                        serde_json::to_vec(&fixture.entry).unwrap().as_slice(),
                        b"\n",
                    ]
                    .concat(),
                );
                assert!(
                    scan_with_limits(
                        &fixture,
                        &fixture.data_root,
                        ScanLimits::new(bounded_values).unwrap()
                    )
                    .is_ok()
                );
                private_file(&manifest_path(&fixture), faulty_row);
                let (result, observation) = with_resource_observation(|observation| {
                    (
                        scan_with_limits(
                            &fixture,
                            &fixture.data_root,
                            ScanLimits::new(bounded_values).unwrap(),
                        ),
                        observation,
                    )
                });
                assert_eq!(result.err(), Some(DailyEvidenceError::SourceScanLimit));
                assert_eq!(
                    observation
                        .lock()
                        .unwrap()
                        .events
                        .iter()
                        .filter(|event| **event == ObservationEvent::FileEntryMaterialized)
                        .count(),
                    1
                );
                assert_no_batch_tree_or_leaf_open(&observation);
                assert_descriptors_closed(
                    &observation,
                    &observed_descriptors(&observation),
                    fixture._temp.path(),
                );
                assert_lock_closed_last(&observation);
            }
        }

        #[test]
        fn s2_t08_total_file_cap_rejects_before_any_batch_tree_or_leaf_open() {
            let mut fixture = make_fixture();
            let mut second_file = fixture.entry.files[0].clone();
            second_file.file_name = "second.json".to_owned();
            second_file.content_hash = ContentHash::from_bytes(b"second opaque body");
            second_file.size_bytes = u64::try_from(b"second opaque body".len()).unwrap();
            fixture.entry.files.push(second_file);
            private_file(
                &batch_dir(&fixture).join("second.json"),
                b"second opaque body",
            );
            private_file(
                &metadata_path(&fixture),
                &serde_json::to_vec_pretty(&fixture.entry).unwrap(),
            );
            private_file(
                &manifest_path(&fixture),
                &[
                    serde_json::to_vec(&fixture.entry).unwrap().as_slice(),
                    b"\n",
                ]
                .concat(),
            );
            let mut values = test_limits().values();
            values.max_files = 2;
            assert!(
                scan_with_limits(
                    &fixture,
                    &fixture.data_root,
                    ScanLimits::new(values).unwrap()
                )
                .is_ok()
            );
            values.max_files = 1;
            let (result, observation) = with_resource_observation(|observation| {
                (
                    scan_with_limits(
                        &fixture,
                        &fixture.data_root,
                        ScanLimits::new(values).unwrap(),
                    ),
                    observation,
                )
            });
            assert_eq!(result.err(), Some(DailyEvidenceError::SourceScanLimit));
            assert_no_batch_tree_or_leaf_open(&observation);
            assert_descriptors_closed(
                &observation,
                &observed_descriptors(&observation),
                fixture._temp.path(),
            );
            assert_lock_closed_last(&observation);
        }

        #[test]
        fn s2_t08_unique_batch_cap_rejects_before_any_batch_tree_or_leaf_open() {
            let fixture = make_fixture();
            let mut second = fixture.entry.clone();
            second.batch_id = BatchId::from_uuid(Uuid::from_u128(2));
            let second_directory = batch_path(&fixture.data_root, &second);
            private_tree_dir(&fixture.data_root, &second_directory);
            private_file(
                &second_directory.join("batch.json"),
                &serde_json::to_vec_pretty(&second).unwrap(),
            );
            private_file(&second_directory.join("bars.json"), BODY);
            private_file(
                &manifest_path(&fixture),
                &[
                    serde_json::to_vec(&fixture.entry).unwrap().as_slice(),
                    b"\n",
                    serde_json::to_vec(&second).unwrap().as_slice(),
                    b"\n",
                ]
                .concat(),
            );
            let mut values = test_limits().values();
            values.max_batches = 2;
            assert!(
                scan_with_limits(
                    &fixture,
                    &fixture.data_root,
                    ScanLimits::new(values).unwrap()
                )
                .is_ok()
            );
            values.max_batches = 1;
            let (result, observation) = with_resource_observation(|observation| {
                (
                    scan_with_limits(
                        &fixture,
                        &fixture.data_root,
                        ScanLimits::new(values).unwrap(),
                    ),
                    observation,
                )
            });
            assert_eq!(result.err(), Some(DailyEvidenceError::SourceScanLimit));
            assert_no_batch_tree_or_leaf_open(&observation);
            assert_descriptors_closed(
                &observation,
                &observed_descriptors(&observation),
                fixture._temp.path(),
            );
            assert_lock_closed_last(&observation);
        }

        #[test]
        fn s2_t03_bounded_entry_sequence_schema_matches_legacy_serde() {
            let fixture = make_fixture();
            let object = serde_json::to_value(&fixture.entry).unwrap();
            let fields = [
                "batch_id",
                "provider",
                "market",
                "date",
                "retrieved_at",
                "mode",
                "entitlement_reference",
                "files",
            ];
            let sequence = serde_json::Value::Array(
                fields.iter().map(|field| object[*field].clone()).collect(),
            );
            let bytes = serde_json::to_vec(&sequence).unwrap();
            let legacy: ManifestEntry = serde_json::from_slice(&bytes).unwrap();
            let bounded = parse_bounded_entry(&bytes, test_limits().values.max_files_per_batch)
                .map_err(|_| DailyEvidenceError::InvalidManifest)
                .unwrap();
            assert_eq!(bounded, legacy);
        }

        #[test]
        fn s2_t04_identical_rows_retain_one_batch_arc_and_one_evidence_charge() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before duplicate rows"
            );
            let row = serde_json::to_vec(&fixture.entry).unwrap();
            private_file(
                &manifest_path(&fixture),
                &[row.as_slice(), b"\n", row.as_slice(), b"\n"].concat(),
            );
            let snapshot = scan(&fixture).unwrap();
            assert_eq!(snapshot.stats.manifest_records, 2);
            assert_eq!(snapshot.batches.len(), 1);
            assert_eq!(std::sync::Arc::strong_count(&snapshot.manifest_bytes), 2);
            let retained = snapshot.manifest_bytes.len()
                + snapshot.batches[0].batch_json.bytes.len()
                + snapshot.batches[0].files[0].bytes.len();
            assert_eq!(snapshot.stats.retained_evidence_bytes, retained);
        }

        #[test]
        fn s2_t09_t10_owned_resource_observations_match_the_capture() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before observation"
            );
            let (snapshot, observation) =
                with_resource_observation(|observation| (scan(&fixture).unwrap(), observation));
            let events = observation.lock().unwrap().events.clone();
            let descriptors = observed_descriptors(&observation);
            assert_eq!(descriptors.len(), snapshot.stats.open_descriptors);
            assert_eq!(
                snapshot.stats.open_descriptors,
                snapshot.stats.peak_open_descriptors
            );
            assert!(
                descriptors
                    .iter()
                    .all(|descriptor| descriptor_is_live(&observation, *descriptor))
            );
            for (index, descriptor) in descriptors.iter().enumerate() {
                assert!(
                    !descriptors[..index]
                        .iter()
                        .any(|earlier| earlier.raw_fd == descriptor.raw_fd)
                );
                assert!(
                    !descriptors[..index]
                        .iter()
                        .any(|earlier| earlier.open_id == descriptor.open_id)
                );
            }
            let reservations = events
                .iter()
                .filter(|event| **event == ObservationEvent::DescriptorReserved)
                .count();
            let attempts = events
                .iter()
                .filter(|event| **event == ObservationEvent::DescriptorOpenAttempt)
                .count();
            assert_eq!(reservations, descriptors.len());
            assert_eq!(attempts, descriptors.len());
            for (index, event) in events.iter().enumerate() {
                if *event == ObservationEvent::DescriptorOpenAttempt {
                    assert_eq!(events[index - 1], ObservationEvent::DescriptorReserved);
                    assert!(matches!(
                        events.get(index + 1),
                        Some(ObservationEvent::DescriptorOpened(_))
                    ));
                }
            }
            let mut actual_retained = snapshot.manifest_bytes().len();
            for batch in snapshot.batches() {
                actual_retained += batch.batch_json_bytes().len();
                for file in batch.files() {
                    actual_retained += file.bytes().len();
                }
            }
            assert_eq!(actual_retained, snapshot.stats.retained_evidence_bytes);
            let mut reserved_total = 0usize;
            let mut pending_allocation = None;
            for event in &events {
                match event {
                    ObservationEvent::EvidenceBytesReserved { amount, total } => {
                        reserved_total = reserved_total.checked_add(*amount).unwrap();
                        assert_eq!(*total, reserved_total);
                        pending_allocation = Some(*amount);
                    }
                    ObservationEvent::EvidenceBufferReady {
                        requested,
                        capacity,
                    } => {
                        assert!(*capacity >= *requested);
                        assert_eq!(pending_allocation.take(), Some(*requested));
                    }
                    _ => {}
                }
            }
            assert_eq!(pending_allocation, None);
            assert_eq!(reserved_total, actual_retained);
            for (opened_at, event) in events.iter().enumerate() {
                if !matches!(
                    event,
                    ObservationEvent::DescriptorOpened(descriptor)
                        if descriptor.role == DescriptorRole::Provider
                ) {
                    continue;
                }
                let attempt_at = events[..opened_at]
                    .iter()
                    .rposition(|event| *event == ObservationEvent::DescriptorOpenAttempt)
                    .unwrap();
                let reserve_at = events[..attempt_at]
                    .iter()
                    .rposition(|event| {
                        matches!(event, ObservationEvent::EvidenceBytesReserved { .. })
                    })
                    .unwrap();
                assert!(
                    !events[reserve_at + 1..attempt_at]
                        .iter()
                        .any(|event| matches!(event, ObservationEvent::EvidenceBufferReady { .. }))
                );
            }
            drop(snapshot);
            assert_descriptors_closed(&observation, &descriptors, fixture._temp.path());
            assert_lock_closed_last(&observation);
            lock_exclusive(&fixture);

            let mut values = test_limits().values();
            values.max_retained_bytes = actual_retained.checked_sub(1).unwrap();
            let retained_one_under = ScanLimits::new(values).unwrap();
            let (retained_result, retained_observation) =
                with_resource_observation(|observation| {
                    (
                        scan_with_limits(&fixture, &fixture.data_root, retained_one_under),
                        observation,
                    )
                });
            assert_eq!(
                retained_result.err(),
                Some(DailyEvidenceError::SourceScanLimit)
            );
            let retained_events = retained_observation.lock().unwrap().events.clone();
            let retained_descriptors = observed_descriptors(&retained_observation);
            assert!(
                retained_descriptors
                    .iter()
                    .all(|descriptor| descriptor.role != DescriptorRole::Provider)
            );
            let buffer_count = retained_events
                .iter()
                .filter(|event| matches!(event, ObservationEvent::EvidenceBufferReady { .. }))
                .count();
            assert_eq!(
                buffer_count, 2,
                "only manifest and batch metadata are captured"
            );
            let retained_high_water = retained_events
                .iter()
                .filter_map(|event| match event {
                    ObservationEvent::EvidenceBytesReserved { total, .. } => Some(*total),
                    _ => None,
                })
                .next_back()
                .unwrap();
            assert_eq!(
                retained_high_water,
                actual_retained.checked_sub(BODY.len()).unwrap()
            );
            assert_descriptors_closed(
                &retained_observation,
                &retained_descriptors,
                fixture._temp.path(),
            );
            assert_lock_closed_last(&retained_observation);
            lock_exclusive(&fixture);

            let required = scan(&fixture).unwrap().stats.open_descriptors;
            let mut values = test_limits().values();
            values.max_open_descriptors = required - 1;
            let one_under = ScanLimits::new(values).unwrap();
            let (result, under_observation) = with_resource_observation(|observation| {
                (
                    scan_with_limits(&fixture, &fixture.data_root, one_under),
                    observation,
                )
            });
            assert_eq!(result.err(), Some(DailyEvidenceError::SourceScanLimit));
            let under_events = under_observation.lock().unwrap().events.clone();
            assert!(!under_events.contains(&ObservationEvent::BatchTreeOpen));
            let under_descriptors = observed_descriptors(&under_observation);
            assert!(under_descriptors.iter().all(|descriptor| !matches!(
                descriptor.role,
                DescriptorRole::Metadata | DescriptorRole::Provider
            )));
            let data_root_components = fixture
                .data_root
                .components()
                .filter(|component| matches!(component, std::path::Component::Normal(_)))
                .count();
            assert_eq!(under_descriptors.len(), data_root_components + 7);
            assert_descriptors_closed(&under_observation, &under_descriptors, fixture._temp.path());
            assert_lock_closed_last(&under_observation);
            lock_exclusive(&fixture);
        }

        #[test]
        fn s2_t13_parent_and_relative_roots_reject_before_component_copy_or_open() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before root fault"
            );
            let invalid_root = fixture.data_root.join("..").join("normalized-away");
            let (result, observation) = with_resource_observation(|observation| {
                (scan_at(&fixture, &invalid_root), observation)
            });
            assert_eq!(result.err(), Some(DailyEvidenceError::UnsafePath));
            assert!(
                !observation
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .any(|event| matches!(
                        event,
                        ObservationEvent::RootComponentCopied
                            | ObservationEvent::RootComponentsReserve(_)
                            | ObservationEvent::RootComponentReserve(_)
                            | ObservationEvent::NamePoolReserve(_)
                            | ObservationEvent::NameCopied(_)
                            | ObservationEvent::DescriptorReserved
                            | ObservationEvent::DescriptorOpenAttempt
                            | ObservationEvent::DescriptorOpened(_)
                    ))
            );

            let (relative_result, relative_observation) =
                with_resource_observation(|observation| {
                    (scan_at(&fixture, Path::new("relative/raw")), observation)
                });
            assert_eq!(relative_result.err(), Some(DailyEvidenceError::UnsafePath));
            assert!(relative_observation.lock().unwrap().events.is_empty());
        }

        #[test]
        fn r15_recorded_body_size_precedes_data_tree_and_leaf_io() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before size faults"
            );

            let mut zero_size = fixture.entry.clone();
            zero_size.files[0].size_bytes = 0;
            private_file(
                &manifest_path(&fixture),
                &[serde_json::to_vec(&zero_size).unwrap().as_slice(), b"\n"].concat(),
            );
            let zero_capture_phase_seen = Rc::new(Cell::new(false));
            let zero_capture_seen_by_hook = Rc::clone(&zero_capture_phase_seen);
            let (zero_result, zero_observation) = with_resource_observation(|observation| {
                let result = with_phase_hook(
                    move |phase| {
                        if matches!(
                            phase,
                            CapturePhase::BeforeBatchTreeOpen | CapturePhase::AfterMetadataCapture
                        ) {
                            zero_capture_seen_by_hook.set(true);
                        }
                        Ok(())
                    },
                    || scan(&fixture),
                );
                (result, observation)
            });
            assert_eq!(zero_result.err(), Some(DailyEvidenceError::SizeMismatch));
            assert!(!zero_capture_phase_seen.get());
            let zero_events = zero_observation.lock().unwrap().events.clone();
            assert!(!zero_events.contains(&ObservationEvent::BatchTreeOpen));
            assert!(
                observed_descriptors(&zero_observation)
                    .iter()
                    .all(|descriptor| {
                        !matches!(
                            descriptor.role,
                            DescriptorRole::Metadata | DescriptorRole::Provider
                        )
                    })
            );

            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before size cap fault"
            );
            let mut over_cap = fixture.entry.clone();
            over_cap.files[0].size_bytes = 9;
            private_file(
                &manifest_path(&fixture),
                &[serde_json::to_vec(&over_cap).unwrap().as_slice(), b"\n"].concat(),
            );
            let mut values = test_limits().values();
            values.max_source_file_bytes = 8;
            let limits = ScanLimits::new(values).unwrap();
            let cap_capture_phase_seen = Rc::new(Cell::new(false));
            let cap_capture_seen_by_hook = Rc::clone(&cap_capture_phase_seen);
            let (cap_result, cap_observation) = with_resource_observation(|observation| {
                let result = with_phase_hook(
                    move |phase| {
                        if matches!(
                            phase,
                            CapturePhase::BeforeBatchTreeOpen | CapturePhase::AfterMetadataCapture
                        ) {
                            cap_capture_seen_by_hook.set(true);
                        }
                        Ok(())
                    },
                    || scan_with_limits(&fixture, &fixture.data_root, limits),
                );
                (result, observation)
            });
            assert_eq!(cap_result.err(), Some(DailyEvidenceError::SourceScanLimit));
            assert!(!cap_capture_phase_seen.get());
            let cap_events = cap_observation.lock().unwrap().events.clone();
            assert!(!cap_events.contains(&ObservationEvent::BatchTreeOpen));
            assert!(
                observed_descriptors(&cap_observation)
                    .iter()
                    .all(|descriptor| {
                        !matches!(
                            descriptor.role,
                            DescriptorRole::Metadata | DescriptorRole::Provider
                        )
                    })
            );
        }

        #[test]
        fn s2_t23_lock_path_replacement_before_and_after_shared_acquisition_fails() {
            for phase in [CapturePhase::AfterLockOpen, CapturePhase::AfterSharedLock] {
                let fixture = make_fixture();
                let path = lock_path(&fixture);
                let invoked = Rc::new(Cell::new(false));
                let hook_invoked = Rc::clone(&invoked);
                expect_replacement_error(
                    &fixture,
                    phase,
                    mutation_at(
                        phase,
                        move || replace_file_with_same_bytes(&path),
                        Rc::clone(&hook_invoked),
                    ),
                    invoked,
                );
            }
        }

        #[test]
        fn s2_t23_same_lock_inode_ignores_contents_length_and_timestamps() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before mutation"
            );
            let path = lock_path(&fixture);
            let before = fs::metadata(&path).unwrap();
            let hook_path = path.clone();
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            let result = with_phase_hook(
                move |phase| {
                    if phase == CapturePhase::AfterSharedLock {
                        let mut lock = OpenOptions::new().write(true).open(&hook_path).unwrap();
                        lock.set_len(0).unwrap();
                        lock.write_all(b"changed lock contents").unwrap();
                        lock.set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH))
                            .unwrap();
                        hook_invoked.set(true);
                    }
                    Ok(())
                },
                || scan(&fixture),
            );
            assert!(invoked.get());
            assert!(result.is_ok());
            let after = fs::metadata(&path).unwrap();
            assert_eq!(before.dev(), after.dev());
            assert_eq!(before.ino(), after.ino());
            assert_ne!(before.len(), after.len());
        }

        #[test]
        fn s2_t24_replacements_at_capture_and_final_hooks_are_rejected() {
            let fixture = make_fixture();
            let path = raw_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterSharedLock,
                mutation_at(
                    CapturePhase::AfterSharedLock,
                    move || replace_directory_with_empty(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = manifest_dir(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterSharedLock,
                mutation_at(
                    CapturePhase::AfterSharedLock,
                    move || replace_directory_with_empty(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = manifest_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterManifestCapture,
                mutation_at(
                    CapturePhase::AfterManifestCapture,
                    move || replace_file_with_same_bytes(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = batch_dir(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterMetadataCapture,
                mutation_at(
                    CapturePhase::AfterMetadataCapture,
                    move || replace_directory_with_empty(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = metadata_path(&fixture);
            let saved = fixture._temp.path().join("saved-metadata-inode");
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterMetadataCapture,
                mutation_at(
                    CapturePhase::AfterMetadataCapture,
                    move || replace_file_with_same_bytes_outside(&path, &saved),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = body_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::BeforeFinalBytes,
                mutation_at(
                    CapturePhase::BeforeFinalBytes,
                    move || replace_file_with_same_bytes(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let extra = batch_dir(&fixture).join("late-extra.json");
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            let result = with_phase_hook(
                mutation_at(
                    CapturePhase::BeforeFinalNames,
                    move || private_file(&extra, b"late extra"),
                    hook_invoked,
                ),
                || scan(&fixture),
            );
            assert!(invoked.get());
            assert_eq!(result.err(), Some(DailyEvidenceError::UnexpectedBatchEntry));

            let fixture = make_fixture();
            let metadata = metadata_path(&fixture);
            let saved = fixture._temp.path().join("final-metadata-inode");
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::BeforeFinalNames,
                mutation_at(
                    CapturePhase::BeforeFinalNames,
                    move || replace_file_with_same_bytes_outside(&metadata, &saved),
                    hook_invoked,
                ),
                invoked,
            );
        }

        #[test]
        fn s2_t24_final_membership_loss_of_body_or_metadata_is_source_changed() {
            for remove_metadata in [false, true] {
                let fixture = make_fixture();
                let path = if remove_metadata {
                    metadata_path(&fixture)
                } else {
                    body_path(&fixture)
                };
                let remaining_name = if remove_metadata {
                    "bars.json"
                } else {
                    "batch.json"
                };
                let batch_directory = batch_dir(&fixture);
                let saved = fixture._temp.path().join("captured-inode-outside-batch");
                let invoked = Rc::new(Cell::new(false));
                let hook_invoked = Rc::clone(&invoked);
                expect_replacement_error(
                    &fixture,
                    CapturePhase::BeforeFinalNames,
                    mutation_at(
                        CapturePhase::BeforeFinalNames,
                        move || {
                            fs::rename(&path, &saved).unwrap();
                            assert!(!path.exists());
                            assert!(saved.is_file());
                            let remaining: Vec<_> = fs::read_dir(&batch_directory)
                                .unwrap()
                                .map(|entry| entry.unwrap().file_name())
                                .collect();
                            assert_eq!(remaining, vec![std::ffi::OsString::from(remaining_name)]);
                        },
                        hook_invoked,
                    ),
                    invoked,
                );
            }
        }

        #[test]
        fn s2_t25_in_place_bytes_mode_and_link_mutations_fail_revalidation() {
            assert_eq!(
                validate_raw_stat_owner_mode(0o600, 4242, Some(4242)),
                Ok(())
            );
            assert_eq!(
                validate_raw_stat_owner_mode(0o600, 4243, Some(4242)),
                Err(DailyEvidenceError::UntrustedOwner)
            );

            let fixture = make_fixture();
            let path = manifest_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterManifestCapture,
                mutation_at(
                    CapturePhase::AfterManifestCapture,
                    move || mutate_in_place(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = metadata_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterMetadataCapture,
                mutation_at(
                    CapturePhase::AfterMetadataCapture,
                    move || mutate_in_place(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = body_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterBodyCapture,
                mutation_at(
                    CapturePhase::AfterBodyCapture,
                    move || mutate_in_place(&path),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = body_path(&fixture);
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterBodyCapture,
                mutation_at(
                    CapturePhase::AfterBodyCapture,
                    move || set_mode(&path, 0o400),
                    Rc::clone(&hook_invoked),
                ),
                invoked,
            );

            let fixture = make_fixture();
            let path = body_path(&fixture);
            let link = fixture._temp.path().join("extra-hard-link");
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            let result = with_phase_hook(
                mutation_at(
                    CapturePhase::AfterBodyCapture,
                    move || fs::hard_link(&path, &link).unwrap(),
                    Rc::clone(&hook_invoked),
                ),
                || scan(&fixture),
            );
            assert!(invoked.get());
            assert_eq!(result.err(), Some(DailyEvidenceError::HardLinkedFile));
        }

        #[test]
        fn s2_t28_injected_failures_release_lock_descriptors_and_preserve_tree() {
            let phases = [
                CapturePhase::AfterLockOpen,
                CapturePhase::AfterSharedLock,
                CapturePhase::AfterManifestCapture,
                CapturePhase::AfterMetadataCapture,
                CapturePhase::AfterBodyCapture,
                CapturePhase::BeforeBatchTreeOpen,
                CapturePhase::BeforeFinalBytes,
                CapturePhase::BeforeFinalNames,
            ];
            for phase in phases {
                let fixture = make_fixture();
                assert!(
                    scan(&fixture).is_ok(),
                    "fixture must be valid before injection"
                );
                let tree_before = tree_state(fixture._temp.path());
                let invoked = Rc::new(Cell::new(false));
                let hook_invoked = Rc::clone(&invoked);
                let (result, observation) = with_resource_observation(|observation| {
                    let result = with_phase_hook(
                        move |current| {
                            if current == phase {
                                hook_invoked.set(true);
                                Err(DailyEvidenceError::Io)
                            } else {
                                Ok(())
                            }
                        },
                        || scan(&fixture),
                    );
                    (result, observation)
                });
                assert!(invoked.get(), "missing failure hook {phase:?}");
                assert_eq!(result.err(), Some(DailyEvidenceError::Io));
                let descriptors = observed_descriptors(&observation);
                assert!(!descriptors.is_empty());
                assert_descriptors_closed(&observation, &descriptors, fixture._temp.path());
                assert_lock_closed_last(&observation);
                assert!(
                    observation
                        .lock()
                        .unwrap()
                        .events
                        .contains(&ObservationEvent::ConstructionCleanupBeforeLockRelease)
                );
                assert_eq!(tree_state(fixture._temp.path()), tree_before);
                lock_exclusive(&fixture);
            }

            let fixture = make_fixture();
            let (snapshot, observation) =
                with_resource_observation(|observation| (scan(&fixture).unwrap(), observation));
            let descriptors = observed_descriptors(&observation);
            assert_eq!(descriptors.len(), snapshot.stats.open_descriptors);
            assert!(
                descriptors
                    .iter()
                    .all(|descriptor| descriptor_is_live(&observation, *descriptor))
            );
            let competing = OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path(&fixture))
                .unwrap();
            assert!(matches!(
                FileExt::try_lock_exclusive(&competing),
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock
            ));
            drop(competing);
            drop(snapshot);
            assert_descriptors_closed(&observation, &descriptors, fixture._temp.path());
            assert_lock_closed_last(&observation);
            lock_exclusive(&fixture);
        }

        #[test]
        fn s2_t28_failure_unwind_closes_temporary_leaves_before_lock_release() {
            let fixture = make_fixture();
            assert!(
                scan(&fixture).is_ok(),
                "fixture must be valid before injection"
            );
            let lock_path = lock_path(&fixture);
            let lock_path_for_hook = lock_path.clone();
            let fixture_root_for_hook = fixture._temp.path().to_path_buf();
            let cleanup_seen = Rc::new(Cell::new(false));
            let cleanup_seen_by_hook = Rc::clone(&cleanup_seen);
            let (result, observation) = with_resource_observation(|observation| {
                let observation_for_hook = Arc::clone(&observation);
                let result = with_phase_hook(
                    move |phase| {
                        if phase == CapturePhase::AfterBodyCapture {
                            return Err(DailyEvidenceError::Io);
                        }
                        if phase == CapturePhase::BeforeLockRelease {
                            let descriptors = observed_descriptors(&observation_for_hook);
                            let lock_descriptors: Vec<_> = descriptors
                                .iter()
                                .filter(|descriptor| descriptor.role == DescriptorRole::Lock)
                                .copied()
                                .collect();
                            assert_eq!(lock_descriptors.len(), 1);
                            assert!(descriptor_is_live(
                                &observation_for_hook,
                                lock_descriptors[0]
                            ));
                            let non_lock: Vec<_> = descriptors
                                .iter()
                                .filter(|descriptor| descriptor.role != DescriptorRole::Lock)
                                .copied()
                                .collect();
                            assert_descriptors_closed(
                                &observation_for_hook,
                                &non_lock,
                                &fixture_root_for_hook,
                            );
                            let competing = OpenOptions::new()
                                .read(true)
                                .write(true)
                                .open(&lock_path_for_hook)
                                .unwrap();
                            assert!(matches!(
                                FileExt::try_lock_exclusive(&competing),
                                Err(ref error)
                                    if error.kind() == std::io::ErrorKind::WouldBlock
                            ));
                            drop(competing);
                            cleanup_seen_by_hook.set(true);
                        }
                        Ok(())
                    },
                    || scan(&fixture),
                );
                (result, observation)
            });
            assert_eq!(result.err(), Some(DailyEvidenceError::Io));
            assert!(cleanup_seen.get());
            assert_descriptors_closed(
                &observation,
                &observed_descriptors(&observation),
                fixture._temp.path(),
            );
            assert_lock_closed_last(&observation);
            lock_exclusive(&fixture);
        }
    }

    #[cfg(not(test))]
    #[derive(Clone, Copy)]
    enum CapturePhase {
        AfterLockOpen,
        AfterSharedLock,
        AfterManifestCapture,
        AfterMetadataCapture,
        AfterBodyCapture,
        BeforeBatchTreeOpen,
        BeforeFinalBytes,
        BeforeFinalNames,
        BeforeLockRelease,
    }

    #[cfg(not(test))]
    fn hit_phase(_phase: CapturePhase) -> Result<(), DailyEvidenceError> {
        Ok(())
    }
}
