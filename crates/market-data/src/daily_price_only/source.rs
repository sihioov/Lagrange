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
}

impl RawProvider {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Kis => "kis",
            Self::KisCalendar => "kis-calendar",
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

        checked_capacity(
            values.max_manifest_records,
            std::mem::size_of::<PendingBatch>(),
        )?;
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
}

impl CommittedRawSnapshot {
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
        if reference.file_name == "batch.json" || validate_file_name(&reference.file_name).is_err()
        {
            return Err(DailyEvidenceError::InvalidFileName);
        }
        if reference.provider != self.scope.provider() || reference.market != self.scope.market() {
            return Err(DailyEvidenceError::ScopeMismatch);
        }

        let index_position = self
            .batch_index
            .binary_search_by_key(&reference.batch_id, |(batch_id, _)| *batch_id)
            .map_err(|_| DailyEvidenceError::NotCommitted)?;
        let batch_index = self.batch_index[index_position].1;
        let batch = self
            .batches
            .get(batch_index)
            .ok_or(DailyEvidenceError::NotCommitted)?;
        let file_index = batch
            .entry
            .files
            .iter()
            .position(|file| file.file_name == reference.file_name)
            .ok_or(DailyEvidenceError::NotCommitted)?;
        if batch.entry.files[file_index].content_hash != reference.content_hash
            || batch.entry.retrieved_at != reference.retrieved_at
        {
            return Err(DailyEvidenceError::ReferenceMismatch);
        }
        Ok(CommittedRawFile {
            batch,
            index: file_index,
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
        self.entry.batch_id
    }

    pub fn batch_date(&self) -> TradingDate {
        self.entry.date
    }

    pub fn retrieved_at(&self) -> UtcTimestamp {
        self.entry.retrieved_at
    }

    pub fn mode(&self) -> FetchMode {
        self.entry.mode
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
        (0..self.files.len()).map(|index| CommittedRawFile { batch: self, index })
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
        &self.batch.entry.files[self.index].file_name
    }

    pub fn kind(&self) -> ResponseKind {
        self.batch.entry.files[self.index].kind
    }

    pub fn content_hash(&self) -> &'snapshot ContentHash {
        &self.batch.files[self.index].content_hash
    }

    pub fn size_bytes(&self) -> u64 {
        self.batch.entry.files[self.index].size_bytes
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
    file: std::fs::File,
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
    use std::os::unix::ffi::OsStrExt;

    use fs2::FileExt;
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RawDir, SeekFrom as RustixSeekFrom, fstat, open, openat,
        seek, statat,
    };
    use rustix::io::Errno;
    use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

    const DIRECTORY_BUFFER_BYTES: usize = 4096;
    const READ_BUFFER_BYTES: usize = 8192;

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
                    links: None,
                    size: None,
                    mtime: None,
                    ctime: None,
                    node_type: expected,
                }),
                NodeType::RegularFile => {
                    validate_single_link_count(u64::from(stat.st_nlink))?;
                    if stat.st_size < 0 {
                        return Err(DailyEvidenceError::SizeMismatch);
                    }
                    Ok(Self {
                        device: stat.st_dev,
                        inode: stat.st_ino,
                        mode: stat.st_mode,
                        uid: stat.st_uid,
                        links: Some(u64::from(stat.st_nlink)),
                        size: Some(stat.st_size),
                        mtime: Some((stat.st_mtime, u64::from(stat.st_mtime_nsec))),
                        ctime: Some((stat.st_ctime, u64::from(stat.st_ctime_nsec))),
                        node_type: expected,
                    })
                }
            }
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
            self.bytes
                .try_reserve_exact(name.len())
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            self.bytes.extend_from_slice(name);
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

    struct DirectoryNode {
        file: File,
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

    struct HeldFile {
        file: File,
        parent: usize,
        name: NameRange,
        fingerprint: Fingerprint,
    }

    /// Field order is intentional: metadata, directories, then the one lock FD.
    pub(super) struct ResourceGuard {
        _manifest_file: HeldFile,
        _directories: Vec<DirectoryNode>,
        _names: NamePool,
        _lock_file: HeldFile,
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
            Ok(())
        }

        fn open<T>(
            &mut self,
            operation: impl FnOnce() -> Result<T, DailyEvidenceError>,
        ) -> Result<T, DailyEvidenceError> {
            self.reserve_before_open()?;
            match operation() {
                Ok(value) => Ok(value),
                Err(error) => {
                    self.current -= 1;
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
            Ok(())
        }
    }

    fn errno_error(error: Errno, missing: DailyEvidenceError) -> DailyEvidenceError {
        if error == Errno::NOENT {
            missing
        } else if error == Errno::LOOP || error == Errno::NOTDIR {
            DailyEvidenceError::UnsafePath
        } else if error == Errno::ACCES || error == Errno::PERM {
            DailyEvidenceError::PermissionDenied
        } else {
            DailyEvidenceError::Io
        }
    }

    fn errno_io(error: Errno) -> DailyEvidenceError {
        if error == Errno::ACCES || error == Errno::PERM {
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

    fn stat_at(
        parent: &File,
        name: &OsStr,
        missing: DailyEvidenceError,
    ) -> Result<rustix::fs::Stat, DailyEvidenceError> {
        statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| errno_error(error, missing))
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

    fn open_child_directory(
        parent_index: usize,
        name: &[u8],
        raw_owner: Option<u32>,
        missing: DailyEvidenceError,
        directories: &mut Vec<DirectoryNode>,
        name_pool: &mut NamePool,
        descriptors: &mut DescriptorBudget,
    ) -> Result<usize, DailyEvidenceError> {
        let range = name_pool.append(name)?;
        let name_os = name_pool.get(range)?;
        let pre_stat = stat_at(&directories[parent_index].file, name_os, missing)?;
        let pre_fingerprint = Fingerprint::from_stat(&pre_stat, NodeType::Directory, raw_owner)?;
        let file = descriptors.open(|| {
            let descriptor = openat(
                &directories[parent_index].file,
                name_os,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| errno_error(error, missing))?;
            Ok(File::from(descriptor))
        })?;
        let opened_stat = fstat(&file).map_err(errno_io)?;
        let opened = Fingerprint::from_stat(&opened_stat, NodeType::Directory, raw_owner)?;
        if opened != pre_fingerprint {
            return Err(DailyEvidenceError::SourceChanged);
        }
        let named = stat_at(
            &directories[parent_index].file,
            name_pool.get(range)?,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_path_fingerprint(&named, pre_fingerprint, raw_owner)?;
        let index = directories.len();
        directories
            .try_reserve(1)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        directories.push(DirectoryNode {
            file,
            parent: Some(parent_index),
            name: Some(range),
            fingerprint: pre_fingerprint,
            raw_protected: raw_owner.is_some(),
        });
        Ok(index)
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
        let component_count = root
            .components()
            .filter(|component| matches!(component, std::path::Component::Normal(_)))
            .count();
        if component_count > limits.values.max_data_root_components {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        let mut components = Vec::new();
        components
            .try_reserve_exact(component_count)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        for component in root.components() {
            match component {
                std::path::Component::RootDir | std::path::Component::CurDir => {}
                std::path::Component::Normal(name) => {
                    let mut component_bytes = Vec::new();
                    component_bytes
                        .try_reserve_exact(name.as_bytes().len())
                        .map_err(|_| DailyEvidenceError::AllocationFailed)?;
                    component_bytes.extend_from_slice(name.as_bytes());
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

    fn snapshot(
        data_root: &Path,
        scope: ApprovedRawScope,
        limits: ScanLimits,
    ) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
        let components = root_components(data_root, &limits)?;
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
        let mut names = NamePool::default();
        let mut descriptors = DescriptorBudget {
            limit: limits.values.max_open_descriptors,
            current: 0,
            peak: 0,
        };

        let root_stat = statat(rustix::fs::CWD, "/", AtFlags::empty())
            .map_err(|error| errno_error(error, DailyEvidenceError::MissingControl))?;
        let root_fingerprint = Fingerprint::from_stat(&root_stat, NodeType::Directory, None)?;
        let root_file = descriptors.open(|| {
            open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|error| errno_error(error, DailyEvidenceError::MissingControl))
        })?;
        let opened_root_stat = fstat(&root_file).map_err(errno_io)?;
        let opened_root = Fingerprint::from_stat(&opened_root_stat, NodeType::Directory, None)?;
        if opened_root != root_fingerprint {
            return Err(DailyEvidenceError::SourceChanged);
        }
        directories.push(DirectoryNode {
            file: root_file,
            parent: None,
            name: None,
            fingerprint: root_fingerprint,
            raw_protected: false,
        });
        let root_index = 0;
        let mut cursor = root_index;
        for component in &components {
            cursor = open_child_directory(
                cursor,
                component,
                None,
                DailyEvidenceError::MissingControl,
                &mut directories,
                &mut names,
                &mut descriptors,
            )?;
        }
        let raw_index = open_child_directory(
            cursor,
            b"raw",
            Some(scope.trusted_raw_owner_uid),
            DailyEvidenceError::MissingControl,
            &mut directories,
            &mut names,
            &mut descriptors,
        )?;
        let manifest_root = open_child_directory(
            raw_index,
            b"manifests",
            Some(scope.trusted_raw_owner_uid),
            DailyEvidenceError::MissingControl,
            &mut directories,
            &mut names,
            &mut descriptors,
        )?;
        let manifest_provider_name = format!("provider={}", scope.provider());
        let manifest_provider = open_child_directory(
            manifest_root,
            manifest_provider_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            DailyEvidenceError::MissingControl,
            &mut directories,
            &mut names,
            &mut descriptors,
        )?;
        let manifest_market_name = format!("market={}", scope.market());
        let manifest_market = open_child_directory(
            manifest_provider,
            manifest_market_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            DailyEvidenceError::MissingControl,
            &mut directories,
            &mut names,
            &mut descriptors,
        )?;

        let lock_name = names.append(b"commit.lock")?;
        let lock_file = open_leaf_control(
            manifest_market,
            lock_name,
            &mut directories,
            &names,
            &mut descriptors,
            DailyEvidenceError::MissingControl,
            Some(scope.trusted_raw_owner_uid),
            None,
        )?;
        hit_phase(CapturePhase::AfterLockOpen)?;
        match FileExt::try_lock_shared(&lock_file.file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(DailyEvidenceError::LockBusy);
            }
            Err(error) => return Err(io_error(error)),
        }
        hit_phase(CapturePhase::AfterSharedLock)?;
        revalidate_directories(&directories, &names, scope.trusted_raw_owner_uid)?;
        revalidate_held_file(
            &lock_file,
            &directories,
            &names,
            Some(scope.trusted_raw_owner_uid),
        )?;

        let manifest_name = names.append(b"manifest.jsonl")?;
        let manifest_file = open_leaf_control(
            manifest_market,
            manifest_name,
            &mut directories,
            &names,
            &mut descriptors,
            DailyEvidenceError::MissingControl,
            Some(scope.trusted_raw_owner_uid),
            Some(limits.values.max_manifest_bytes),
        )?;
        let manifest_length = checked_stat_size(manifest_file.fingerprint)?;
        let mut retained = RetainedBytes {
            total: 0,
            limit: limits.values.max_retained_bytes,
        };
        retained.reserve(manifest_length)?;
        let manifest_captured = read_exact_captured(
            &manifest_file.file,
            manifest_length,
            limits.values.max_manifest_bytes,
        )?;
        revalidate_held_file(
            &manifest_file,
            &directories,
            &names,
            Some(scope.trusted_raw_owner_uid),
        )?;
        hit_phase(CapturePhase::AfterManifestCapture)?;
        let manifest_hash = ContentHash::from_bytes(&manifest_captured);
        let manifest_bytes = Arc::new(manifest_captured);
        let (pending, batch_index, records, total_files) =
            parse_manifest(&scope, &limits, manifest_bytes.as_slice())?;

        let distinct_dates = distinct_date_count(&pending)?;
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
        if expected_descriptors > limits.values.max_open_descriptors {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        let directory_capacity = expected_descriptors
            .checked_sub(directories.len())
            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        directories
            .try_reserve_exact(directory_capacity)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;

        let mut batches = Vec::new();
        batches
            .try_reserve_exact(pending.len())
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        for pending_batch in pending {
            let batch = capture_batch(
                pending_batch,
                &mut batches,
                &mut directories,
                &mut names,
                &mut descriptors,
                &mut retained,
                &scope,
                &limits,
                raw_index,
                &manifest_bytes,
            )?;
            batches.push(batch);
        }
        hit_phase(CapturePhase::BeforeFinalBytes)?;
        final_byte_checks(
            &manifest_file,
            &manifest_bytes,
            &batches,
            &directories,
            &names,
            scope.trusted_raw_owner_uid,
        )?;
        for batch in &batches {
            let batch_directory = batch.batch_json.parent_index;
            let mut final_seen = Vec::new();
            verify_batch_membership(
                &directories[batch_directory],
                &batch.entry,
                &mut final_seen,
                &limits,
            )?;
        }
        hit_phase(CapturePhase::BeforeFinalNames)?;
        revalidate_directories(&directories, &names, scope.trusted_raw_owner_uid)?;
        revalidate_held_file(
            &manifest_file,
            &directories,
            &names,
            Some(scope.trusted_raw_owner_uid),
        )?;
        revalidate_held_file(
            &lock_file,
            &directories,
            &names,
            Some(scope.trusted_raw_owner_uid),
        )?;
        revalidate_all_batch_files(&batches, &directories, &names, scope.trusted_raw_owner_uid)?;

        let stats = SnapshotStats {
            manifest_bytes: manifest_bytes.len(),
            manifest_records: records,
            batches: batches.len(),
            files: total_files,
            retained_evidence_bytes: retained.total,
            open_descriptors: descriptors.current,
            peak_open_descriptors: descriptors.peak,
        };
        Ok(CommittedRawSnapshot {
            scope,
            limits,
            stats,
            manifest_bytes,
            manifest_sha256: manifest_hash,
            batches,
            batch_index,
            _resources: ResourceGuard {
                _manifest_file: manifest_file,
                _directories: directories,
                _names: names,
                _lock_file: lock_file,
            },
        })
    }

    fn distinct_date_count(pending: &[PendingBatch]) -> Result<usize, DailyEvidenceError> {
        let mut dates = Vec::new();
        for batch in pending {
            if dates.iter().any(|date| *date == batch.entry.date) {
                continue;
            }
            dates
                .try_reserve(1)
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            dates.push(batch.entry.date);
        }
        Ok(dates.len())
    }

    fn checked_stat_size(fingerprint: Fingerprint) -> Result<usize, DailyEvidenceError> {
        let size = fingerprint.size.ok_or(DailyEvidenceError::SizeMismatch)?;
        usize::try_from(size).map_err(|_| DailyEvidenceError::ArithmeticOverflow)
    }

    fn open_leaf_control(
        parent: usize,
        name: NameRange,
        directories: &mut Vec<DirectoryNode>,
        names: &NamePool,
        descriptors: &mut DescriptorBudget,
        missing: DailyEvidenceError,
        raw_owner: Option<u32>,
        byte_cap: Option<usize>,
    ) -> Result<HeldFile, DailyEvidenceError> {
        let name_os = names.get(name)?;
        let parent_file = &directories.get(parent).ok_or(DailyEvidenceError::Io)?.file;
        let pre_stat = stat_at(parent_file, name_os, missing)?;
        let fingerprint = Fingerprint::from_stat(&pre_stat, NodeType::RegularFile, raw_owner)?;
        if let Some(maximum) = byte_cap {
            let size = checked_stat_size(fingerprint)?;
            if size > maximum {
                return Err(DailyEvidenceError::SourceScanLimit);
            }
        }
        let name_os = names.get(name)?;
        let file = descriptors.open(|| {
            let descriptor = openat(
                &directories[parent].file,
                name_os,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| errno_error(error, missing))?;
            Ok(File::from(descriptor))
        })?;
        let opened_stat = fstat(&file).map_err(errno_io)?;
        let opened = Fingerprint::from_stat(&opened_stat, NodeType::RegularFile, raw_owner)?;
        if opened != fingerprint {
            return Err(DailyEvidenceError::SourceChanged);
        }
        let named_stat = stat_at(
            &directories[parent].file,
            names.get(name)?,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_path_fingerprint(&named_stat, fingerprint, raw_owner)?;
        Ok(HeldFile {
            file,
            parent,
            name,
            fingerprint,
        })
    }

    fn open_provider_file(
        parent: usize,
        file_name: &str,
        expected_size: u64,
        expected_hash: &ContentHash,
        directories: &mut Vec<DirectoryNode>,
        names: &mut NamePool,
        descriptors: &mut DescriptorBudget,
        retained: &mut RetainedBytes,
        scope: &ApprovedRawScope,
        limits: &ScanLimits,
    ) -> Result<CapturedLeaf, DailyEvidenceError> {
        validate_file_name(file_name)?;
        let name_range = names.append(file_name.as_bytes())?;
        let parent_file = &directories.get(parent).ok_or(DailyEvidenceError::Io)?.file;
        let name = names.get(name_range)?;
        let pre_stat = stat_at(
            parent_file,
            names.get(name_range)?,
            DailyEvidenceError::MissingCommittedEvidence,
        )?;
        let fingerprint = Fingerprint::from_stat(
            &pre_stat,
            NodeType::RegularFile,
            Some(scope.trusted_raw_owner_uid),
        )?;
        let actual_size = checked_stat_size(fingerprint)?;
        if expected_size == 0 {
            return Err(DailyEvidenceError::SizeMismatch);
        }
        let expected_size =
            usize::try_from(expected_size).map_err(|_| DailyEvidenceError::ArithmeticOverflow)?;
        if expected_size > limits.values.max_source_file_bytes
            || actual_size > limits.values.max_source_file_bytes
        {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        if actual_size != expected_size {
            return Err(DailyEvidenceError::SizeMismatch);
        }
        retained.reserve(actual_size)?;
        let file = descriptors.open(|| {
            let descriptor = openat(
                &directories[parent].file,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|error| errno_error(error, DailyEvidenceError::MissingCommittedEvidence))?;
            Ok(File::from(descriptor))
        })?;
        let opened_stat = fstat(&file).map_err(errno_io)?;
        let opened = Fingerprint::from_stat(
            &opened_stat,
            NodeType::RegularFile,
            Some(scope.trusted_raw_owner_uid),
        )?;
        if opened != fingerprint {
            return Err(DailyEvidenceError::SourceChanged);
        }
        let named_stat = stat_at(
            &directories[parent].file,
            names.get(name_range)?,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_path_fingerprint(&named_stat, fingerprint, Some(scope.trusted_raw_owner_uid))?;
        let captured =
            read_exact_captured(&file, actual_size, limits.values.max_source_file_bytes)?;
        let named_after = stat_at(
            &directories[parent].file,
            name,
            DailyEvidenceError::SourceChanged,
        )?;
        verify_path_fingerprint(
            &fstat(&file).map_err(errno_io)?,
            fingerprint,
            Some(scope.trusted_raw_owner_uid),
        )?;
        verify_path_fingerprint(&named_after, fingerprint, Some(scope.trusted_raw_owner_uid))?;
        let hash = ContentHash::from_bytes(&captured);
        if &hash != expected_hash {
            return Err(DailyEvidenceError::HashMismatch);
        }
        hit_phase(CapturePhase::AfterBodyCapture)?;
        Ok(CapturedLeaf {
            bytes: captured,
            content_hash: hash,
            file,
            fingerprint,
            parent_index: parent,
            name: name_range,
        })
    }

    fn capture_batch(
        pending: PendingBatch,
        batches: &mut Vec<CommittedRawBatch>,
        directories: &mut Vec<DirectoryNode>,
        names: &mut NamePool,
        descriptors: &mut DescriptorBudget,
        retained: &mut RetainedBytes,
        scope: &ApprovedRawScope,
        limits: &ScanLimits,
        raw_index: usize,
        manifest: &Arc<Vec<u8>>,
    ) -> Result<CommittedRawBatch, DailyEvidenceError> {
        let entry = pending.entry;
        let provider_name = format!("provider={}", scope.provider());
        let provider_directory = find_or_open_child(
            raw_index,
            provider_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            directories,
            names,
            descriptors,
        )?;
        let market_name = format!("market={}", scope.market());
        let market_directory = find_or_open_child(
            provider_directory,
            market_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            directories,
            names,
            descriptors,
        )?;
        let date_name = format!("date={}", entry.date);
        let date_directory = find_or_open_child(
            market_directory,
            date_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            directories,
            names,
            descriptors,
        )?;
        let batch_name = format!("batch={}", entry.batch_id);
        let batch_directory = open_child_directory(
            date_directory,
            batch_name.as_bytes(),
            Some(scope.trusted_raw_owner_uid),
            DailyEvidenceError::MissingCommittedEvidence,
            directories,
            names,
            descriptors,
        )?;

        let mut seen = Vec::new();
        seen.try_reserve_exact(entry.files.len())
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        seen.resize(entry.files.len(), false);
        verify_batch_membership(&directories[batch_directory], &entry, &mut seen, limits)?;

        let metadata_name = names.append(b"batch.json")?;
        let metadata = open_leaf_control(
            batch_directory,
            metadata_name,
            directories,
            names,
            descriptors,
            DailyEvidenceError::MissingCommittedEvidence,
            Some(scope.trusted_raw_owner_uid),
            Some(limits.values.max_batch_metadata_bytes),
        )?;
        let metadata_size = checked_stat_size(metadata.fingerprint)?;
        retained.reserve(metadata_size)?;
        let metadata_bytes = read_exact_captured(
            &metadata.file,
            metadata_size,
            limits.values.max_batch_metadata_bytes,
        )?;
        revalidate_held_file(
            &metadata,
            directories,
            names,
            Some(scope.trusted_raw_owner_uid),
        )?;
        let stored = parse_metadata(&metadata_bytes, limits)?;
        if stored != entry {
            return Err(DailyEvidenceError::BatchMetadataMismatch);
        }
        let metadata_hash = ContentHash::from_bytes(&metadata_bytes);
        let batch_json = CapturedLeaf {
            bytes: metadata_bytes,
            content_hash: metadata_hash,
            file: metadata.file,
            fingerprint: metadata.fingerprint,
            parent_index: metadata.parent,
            name: metadata.name,
        };
        hit_phase(CapturePhase::AfterMetadataCapture)?;

        let next_file_count = batches
            .iter()
            .try_fold(entry.files.len(), |sum, batch| {
                sum.checked_add(batch.entry.files.len())
            })
            .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
        if next_file_count > limits.values.max_files {
            return Err(DailyEvidenceError::SourceScanLimit);
        }
        let mut captured_files = Vec::new();
        captured_files
            .try_reserve_exact(entry.files.len())
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        for file_entry in &entry.files {
            captured_files.push(open_provider_file(
                batch_directory,
                &file_entry.file_name,
                file_entry.size_bytes,
                &file_entry.content_hash,
                directories,
                names,
                descriptors,
                retained,
                scope,
                limits,
            )?);
        }
        verify_batch_membership(&directories[batch_directory], &entry, &mut seen, limits)?;
        let batch = CommittedRawBatch {
            entry,
            manifest: Arc::clone(manifest),
            manifest_line: pending.line,
            manifest_line_sha256: pending.line_hash,
            manifest_line_number: pending.line_number,
            files: captured_files,
            batch_json,
        };
        Ok(batch)
    }

    fn find_or_open_child(
        parent: usize,
        name: &[u8],
        raw_owner: Option<u32>,
        directories: &mut Vec<DirectoryNode>,
        names: &mut NamePool,
        descriptors: &mut DescriptorBudget,
    ) -> Result<usize, DailyEvidenceError> {
        if let Some(index) = directories.iter().position(|candidate| {
            candidate.parent == Some(parent)
                && candidate
                    .name
                    .and_then(|range| names.get(range).ok())
                    .is_some_and(|candidate_name| candidate_name.as_bytes() == name)
        }) {
            return Ok(index);
        }
        open_child_directory(
            parent,
            name,
            raw_owner,
            DailyEvidenceError::MissingCommittedEvidence,
            directories,
            names,
            descriptors,
        )
    }

    fn verify_batch_membership(
        batch_directory: &DirectoryNode,
        entry: &ManifestEntry,
        seen: &mut Vec<bool>,
        limits: &ScanLimits,
    ) -> Result<(), DailyEvidenceError> {
        if seen.len() != entry.files.len() {
            seen.clear();
            if entry.files.len() > limits.values.max_files_per_batch {
                return Err(DailyEvidenceError::SourceScanLimit);
            }
            seen.try_reserve_exact(entry.files.len())
                .map_err(|_| DailyEvidenceError::AllocationFailed)?;
            seen.resize(entry.files.len(), false);
        }
        for seen_file in seen.iter_mut() {
            *seen_file = false;
        }
        let mut metadata_seen = false;
        let mut buffer = [MaybeUninit::<u8>::uninit(); DIRECTORY_BUFFER_BYTES];
        seek(batch_directory.file.as_fd(), RustixSeekFrom::Start(0)).map_err(errno_io)?;
        let mut directory = RawDir::new(batch_directory.file.as_fd(), &mut buffer);
        while let Some(result) = directory.next() {
            let item = result.map_err(errno_io)?;
            let name = item.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if name == b"batch.json" {
                if metadata_seen {
                    return Err(DailyEvidenceError::UnexpectedBatchEntry);
                }
                metadata_seen = true;
                continue;
            }
            let Some(index) = entry
                .files
                .iter()
                .position(|file| file.file_name.as_bytes() == name)
            else {
                return Err(DailyEvidenceError::UnexpectedBatchEntry);
            };
            let Some(slot) = seen.get_mut(index) else {
                return Err(DailyEvidenceError::UnexpectedBatchEntry);
            };
            if *slot {
                return Err(DailyEvidenceError::UnexpectedBatchEntry);
            }
            *slot = true;
        }
        if !metadata_seen || seen.iter().any(|seen_file| !seen_file) {
            return Err(DailyEvidenceError::MissingCommittedEvidence);
        }
        Ok(())
    }

    fn revalidate_held_file(
        held: &HeldFile,
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_owner: Option<u32>,
    ) -> Result<(), DailyEvidenceError> {
        let parent = directories
            .get(held.parent)
            .ok_or(DailyEvidenceError::SourceChanged)?;
        let name = names.get(held.name)?;
        let descriptor_stat = fstat(&held.file).map_err(errno_io)?;
        verify_path_fingerprint(&descriptor_stat, held.fingerprint, raw_owner)?;
        let named = stat_at(&parent.file, name, DailyEvidenceError::SourceChanged)?;
        verify_path_fingerprint(&named, held.fingerprint, raw_owner)
    }

    fn revalidate_directories(
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        for directory in directories {
            let descriptor_stat = fstat(&directory.file).map_err(errno_io)?;
            verify_path_fingerprint(&descriptor_stat, directory.fingerprint, None)?;
            if let (Some(parent_index), Some(name_range)) = (directory.parent, directory.name) {
                let parent = directories
                    .get(parent_index)
                    .ok_or(DailyEvidenceError::SourceChanged)?;
                let name = names.get(name_range)?;
                let named = stat_at(&parent.file, name, DailyEvidenceError::SourceChanged)?;
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

    fn revalidate_all_batch_files(
        batches: &[CommittedRawBatch],
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        for batch in batches {
            revalidate_capture(&batch.batch_json, directories, names, raw_uid)?;
            for captured in &batch.files {
                revalidate_capture(captured, directories, names, raw_uid)?;
            }
        }
        Ok(())
    }

    fn revalidate_capture(
        capture: &CapturedLeaf,
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        let parent = directories
            .get(capture.parent_index)
            .ok_or(DailyEvidenceError::SourceChanged)?;
        let parent_stat = &parent.file;
        let name = names.get(capture.name)?;
        let descriptor_stat = fstat(&capture.file).map_err(errno_io)?;
        verify_path_fingerprint(&descriptor_stat, capture.fingerprint, Some(raw_uid))?;
        let named = stat_at(parent_stat, name, DailyEvidenceError::SourceChanged)?;
        verify_path_fingerprint(&named, capture.fingerprint, Some(raw_uid))
    }

    fn final_byte_checks(
        manifest: &HeldFile,
        manifest_bytes: &[u8],
        batches: &[CommittedRawBatch],
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        compare_held_bytes(manifest, manifest_bytes, directories, names, raw_uid)?;
        for batch in batches {
            compare_captured_bytes(
                &batch.batch_json,
                &batch.batch_json.bytes,
                directories,
                names,
                raw_uid,
            )?;
            for captured in &batch.files {
                compare_captured_bytes(captured, &captured.bytes, directories, names, raw_uid)?;
            }
        }
        Ok(())
    }

    fn compare_held_bytes(
        held: &HeldFile,
        expected: &[u8],
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        revalidate_held_file(held, directories, names, Some(raw_uid))?;
        compare_descriptor_bytes(&held.file, expected)?;
        revalidate_held_file(held, directories, names, Some(raw_uid))
    }

    fn compare_captured_bytes(
        capture: &CapturedLeaf,
        expected: &[u8],
        directories: &[DirectoryNode],
        names: &NamePool,
        raw_uid: u32,
    ) -> Result<(), DailyEvidenceError> {
        revalidate_capture(capture, directories, names, raw_uid)?;
        compare_descriptor_bytes(&capture.file, expected)?;
        revalidate_capture(capture, directories, names, raw_uid)
    }

    fn compare_descriptor_bytes(file: &File, expected: &[u8]) -> Result<(), DailyEvidenceError> {
        let mut reader = file;
        reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut scratch = [0u8; READ_BUFFER_BYTES];
        let mut offset = 0usize;
        while offset < expected.len() {
            let wanted = (expected.len() - offset).min(scratch.len());
            let count = reader.read(&mut scratch[..wanted]).map_err(io_error)?;
            if count == 0 {
                return Err(DailyEvidenceError::SourceChanged);
            }
            let next = offset
                .checked_add(count)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            if expected.get(offset..next) != Some(&scratch[..count]) {
                return Err(DailyEvidenceError::SourceChanged);
            }
            offset = next;
        }
        let mut probe = [0u8; 1];
        if reader.read(&mut probe).map_err(io_error)? != 0 {
            return Err(DailyEvidenceError::SourceChanged);
        }
        Ok(())
    }

    fn read_exact_captured(
        file: &File,
        expected: usize,
        byte_cap: usize,
    ) -> Result<Vec<u8>, DailyEvidenceError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(expected)
            .map_err(|_| DailyEvidenceError::AllocationFailed)?;
        let mut reader = file;
        let mut scratch = [0u8; READ_BUFFER_BYTES];
        while bytes.len() < expected {
            let wanted = (expected - bytes.len()).min(scratch.len());
            let count = reader.read(&mut scratch[..wanted]).map_err(io_error)?;
            if count == 0 {
                return Err(DailyEvidenceError::SizeMismatch);
            }
            let next = bytes
                .len()
                .checked_add(count)
                .ok_or(DailyEvidenceError::ArithmeticOverflow)?;
            bytes.extend_from_slice(&scratch[..count]);
            if next > byte_cap {
                return Err(DailyEvidenceError::SourceScanLimit);
            }
        }
        let mut probe = [0u8; 1];
        if reader.read(&mut probe).map_err(io_error)? != 0 {
            return Err(if expected >= byte_cap {
                DailyEvidenceError::SourceScanLimit
            } else {
                DailyEvidenceError::SourceChanged
            });
        }
        Ok(bytes)
    }

    fn parse_manifest(
        scope: &ApprovedRawScope,
        limits: &ScanLimits,
        bytes: &[u8],
    ) -> Result<(Vec<PendingBatch>, Vec<(BatchId, usize)>, usize, usize), DailyEvidenceError> {
        let mut pending = Vec::new();
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
        Ok((pending, index, records, total_files))
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
                    if depth > 0 {
                        depth -= 1;
                    }
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
                    "batch_id" => set_once(&mut batch_id, map.next_value()?, "batch_id")?,
                    "provider" => set_once(&mut provider, map.next_value()?, "provider")?,
                    "market" => set_once(&mut market, map.next_value()?, "market")?,
                    "date" => set_once(&mut date, map.next_value()?, "date")?,
                    "retrieved_at" => {
                        set_once(&mut retrieved_at, map.next_value()?, "retrieved_at")?
                    }
                    "mode" => set_once(&mut mode, map.next_value()?, "mode")?,
                    "entitlement_reference" => set_once(
                        &mut entitlement_reference,
                        map.next_value()?,
                        "entitlement_reference",
                    )?,
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
                let Some(file) = sequence.next_element::<crate::storage::FileEntry>()? else {
                    break;
                };
                files.try_reserve(1).map_err(|_| {
                    self.state.resource.set(Some(ParseResource::Allocation));
                    de::Error::custom("bounded Raw allocation failed")
                })?;
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
        BeforeFinalBytes,
        BeforeFinalNames,
    }

    #[cfg(test)]
    thread_local! {
        static TEST_PHASE_HOOK: RefCell<Option<Box<dyn FnMut(CapturePhase) -> Result<(), DailyEvidenceError>>>> = const { RefCell::new(None) };
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
    mod private_tests {
        use super::*;
        use std::cell::Cell;
        use std::fs::{self, OpenOptions};
        use std::io::{Seek, SeekFrom, Write};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::path::{Path, PathBuf};
        use std::rc::Rc;

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

        fn make_fixture() -> Fixture {
            let temp = tempfile::tempdir().unwrap();
            let data_root = temp.path().join("data");
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
            private_dir(&manifest_dir);
            private_dir(&batch_dir);
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

        fn scan(fixture: &Fixture) -> Result<CommittedRawSnapshot, DailyEvidenceError> {
            let scope = ApprovedRawScope::new("kis", "kr", fixture.uid)?;
            let limits = ScanLimits::new(ScanLimitValues {
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
            })?;
            CommittedRawReader::new(&fixture.data_root).snapshot(scope, limits)
        }

        fn replace_file_with_same_bytes(path: &Path) {
            let bytes = fs::read(path).unwrap();
            let saved = path.with_extension("saved");
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

        fn open_fd_count() -> usize {
            fs::read_dir("/proc/self/fd").unwrap().count()
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
            let invoked = Rc::new(Cell::new(false));
            let hook_invoked = Rc::clone(&invoked);
            expect_replacement_error(
                &fixture,
                CapturePhase::AfterMetadataCapture,
                mutation_at(
                    CapturePhase::AfterMetadataCapture,
                    move || replace_file_with_same_bytes(&path),
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
                let fd_before = open_fd_count();
                let invoked = Rc::new(Cell::new(false));
                let hook_invoked = Rc::clone(&invoked);
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
                assert!(invoked.get(), "missing failure hook {phase:?}");
                assert_eq!(result.err(), Some(DailyEvidenceError::Io));
                assert_eq!(tree_state(fixture._temp.path()), tree_before);
                assert_eq!(open_fd_count(), fd_before);
                lock_exclusive(&fixture);
            }

            let fixture = make_fixture();
            let fd_before = open_fd_count();
            let snapshot = scan(&fixture).unwrap();
            assert_eq!(
                FileExt::try_lock_exclusive(
                    &OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(lock_path(&fixture))
                        .unwrap()
                )
                .err()
                .map(io_error),
                Some(DailyEvidenceError::LockBusy)
            );
            drop(snapshot);
            assert_eq!(open_fd_count(), fd_before);
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
        BeforeFinalBytes,
        BeforeFinalNames,
    }

    #[cfg(not(test))]
    fn hit_phase(_phase: CapturePhase) -> Result<(), DailyEvidenceError> {
        Ok(())
    }
}
