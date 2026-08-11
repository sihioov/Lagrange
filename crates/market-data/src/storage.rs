//! Immutable Raw storage zone (FR-DATA-001, System Design 6.3/7.1).
//!
//! Layout (root = the `data/` directory):
//!
//! ```text
//! data/raw/provider=krx/market=kr/date=2020-01-31/batch=<id>/  <- one dir per delivery
//!     <provider-file>...      exact provider bytes (create_new: never overwritten)
//!     batch.json              the ManifestEntry for this batch (pretty JSON)
//! data/raw/manifests/provider=krx/market=kr/manifest.jsonl    <- append-only, one row per batch
//! ```
//!
//! Invariants enforced here:
//! - **Never overwrite Raw**: every delivery is a NEW batch dir; file writes use
//!   exclusive `create_new`, so an existing file is never clobbered.
//! - **Crash-durable batches**: evidence is synced before atomic `batch.json`
//!   publication. Pre-publication failures clean up; once final metadata is
//!   visible, any later failure preserves an indeterminate batch for recovery.
//!   Orphan discovery re-syncs its files and directory hierarchy before exposure.
//! - **Path traversal rejection**: provider file names must be plain names.
//! - **Append-only manifest**: JSONL writes are serialized under a file lock;
//!   reads verify stored bytes against the recorded content hash (tamper detection).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::contract::{FetchMode, RawEnvelope, ResponseKind, StoredFile, date_partition};

/// Per-file record inside a manifest row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub kind: ResponseKind,
    pub file_name: String,
    pub content_hash: ContentHash,
    pub size_bytes: u64,
    pub request: crate::contract::RequestMetadata,
}

/// One append-only manifest row: exactly one per ingestion batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub batch_id: BatchId,
    pub provider: String,
    pub market: String,
    pub date: TradingDate,
    pub retrieved_at: UtcTimestamp,
    pub mode: FetchMode,
    /// Reference to the governing licensed-data contract (Todo 5 entitlement),
    /// e.g. `vault://krx-entitlements/ent_krx_2026_0001.pdf`; `None` when no
    /// entitlement record covers the dataset on the batch date.
    pub entitlement_reference: Option<String>,
    pub files: Vec<FileEntry>,
}

impl ManifestEntry {
    /// The batch's own JSON metadata file inside its dir.
    pub fn batch_json_file_name(&self) -> &'static str {
        "batch.json"
    }
}

/// A typed failure from the raw zone.
#[derive(Debug)]
pub enum StoreError {
    /// A file already exists at the target path — Raw must never be overwritten.
    FileExists { path: String },
    /// The provider file name is not a plain name (traversal, separators, ...).
    UnsafeFileName { file_name: String, reason: String },
    /// Provider or market would be unsafe as a filesystem path component.
    UnsafeScope {
        component: String,
        value: String,
        reason: String,
    },
    /// An embedded manifest scope differs from the requested trusted scope.
    ScopeMismatch {
        expected_provider: String,
        expected_market: String,
        actual_provider: String,
        actual_market: String,
    },
    /// A canonicalized path leaves the intended immutable batch directory.
    UnsafePath { path: String, reason: String },
    /// Read-back bytes differ from the content hash recorded in immutable Raw.
    ContentHashMismatch {
        path: String,
        recorded: String,
        actual: String,
    },
    /// A complete manifest record is malformed. Only an unterminated final
    /// record is tolerated as a crash tail.
    CorruptManifest {
        path: String,
        line: usize,
        source: serde_json::Error,
    },
    /// A durable orphan's `batch.json` is malformed.
    CorruptBatchMetadata {
        path: String,
        source: serde_json::Error,
    },
    /// A durable orphan's location and metadata disagree.
    InvalidBatchMetadata { path: String, reason: String },
    /// Immutable evidence named by committed metadata is missing.
    MissingEvidence {
        path: String,
        source: std::io::Error,
    },
    /// Serialization failed before bytes could be committed.
    Serialization {
        context: String,
        source: serde_json::Error,
    },
    /// Removal of a pre-visible partial batch failed; both causes are retained.
    CleanupFailed {
        path: String,
        original: Box<StoreError>,
        cleanup: std::io::Error,
    },
    /// Final metadata is visible, but a later durability or manifest step failed.
    /// Recovery must re-sync and reuse this exact entry rather than recapture.
    IndeterminateBatchCommit {
        entry: Box<ManifestEntry>,
        source: Box<StoreError>,
    },
    /// The manifest already contains different metadata for this batch id.
    ManifestConflict { path: String, batch_id: BatchId },
    /// Genuine filesystem create/read/write/sync/lock failure.
    Io {
        context: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileExists { path } => {
                write!(f, "raw file already exists (immutable zone): {path}")
            }
            Self::UnsafeFileName { file_name, reason } => {
                write!(f, "unsafe raw file name {file_name:?}: {reason}")
            }
            Self::UnsafeScope {
                component,
                value,
                reason,
            } => write!(f, "unsafe raw {component} scope {value:?}: {reason}"),
            Self::ScopeMismatch {
                expected_provider,
                expected_market,
                actual_provider,
                actual_market,
            } => write!(
                f,
                "raw scope mismatch: requested {expected_provider}/{expected_market}, entry {actual_provider}/{actual_market}"
            ),
            Self::UnsafePath { path, reason } => write!(f, "unsafe raw path {path:?}: {reason}"),
            Self::ContentHashMismatch {
                path,
                recorded,
                actual,
            } => write!(
                f,
                "raw content hash mismatch at {path}: recorded {recorded} != read {actual}"
            ),
            Self::CorruptManifest { path, line, source } => write!(
                f,
                "corrupt Raw manifest record at {path} line {line}: {source}"
            ),
            Self::CorruptBatchMetadata { path, source } => {
                write!(f, "corrupt Raw batch metadata at {path}: {source}")
            }
            Self::InvalidBatchMetadata { path, reason } => {
                write!(f, "invalid Raw batch metadata at {path}: {reason}")
            }
            Self::MissingEvidence { path, .. } => {
                write!(f, "immutable Raw evidence is missing at {path}")
            }
            Self::Serialization { context, source } => {
                write!(f, "Raw serialization failure ({context}): {source}")
            }
            Self::CleanupFailed {
                path,
                original,
                cleanup,
            } => write!(
                f,
                "Raw partial batch cleanup failed at {path} after {original}: {cleanup}"
            ),
            Self::IndeterminateBatchCommit { entry, source } => write!(
                f,
                "Raw batch {} is visible but commit durability is indeterminate: {source}",
                entry.batch_id
            ),
            Self::ManifestConflict { path, batch_id } => write!(
                f,
                "Raw manifest {path} already contains conflicting metadata for batch {batch_id}"
            ),
            Self::Io { context, source } => {
                write!(f, "raw store io failure ({context}): {source}")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CorruptManifest { source, .. }
            | Self::CorruptBatchMetadata { source, .. }
            | Self::Serialization { source, .. } => Some(source),
            Self::MissingEvidence { source, .. } | Self::Io { source, .. } => Some(source),
            Self::CleanupFailed { original, .. } => Some(original.as_ref()),
            Self::IndeterminateBatchCommit { source, .. } => Some(source.as_ref()),
            Self::FileExists { .. }
            | Self::UnsafeFileName { .. }
            | Self::UnsafeScope { .. }
            | Self::ScopeMismatch { .. }
            | Self::UnsafePath { .. }
            | Self::ContentHashMismatch { .. }
            | Self::InvalidBatchMetadata { .. }
            | Self::ManifestConflict { .. } => None,
        }
    }
}

impl StoreError {
    pub fn batch_id(&self) -> Option<BatchId> {
        match self {
            Self::IndeterminateBatchCommit { entry, .. } => Some(entry.batch_id),
            _ => None,
        }
    }
}

/// Everything the raw zone needs to persist one delivery as a new batch.
#[derive(Debug, Clone)]
pub struct BatchSpec<'a> {
    pub provider: &'a str,
    pub market: &'a str,
    pub date: &'a TradingDate,
    pub batch_id: BatchId,
    pub entitlement_reference: Option<&'a str>,
    pub mode: FetchMode,
}

/// The immutable raw zone rooted at `data/raw/`.
#[derive(Debug, Clone)]
pub struct RawStore {
    root: PathBuf,
}

trait BatchCommitOps: Send + Sync {
    fn before_metadata_publish(&self) -> Result<(), StoreError> {
        Ok(())
    }

    fn cleanup_batch(&self, batch_dir: &Path) -> std::io::Result<()> {
        fs::remove_dir_all(batch_dir)
    }

    fn sync_published_metadata(&self, file: &File) -> Result<(), StoreError> {
        file.sync_all()
            .map_err(|error| io_err("sync published batch.json", error))
    }

    fn after_metadata_visible(&self) {}

    fn sync_batch_directories(&self, batch_dir: &Path) -> Result<(), StoreError> {
        sync_batch_directories(batch_dir)
    }

    fn before_commit_lock(&self) {}
}

#[derive(Debug)]
struct SystemBatchCommitOps;

impl BatchCommitOps for SystemBatchCommitOps {}

trait ManifestReadOps: Send + Sync {
    fn before_shared_lock(&self) {}

    fn after_shared_lock(&self) {}

    fn sync_file(&self, path: &Path) -> Result<(), StoreError> {
        sync_file(path, &format!("re-sync orphan file {}", path.display()))
    }

    fn sync_batch_directories(&self, batch_dir: &Path) -> Result<(), StoreError> {
        sync_batch_directories(batch_dir)
    }

    fn before_manifest_append(&self, _entry: &ManifestEntry) -> Result<(), StoreError> {
        Ok(())
    }

    fn sync_manifest_file(&self, file: &File) -> Result<(), StoreError> {
        file.sync_all()
            .map_err(|error| io_err("sync manifest", error))
    }

    fn sync_manifest_directories(&self, manifest_parent: &Path) -> Result<(), StoreError> {
        sync_manifest_directories(manifest_parent)
    }
}

#[derive(Debug)]
struct SystemManifestReadOps;

impl ManifestReadOps for SystemManifestReadOps {}

#[derive(Debug)]
struct LockedManifest {
    path: PathBuf,
    file: File,
    entries: BTreeMap<BatchId, ManifestEntry>,
    order: Vec<BatchId>,
}

impl RawStore {
    /// `root` is the `data/` directory; raw files live under `root/raw/...`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The `data/` root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `data/raw/provider=<p>/market=<m>`
    pub fn provider_dir(&self, provider: &str, market: &str) -> PathBuf {
        self.root
            .join("raw")
            .join(format!("provider={provider}"))
            .join(format!("market={market}"))
    }

    /// `data/raw/provider=<p>/market=<m>/date=<YYYY-MM-DD>/batch=<id>`
    pub fn batch_dir(
        &self,
        provider: &str,
        market: &str,
        date: &TradingDate,
        batch_id: &BatchId,
    ) -> PathBuf {
        self.provider_dir(provider, market)
            .join(date_partition(date))
            .join(format!("batch={batch_id}"))
    }

    /// `data/raw/manifests/provider=<p>/market=<m>/manifest.jsonl`
    pub fn manifest_path(&self, provider: &str, market: &str) -> PathBuf {
        self.root
            .join("raw")
            .join("manifests")
            .join(format!("provider={provider}"))
            .join(format!("market={market}"))
            .join("manifest.jsonl")
    }

    fn commit_lock_path(&self, provider: &str, market: &str) -> PathBuf {
        self.manifest_path(provider, market)
            .with_file_name("commit.lock")
    }

    fn open_commit_lock(&self, provider: &str, market: &str) -> Result<File, StoreError> {
        let path = self.commit_lock_path(provider, market);
        let parent = path.parent().ok_or_else(|| {
            io_err(
                "commit lock parent",
                std::io::Error::other(format!("{} has no parent", path.display())),
            )
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| io_err("create commit lock directory", error))?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| io_err(&format!("open commit lock {}", path.display()), error))
    }

    /// Persists one delivery as a new immutable batch and appends its manifest row.
    ///
    /// Failures before final metadata publication remove the partial batch.
    /// Once `batch.json` is visible, any later failure preserves the batch with
    /// its exact identity so orphan discovery can re-sync it before exposure.
    pub fn store_batch(
        &self,
        spec: &BatchSpec<'_>,
        envelopes: &[RawEnvelope],
    ) -> Result<ManifestEntry, StoreError> {
        self.store_batch_with_commit_ops(spec, envelopes, &SystemBatchCommitOps)
    }

    fn store_batch_with_commit_ops<O: BatchCommitOps + ?Sized>(
        &self,
        spec: &BatchSpec<'_>,
        envelopes: &[RawEnvelope],
        operations: &O,
    ) -> Result<ManifestEntry, StoreError> {
        let BatchSpec {
            provider,
            market,
            date,
            batch_id,
            entitlement_reference,
            mode,
        } = *spec;
        validate_scope(provider, market)?;
        for env in envelopes {
            validate_file_entry_name(&env.file_name)?;
        }

        let dir = self.batch_dir(provider, market, date, &batch_id);
        if dir.exists() {
            return Err(StoreError::FileExists {
                path: dir.display().to_string(),
            });
        }
        let parent = dir.parent().ok_or_else(|| {
            io_err(
                "batch-dir-parent",
                std::io::Error::other(format!("{} has no parent", dir.display())),
            )
        })?;
        fs::create_dir_all(parent).map_err(|e| io_err("create date partition", e))?;
        fs::create_dir(&dir)
            .map_err(|e| io_err(&format!("create batch dir {}", dir.display()), e))?;

        let cleanup = |original: StoreError| -> StoreError {
            match operations.cleanup_batch(&dir) {
                Ok(()) => original,
                Err(cleanup) => StoreError::CleanupFailed {
                    path: dir.display().to_string(),
                    original: Box::new(original),
                    cleanup,
                },
            }
        };
        self.canonical_batch_dir(provider, market, date, &batch_id)
            .map_err(cleanup)?;

        for env in envelopes {
            let path = dir.join(&env.file_name);
            let mut out = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(cleanup(StoreError::FileExists {
                        path: path.display().to_string(),
                    }));
                }
                Err(e) => return Err(cleanup(io_err(&format!("create {}", path.display()), e))),
            };
            out.write_all(&env.bytes)
                .map_err(|e| cleanup(io_err(&format!("write {}", path.display()), e)))?;
            out.sync_all()
                .map_err(|e| cleanup(io_err(&format!("sync {}", path.display()), e)))?;
        }

        let entry = ManifestEntry {
            batch_id,
            provider: provider.to_owned(),
            market: market.to_owned(),
            date: *date,
            retrieved_at: envelopes
                .first()
                .map(|e| e.retrieved_at)
                .unwrap_or(UtcTimestamp::now()),
            mode,
            entitlement_reference: entitlement_reference.map(str::to_owned),
            files: envelopes
                .iter()
                .map(|e| FileEntry {
                    kind: e.kind,
                    file_name: e.file_name.clone(),
                    content_hash: e.content_hash.clone(),
                    size_bytes: e.bytes.len() as u64,
                    request: e.request.clone(),
                })
                .collect(),
        };

        let prepared = prepare_batch_metadata(&dir, &entry).map_err(cleanup)?;
        operations.before_metadata_publish().map_err(cleanup)?;
        let commit_lock = self.open_commit_lock(provider, market).map_err(cleanup)?;
        operations.before_commit_lock();
        FileExt::lock_exclusive(&commit_lock)
            .map_err(|error| cleanup(io_err("lock Raw commit", error)))?;
        let commit_result = (|| {
            let published =
                publish_batch_metadata(prepared, &dir.join(entry.batch_json_file_name()))
                    .map_err(cleanup)?;
            operations
                .sync_published_metadata(&published)
                .map_err(|source| StoreError::IndeterminateBatchCommit {
                    entry: Box::new(entry.clone()),
                    source: Box::new(source),
                })?;
            operations.after_metadata_visible();
            operations.sync_batch_directories(&dir).map_err(|source| {
                StoreError::IndeterminateBatchCommit {
                    entry: Box::new(entry.clone()),
                    source: Box::new(source),
                }
            })?;
            self.append_manifest_locked(provider, market, &entry)
                .map_err(|source| StoreError::IndeterminateBatchCommit {
                    entry: Box::new(entry.clone()),
                    source: Box::new(source),
                })?;
            Ok(entry)
        })();
        let unlock_result =
            FileExt::unlock(&commit_lock).map_err(|error| io_err("unlock Raw commit", error));
        match commit_result {
            Err(error) => {
                let _ = unlock_result;
                Err(error)
            }
            Ok(entry) => {
                unlock_result.map_err(|source| StoreError::IndeterminateBatchCommit {
                    entry: Box::new(entry.clone()),
                    source: Box::new(source),
                })?;
                Ok(entry)
            }
        }
    }

    /// Appends one manifest row (JSONL). Never rewrites existing rows.
    pub fn append_manifest(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
    ) -> Result<(), StoreError> {
        validate_scope(provider, market)?;
        validate_entry_scope(provider, market, entry)?;
        validate_manifest_file_names(entry)?;
        let commit_lock = self.open_commit_lock(provider, market)?;
        FileExt::lock_exclusive(&commit_lock).map_err(|error| io_err("lock Raw commit", error))?;
        let result = self.append_manifest_locked(provider, market, entry);
        let unlock =
            FileExt::unlock(&commit_lock).map_err(|error| io_err("unlock Raw commit", error));
        result?;
        unlock
    }

    fn append_manifest_locked(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
    ) -> Result<(), StoreError> {
        self.append_manifest_locked_with_ops(provider, market, entry, &SystemManifestReadOps)
    }

    fn append_manifest_locked_with_ops<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
        operations: &O,
    ) -> Result<(), StoreError> {
        validate_entry_scope(provider, market, entry)?;
        validate_manifest_file_names(entry)?;
        let path = self.manifest_path(provider, market);
        let parent = path.parent().ok_or_else(|| {
            io_err(
                "manifest parent",
                std::io::Error::other(format!("{} has no parent", path.display())),
            )
        })?;
        fs::create_dir_all(parent).map_err(|e| io_err("create manifest dir", e))?;
        let mut manifest = self.open_validated_manifest_locked(provider, market, &path)?;
        self.append_manifest_entry_already_locked(&mut manifest, entry, operations)?;
        operations.sync_manifest_file(&manifest.file)?;
        operations.sync_manifest_directories(parent)
    }

    fn open_validated_manifest_locked(
        &self,
        provider: &str,
        market: &str,
        path: &Path,
    ) -> Result<LockedManifest, StoreError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| io_err("read manifest before append", e))?;
        let mut entries = BTreeMap::new();
        let mut order = Vec::new();
        parse_manifest_records(provider, market, path, &bytes, &mut entries, &mut order)?;
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            let tail_start = bytes
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |position| position + 1);
            match serde_json::from_slice::<ManifestEntry>(&bytes[tail_start..]) {
                Ok(tail_entry) => {
                    validate_entry_scope(provider, market, &tail_entry)?;
                    validate_manifest_file_names(&tail_entry)?;
                    file.seek(SeekFrom::End(0))
                        .map_err(|e| io_err("seek unterminated manifest end", e))?;
                    file.write_all(b"\n")
                        .map_err(|e| io_err("terminate complete manifest record", e))?;
                }
                Err(source) if source.is_eof() => {
                    file.set_len(tail_start as u64)
                        .map_err(|e| io_err("trim truncated manifest tail", e))?;
                }
                Err(source) => {
                    return Err(StoreError::CorruptManifest {
                        path: path.display().to_string(),
                        line: bytes[..tail_start]
                            .iter()
                            .filter(|byte| **byte == b'\n')
                            .count()
                            + 1,
                        source,
                    });
                }
            }
        }

        file.seek(SeekFrom::End(0))
            .map_err(|e| io_err("seek manifest end", e))?;
        Ok(LockedManifest {
            path: path.to_owned(),
            file,
            entries,
            order,
        })
    }

    fn append_manifest_entry_already_locked<O: ManifestReadOps + ?Sized>(
        &self,
        manifest: &mut LockedManifest,
        entry: &ManifestEntry,
        operations: &O,
    ) -> Result<(), StoreError> {
        if manifest.entries.get(&entry.batch_id) == Some(entry) {
            return Ok(());
        }
        if manifest.entries.contains_key(&entry.batch_id) {
            return Err(StoreError::ManifestConflict {
                path: manifest.path.display().to_string(),
                batch_id: entry.batch_id,
            });
        }
        operations.before_manifest_append(entry)?;
        let mut line = serde_json::to_vec(entry).map_err(|source| StoreError::Serialization {
            context: "manifest entry serialize".to_owned(),
            source,
        })?;
        line.push(b'\n');
        manifest
            .file
            .write_all(&line)
            .map_err(|e| io_err("append manifest", e))?;
        manifest.entries.insert(entry.batch_id, entry.clone());
        manifest.order.push(entry.batch_id);
        Ok(())
    }

    /// All committed manifest rows plus durable orphan batches, oldest first.
    /// An unterminated final manifest fragment is ignored as a crash tail;
    /// malformed complete or middle records are permanent corruption.
    pub fn read_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        self.read_manifest_with_ops(provider, market, &SystemManifestReadOps)
    }

    /// Returns recovery's durable append-order snapshot.
    ///
    /// Unlike [`Self::read_manifest`], this takes the commit lock exclusively
    /// and appends every verified orphan to the JSONL manifest before exposing
    /// it. A returned batch ID therefore identifies an immutable manifest line,
    /// never a synthetic orphan suffix position.
    pub fn read_reconciled_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        self.read_reconciled_manifest_with_ops(provider, market, &SystemManifestReadOps)
    }

    fn read_reconciled_manifest_with_ops<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        operations: &O,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        validate_scope(provider, market)?;
        let commit_lock = self.open_commit_lock(provider, market)?;
        FileExt::lock_exclusive(&commit_lock)
            .map_err(|error| io_err("lock Raw commit for manifest reconciliation", error))?;
        let result = self.read_reconciled_manifest_locked(provider, market, operations);
        let unlock = FileExt::unlock(&commit_lock)
            .map_err(|error| io_err("unlock reconciled Raw commit", error));
        let entries = result?;
        unlock?;
        Ok(entries)
    }

    fn read_reconciled_manifest_locked<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        operations: &O,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        let path = self.manifest_path(provider, market);
        let parent = path.parent().ok_or_else(|| {
            io_err(
                "manifest parent",
                std::io::Error::other(format!("{} has no parent", path.display())),
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| io_err("create manifest dir", error))?;
        let mut manifest = self.open_validated_manifest_locked(provider, market, &path)?;

        self.discover_orphan_batches(provider, market, &mut manifest.entries, operations)?;
        let manifest_ids: BTreeSet<_> = manifest.order.iter().copied().collect();
        let mut orphan_ids: Vec<_> = manifest
            .entries
            .keys()
            .copied()
            .filter(|batch_id| !manifest_ids.contains(batch_id))
            .collect();
        orphan_ids.sort_by(|left, right| {
            manifest.entries[left]
                .retrieved_at
                .cmp(&manifest.entries[right].retrieved_at)
                .then_with(|| left.cmp(right))
        });
        for batch_id in orphan_ids {
            let entry = manifest
                .entries
                .remove(&batch_id)
                .expect("discovered orphan exists");
            self.append_manifest_entry_already_locked(&mut manifest, &entry, operations)?;
        }
        operations.sync_manifest_file(&manifest.file)?;
        operations.sync_manifest_directories(parent)?;

        Ok(manifest
            .order
            .into_iter()
            .map(|batch_id| {
                manifest
                    .entries
                    .remove(&batch_id)
                    .expect("durable manifest batch exists")
            })
            .collect())
    }

    fn read_manifest_with_ops<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        operations: &O,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        validate_scope(provider, market)?;
        let commit_lock = self.open_commit_lock(provider, market)?;
        operations.before_shared_lock();
        FileExt::lock_shared(&commit_lock).map_err(|error| io_err("lock Raw commit", error))?;
        operations.after_shared_lock();
        let result = self.read_manifest_locked(provider, market, operations);
        let unlock =
            FileExt::unlock(&commit_lock).map_err(|error| io_err("unlock Raw commit", error));
        let entries = result?;
        unlock?;
        Ok(entries)
    }

    fn read_manifest_locked<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        operations: &O,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        let path = self.manifest_path(provider, market);
        let mut entries = BTreeMap::new();
        let mut manifest_order = Vec::new();
        if path.exists() {
            let mut f = File::open(&path)
                .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes)
                .map_err(|e| io_err("read manifest", e))?;
            parse_manifest_records(
                provider,
                market,
                &path,
                &bytes,
                &mut entries,
                &mut manifest_order,
            )?;
        }
        self.discover_orphan_batches(provider, market, &mut entries, operations)?;
        let manifest_ids: BTreeSet<_> = manifest_order.iter().copied().collect();
        let mut orphan_ids: Vec<_> = entries
            .keys()
            .copied()
            .filter(|batch_id| !manifest_ids.contains(batch_id))
            .collect();
        orphan_ids.sort_by(|left, right| {
            entries[left]
                .retrieved_at
                .cmp(&entries[right].retrieved_at)
                .then_with(|| left.cmp(right))
        });
        manifest_order.extend(orphan_ids);
        Ok(manifest_order
            .into_iter()
            .map(|batch_id| entries.remove(&batch_id).expect("ordered batch exists"))
            .collect())
    }

    fn discover_orphan_batches<O: ManifestReadOps + ?Sized>(
        &self,
        provider: &str,
        market: &str,
        entries: &mut BTreeMap<BatchId, ManifestEntry>,
        operations: &O,
    ) -> Result<(), StoreError> {
        let provider_dir = self.provider_dir(provider, market);
        if !provider_dir.exists() {
            return Ok(());
        }
        for date_dir in sorted_directories(&provider_dir)? {
            let date_path = date_dir.path();
            let date_name = date_dir.file_name().to_string_lossy().into_owned();
            let Some(date_text) = date_name.strip_prefix("date=") else {
                continue;
            };
            let date = TradingDate::parse(date_text).map_err(|error| {
                StoreError::InvalidBatchMetadata {
                    path: date_path.display().to_string(),
                    reason: error.to_string(),
                }
            })?;
            for batch_dir in sorted_directories(&date_path)? {
                let batch_path = batch_dir.path();
                let batch_name = batch_dir.file_name().to_string_lossy().into_owned();
                let Some(batch_text) = batch_name.strip_prefix("batch=") else {
                    continue;
                };
                let batch_id = batch_text.parse::<BatchId>().map_err(|error| {
                    StoreError::InvalidBatchMetadata {
                        path: batch_path.display().to_string(),
                        reason: error.to_string(),
                    }
                })?;
                let metadata_path = batch_path.join("batch.json");
                if !metadata_path.exists() {
                    continue;
                }
                let (raw_root, canonical_batch) =
                    self.canonical_batch_dir(provider, market, &date, &batch_id)?;
                let canonical_metadata = fs::canonicalize(&metadata_path).map_err(|source| {
                    if source.kind() == std::io::ErrorKind::NotFound {
                        StoreError::MissingEvidence {
                            path: metadata_path.display().to_string(),
                            source,
                        }
                    } else {
                        io_err(&format!("canonicalize {}", metadata_path.display()), source)
                    }
                })?;
                if canonical_metadata != canonical_batch.join("batch.json") {
                    return Err(StoreError::UnsafePath {
                        path: canonical_metadata.display().to_string(),
                        reason: "batch.json must be a direct file in its canonical batch directory"
                            .to_owned(),
                    });
                }
                let metadata = fs::read(&canonical_metadata).map_err(|source| {
                    if source.kind() == std::io::ErrorKind::NotFound {
                        StoreError::MissingEvidence {
                            path: metadata_path.display().to_string(),
                            source,
                        }
                    } else {
                        io_err(&format!("read {}", metadata_path.display()), source)
                    }
                })?;
                let entry: ManifestEntry = serde_json::from_slice(&metadata).map_err(|source| {
                    StoreError::CorruptBatchMetadata {
                        path: metadata_path.display().to_string(),
                        source,
                    }
                })?;
                validate_entry_scope(provider, market, &entry)?;
                validate_manifest_file_names(&entry)?;
                if entry.date != date || entry.batch_id != batch_id {
                    return Err(StoreError::InvalidBatchMetadata {
                        path: metadata_path.display().to_string(),
                        reason: format!(
                            "directory identifies {date}/{batch_id}, metadata identifies {}/{}",
                            entry.date, entry.batch_id
                        ),
                    });
                }
                if !entries.contains_key(&entry.batch_id) {
                    self.resync_orphan_batch(
                        &entry,
                        &batch_path,
                        &raw_root,
                        &canonical_batch,
                        &canonical_metadata,
                        operations,
                    )?;
                }
                merge_manifest_entry(entries, entry, &metadata_path)?;
            }
        }
        Ok(())
    }

    fn resync_orphan_batch<O: ManifestReadOps + ?Sized>(
        &self,
        entry: &ManifestEntry,
        batch_path: &Path,
        raw_root: &Path,
        canonical_batch: &Path,
        canonical_metadata: &Path,
        operations: &O,
    ) -> Result<(), StoreError> {
        for file in &entry.files {
            let path = batch_path.join(&file.file_name);
            let canonical_file = fs::canonicalize(&path).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    StoreError::MissingEvidence {
                        path: path.display().to_string(),
                        source,
                    }
                } else {
                    io_err(&format!("canonicalize {}", path.display()), source)
                }
            })?;
            let expected = canonical_batch.join(&file.file_name);
            if !canonical_file.starts_with(raw_root)
                || !canonical_file.starts_with(canonical_batch)
                || canonical_file != expected
            {
                return Err(StoreError::UnsafePath {
                    path: canonical_file.display().to_string(),
                    reason: format!(
                        "orphan evidence must be a direct file inside {} below raw root {}",
                        canonical_batch.display(),
                        raw_root.display()
                    ),
                });
            }
            operations.sync_file(&canonical_file)?;
        }
        operations.sync_file(canonical_metadata)?;
        operations.sync_batch_directories(canonical_batch)
    }

    /// Reads a batch back and verifies every stored file against its recorded
    /// content hash (tamper detection).
    pub fn read_batch_bytes(
        &self,
        provider: &str,
        market: &str,
        entry: &ManifestEntry,
    ) -> Result<Vec<StoredFile>, StoreError> {
        validate_scope(provider, market)?;
        validate_entry_scope(provider, market, entry)?;
        validate_manifest_file_names(entry)?;
        let dir = self.batch_dir(provider, market, &entry.date, &entry.batch_id);
        let (raw_root, canonical_dir) =
            self.canonical_batch_dir(provider, market, &entry.date, &entry.batch_id)?;
        let mut out = Vec::with_capacity(entry.files.len());
        for file in &entry.files {
            let path = dir.join(&file.file_name);
            let storage_path = fs::canonicalize(&path).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    StoreError::MissingEvidence {
                        path: path.display().to_string(),
                        source,
                    }
                } else {
                    io_err(&format!("canonicalize {}", path.display()), source)
                }
            })?;
            let expected_path = canonical_dir.join(&file.file_name);
            if !storage_path.starts_with(&raw_root)
                || !storage_path.starts_with(&canonical_dir)
                || storage_path != expected_path
            {
                return Err(StoreError::UnsafePath {
                    path: storage_path.display().to_string(),
                    reason: format!(
                        "must be the direct file inside batch {} below raw root {}",
                        canonical_dir.display(),
                        raw_root.display()
                    ),
                });
            }
            let bytes = fs::read(&storage_path).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    StoreError::MissingEvidence {
                        path: path.display().to_string(),
                        source,
                    }
                } else {
                    io_err(&format!("read {}", path.display()), source)
                }
            })?;
            let actual = ContentHash::from_bytes(&bytes);
            if actual != file.content_hash {
                return Err(StoreError::ContentHashMismatch {
                    path: storage_path.display().to_string(),
                    recorded: file.content_hash.to_string(),
                    actual: actual.to_string(),
                });
            }
            out.push(StoredFile {
                file_name: file.file_name.clone(),
                bytes,
                storage_path,
            });
        }
        Ok(out)
    }

    /// The batch ids stored under one date partition (QA/diff channel).
    pub fn batch_ids(
        &self,
        provider: &str,
        market: &str,
        date: &TradingDate,
    ) -> Result<Vec<BatchId>, StoreError> {
        validate_scope(provider, market)?;
        let dir = self
            .provider_dir(provider, market)
            .join(date_partition(date));
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in
            fs::read_dir(&dir).map_err(|e| io_err(&format!("list {}", dir.display()), e))?
        {
            let entry = entry.map_err(|e| io_err("read_dir entry", e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("batch=") else {
                continue;
            };
            match rest.parse::<BatchId>() {
                Ok(id) => ids.push(id),
                Err(_) => continue,
            }
        }
        Ok(ids)
    }

    fn canonical_raw_root(&self) -> Result<PathBuf, StoreError> {
        let raw_root = self.root.join("raw");
        fs::canonicalize(&raw_root).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                StoreError::MissingEvidence {
                    path: raw_root.display().to_string(),
                    source,
                }
            } else {
                io_err(
                    &format!("canonicalize raw root {}", raw_root.display()),
                    source,
                )
            }
        })
    }

    fn canonical_batch_dir(
        &self,
        provider: &str,
        market: &str,
        date: &TradingDate,
        batch_id: &BatchId,
    ) -> Result<(PathBuf, PathBuf), StoreError> {
        let raw_root = self.canonical_raw_root()?;
        let components = [
            format!("provider={provider}"),
            format!("market={market}"),
            date_partition(date),
            format!("batch={batch_id}"),
        ];
        let mut lexical = self.root.join("raw");
        let mut expected = raw_root.clone();
        for component in components {
            lexical.push(&component);
            expected.push(&component);
            let actual = fs::canonicalize(&lexical).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    StoreError::MissingEvidence {
                        path: lexical.display().to_string(),
                        source,
                    }
                } else {
                    io_err(&format!("canonicalize {}", lexical.display()), source)
                }
            })?;
            if !actual.starts_with(&raw_root) || actual != expected {
                return Err(StoreError::UnsafePath {
                    path: actual.display().to_string(),
                    reason: format!(
                        "unexpected symlink or redirect below raw root {}",
                        raw_root.display()
                    ),
                });
            }
        }
        Ok((raw_root, expected))
    }
}

fn prepare_batch_metadata(
    batch_dir: &Path,
    entry: &ManifestEntry,
) -> Result<tempfile::NamedTempFile, StoreError> {
    let json = serde_json::to_vec_pretty(entry).map_err(|source| StoreError::Serialization {
        context: "batch.json serialize".to_owned(),
        source,
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(batch_dir)
        .map_err(|error| io_err("create temporary batch metadata", error))?;
    temporary
        .write_all(&json)
        .map_err(|error| io_err("write temporary batch metadata", error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| io_err("sync temporary batch metadata", error))?;
    Ok(temporary)
}

fn publish_batch_metadata(
    temporary: tempfile::NamedTempFile,
    final_path: &Path,
) -> Result<File, StoreError> {
    publish_batch_metadata_platform(temporary, final_path)
}

#[cfg(not(windows))]
fn publish_batch_metadata_platform(
    temporary: tempfile::NamedTempFile,
    final_path: &Path,
) -> Result<File, StoreError> {
    let file = temporary.persist_noclobber(final_path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            StoreError::FileExists {
                path: final_path.display().to_string(),
            }
        } else {
            io_err("publish batch.json", error.error)
        }
    })?;
    Ok(file)
}

#[cfg(windows)]
fn publish_batch_metadata_platform(
    temporary: tempfile::NamedTempFile,
    final_path: &Path,
) -> Result<File, StoreError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let file = temporary
        .reopen()
        .map_err(|error| io_err("open temporary batch.json for final sync", error))?;
    let temporary_path = temporary.into_temp_path();
    let source: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both paths are stable, NUL-terminated UTF-16 buffers for the
    // duration of the call. Omitting MOVEFILE_REPLACE_EXISTING preserves Raw.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
            StoreError::FileExists {
                path: final_path.display().to_string(),
            }
        } else {
            io_err("publish batch.json with write-through rename", error)
        });
    }

    Ok(file)
}

fn parse_manifest_records(
    provider: &str,
    market: &str,
    path: &Path,
    bytes: &[u8],
    entries: &mut BTreeMap<BatchId, ManifestEntry>,
    manifest_order: &mut Vec<BatchId>,
) -> Result<(), StoreError> {
    let terminated = bytes.ends_with(b"\n");
    let records: Vec<_> = bytes.split(|byte| *byte == b'\n').collect();
    for (index, record) in records.iter().enumerate() {
        if record.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let final_unterminated = index + 1 == records.len() && !terminated;
        let entry = match serde_json::from_slice::<ManifestEntry>(record) {
            Ok(entry) => entry,
            Err(source) if final_unterminated && source.is_eof() => continue,
            Err(source) => {
                return Err(StoreError::CorruptManifest {
                    path: path.display().to_string(),
                    line: index + 1,
                    source,
                });
            }
        };
        validate_entry_scope(provider, market, &entry)?;
        validate_manifest_file_names(&entry)?;
        let is_new = !entries.contains_key(&entry.batch_id);
        let batch_id = entry.batch_id;
        merge_manifest_entry(entries, entry, path)?;
        if is_new {
            manifest_order.push(batch_id);
        }
    }
    Ok(())
}

fn merge_manifest_entry(
    entries: &mut BTreeMap<BatchId, ManifestEntry>,
    entry: ManifestEntry,
    source_path: &Path,
) -> Result<(), StoreError> {
    if let Some(existing) = entries.get(&entry.batch_id) {
        if existing == &entry {
            return Ok(());
        }
        return Err(StoreError::InvalidBatchMetadata {
            path: source_path.display().to_string(),
            reason: format!("batch {} has conflicting metadata", entry.batch_id),
        });
    }
    entries.insert(entry.batch_id, entry);
    Ok(())
}

fn sorted_directories(path: &Path) -> Result<Vec<fs::DirEntry>, StoreError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| io_err(&format!("list {}", path.display()), e))? {
        let entry = entry.map_err(|e| io_err("read directory entry", e))?;
        if entry.path().is_dir() {
            entries.push(entry);
        }
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn sync_file(path: &Path, context: &str) -> Result<(), StoreError> {
    open_file_for_sync(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                StoreError::MissingEvidence {
                    path: path.display().to_string(),
                    source,
                }
            } else {
                io_err(context, source)
            }
        })
}

// Linux permits fsync(2) on an O_RDONLY regular-file descriptor. Raw evidence
// and batch.json are immutable and deliberately owner-read-only after
// deployment initialization, so recovery must not request write access merely
// to re-establish their durability before manifest exposure.
#[cfg(unix)]
fn open_file_for_sync(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

// FlushFileBuffers requires a write-capable Windows handle. Preserve the
// established access mode there; Raw ACLs on Windows must grant that platform-
// specific durability access to the worker.
#[cfg(windows)]
fn open_file_for_sync(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(all(not(unix), not(windows)))]
fn open_file_for_sync(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path, context: &str) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| io_err(context, e))
}

#[cfg(windows)]
fn sync_directory(path: &Path, context: &str) -> Result<(), StoreError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_err(context, error))
}

#[cfg(all(not(unix), not(windows)))]
fn sync_directory(path: &Path, context: &str) -> Result<(), StoreError> {
    Err(io_err(
        context,
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("directory durability is unsupported for {}", path.display()),
        ),
    ))
}

fn sync_batch_directories(batch_dir: &Path) -> Result<(), StoreError> {
    let mut current = Some(batch_dir);
    for _ in 0..6 {
        let Some(path) = current else {
            break;
        };
        sync_directory(path, &format!("sync Raw directory {}", path.display()))?;
        current = path.parent();
    }
    Ok(())
}

fn sync_manifest_directories(manifest_parent: &Path) -> Result<(), StoreError> {
    let mut current = Some(manifest_parent);
    for _ in 0..5 {
        let Some(path) = current else {
            break;
        };
        sync_directory(path, &format!("sync manifest directory {}", path.display()))?;
        current = path.parent();
    }
    Ok(())
}

/// A provider file name must be a plain name: no separators, no traversal, no
/// drive/absolute paths, no control characters, bounded length.
fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty file name".to_owned());
    }
    if name.len() > 255 {
        return Err("file name longer than 255 bytes".to_owned());
    }
    if name == "." || name == ".." {
        return Err("reserved name".to_owned());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("contains a path separator".to_owned());
    }
    if name.contains(':') {
        return Err("contains a drive/colon character".to_owned());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("has a Windows-ambiguous trailing dot or space".to_owned());
    }
    if name.bytes().any(|b| b.is_ascii_control()) {
        return Err("contains control characters".to_owned());
    }
    Ok(())
}

fn validate_file_entry_name(name: &str) -> Result<(), StoreError> {
    validate_file_name(name).map_err(|reason| StoreError::UnsafeFileName {
        file_name: name.to_owned(),
        reason,
    })
}

fn validate_scope_component(component: &str, value: &str) -> Result<(), StoreError> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value != "."
        && value != ".."
        && !value.ends_with('.')
        && !value.ends_with(' ')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(StoreError::UnsafeScope {
            component: component.to_owned(),
            value: value.to_owned(),
            reason: "must be a bounded plain path component".to_owned(),
        })
    }
}

fn validate_scope(provider: &str, market: &str) -> Result<(), StoreError> {
    validate_scope_component("provider", provider)?;
    validate_scope_component("market", market)
}

fn validate_entry_scope(
    provider: &str,
    market: &str,
    entry: &ManifestEntry,
) -> Result<(), StoreError> {
    if entry.provider != provider || entry.market != market {
        return Err(StoreError::ScopeMismatch {
            expected_provider: provider.to_owned(),
            expected_market: market.to_owned(),
            actual_provider: entry.provider.clone(),
            actual_market: entry.market.clone(),
        });
    }
    Ok(())
}

fn validate_manifest_file_names(entry: &ManifestEntry) -> Result<(), StoreError> {
    for file in &entry.files {
        validate_file_entry_name(&file.file_name)?;
    }
    Ok(())
}

fn io_err(context: &str, e: std::io::Error) -> StoreError {
    StoreError::Io {
        context: context.to_owned(),
        source: e,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier, Mutex, mpsc};
    use std::time::Duration;

    use super::*;

    #[derive(Debug)]
    struct PausingCommit {
        visible: Barrier,
        release: Barrier,
        fail_directory_sync: bool,
    }

    impl BatchCommitOps for PausingCommit {
        fn after_metadata_visible(&self) {
            self.visible.wait();
            self.release.wait();
        }

        fn sync_batch_directories(&self, batch_dir: &Path) -> Result<(), StoreError> {
            if self.fail_directory_sync {
                Err(io_err(
                    "injected batch directory sync",
                    std::io::Error::other("injected directory sync failure"),
                ))
            } else {
                super::sync_batch_directories(batch_dir)
            }
        }
    }

    #[derive(Debug)]
    struct FailingFinalMetadataSync;

    impl BatchCommitOps for FailingFinalMetadataSync {
        fn sync_published_metadata(&self, _file: &File) -> Result<(), StoreError> {
            Err(io_err(
                "injected final batch.json sync",
                std::io::Error::other("injected final metadata sync failure"),
            ))
        }
    }

    #[derive(Debug)]
    struct FailingPreVisibleCleanup;

    impl BatchCommitOps for FailingPreVisibleCleanup {
        fn before_metadata_publish(&self) -> Result<(), StoreError> {
            Err(io_err(
                "injected pre-visible failure",
                std::io::Error::other("injected pre-visible failure"),
            ))
        }

        fn cleanup_batch(&self, _batch_dir: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("injected cleanup failure"))
        }
    }

    #[derive(Debug, Default)]
    struct RecoverySyncProbe {
        fail_file_sync: AtomicBool,
        fail_directory_sync: AtomicBool,
        synced_files: Mutex<Vec<PathBuf>>,
        synced_directories: Mutex<Vec<PathBuf>>,
    }

    impl ManifestReadOps for RecoverySyncProbe {
        fn sync_file(&self, path: &Path) -> Result<(), StoreError> {
            self.synced_files.lock().unwrap().push(path.to_owned());
            if self.fail_file_sync.load(Ordering::SeqCst) {
                Err(io_err(
                    "injected orphan file sync",
                    std::io::Error::other("injected orphan file sync failure"),
                ))
            } else {
                sync_file(path, "re-sync orphan file")
            }
        }

        fn sync_batch_directories(&self, batch_dir: &Path) -> Result<(), StoreError> {
            self.synced_directories
                .lock()
                .unwrap()
                .push(batch_dir.to_owned());
            if self.fail_directory_sync.load(Ordering::SeqCst) {
                Err(io_err(
                    "injected orphan directory sync",
                    std::io::Error::other("injected orphan directory sync failure"),
                ))
            } else {
                super::sync_batch_directories(batch_dir)
            }
        }
    }

    #[derive(Debug)]
    struct PausingOrphanAppend {
        reached: mpsc::Sender<()>,
        release: Arc<Barrier>,
    }

    impl ManifestReadOps for PausingOrphanAppend {
        fn before_manifest_append(&self, _entry: &ManifestEntry) -> Result<(), StoreError> {
            self.reached.send(()).unwrap();
            self.release.wait();
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FailBeforeOrphanAppend(AtomicBool);

    impl ManifestReadOps for FailBeforeOrphanAppend {
        fn before_manifest_append(&self, _entry: &ManifestEntry) -> Result<(), StoreError> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err(io_err(
                    "injected pre-append failure",
                    std::io::Error::other("injected pre-append failure"),
                ))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug)]
    struct FailManifestSync(AtomicBool);

    impl ManifestReadOps for FailManifestSync {
        fn sync_manifest_file(&self, file: &File) -> Result<(), StoreError> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err(io_err(
                    "injected manifest sync failure",
                    std::io::Error::other("injected manifest sync failure"),
                ))
            } else {
                file.sync_all()
                    .map_err(|error| io_err("sync manifest", error))
            }
        }
    }

    #[derive(Debug)]
    struct CommitLockAttemptProbe(mpsc::Sender<()>);

    impl BatchCommitOps for CommitLockAttemptProbe {
        fn before_commit_lock(&self) {
            self.0.send(()).unwrap();
        }
    }

    #[derive(Debug)]
    struct SharedLockProbe {
        before: mpsc::Sender<()>,
        after: mpsc::Sender<()>,
    }

    impl ManifestReadOps for SharedLockProbe {
        fn before_shared_lock(&self) {
            self.before.send(()).unwrap();
        }

        fn after_shared_lock(&self) {
            self.after.send(()).unwrap();
        }
    }

    fn test_envelope(batch_id: BatchId) -> RawEnvelope {
        RawEnvelope::new(
            batch_id,
            ResponseKind::Reference,
            "reference.json".to_owned(),
            b"{}".to_vec(),
            UtcTimestamp::parse_rfc3339("2026-08-10T00:00:00Z").unwrap(),
            crate::contract::RequestMetadata {
                endpoint: "test".to_owned(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: FetchMode::Synthetic,
            },
        )
    }

    fn store_test_batch(store: &RawStore, batch_id: BatchId) -> ManifestEntry {
        let date = TradingDate::parse("2020-01-31").unwrap();
        store
            .store_batch(
                &BatchSpec {
                    provider: crate::contract::PROVIDER_KRX,
                    market: crate::contract::MARKET_KR,
                    date: &date,
                    batch_id,
                    entitlement_reference: None,
                    mode: FetchMode::Synthetic,
                },
                &[test_envelope(batch_id)],
            )
            .unwrap()
    }

    #[test]
    fn file_name_validation_rejects_traversal() {
        for bad in ["", ".", "..", "a/b", "a\\b", "/etc/x", "C:\\x", "a\x00b"] {
            assert!(validate_file_name(bad).is_err(), "{bad:?} must be rejected");
        }
        for good in ["bars.json", "069500.KRX.json", "2020-01-31_ohlcv.csv"] {
            assert!(
                validate_file_name(good).is_ok(),
                "{good:?} must be accepted"
            );
        }
    }

    #[test]
    fn final_metadata_sync_failure_is_indeterminate_and_preserves_exact_batch() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let date = TradingDate::parse("2020-01-31").unwrap();
        let batch_id = BatchId::generate();
        let spec = BatchSpec {
            provider: crate::contract::PROVIDER_KRX,
            market: crate::contract::MARKET_KR,
            date: &date,
            batch_id,
            entitlement_reference: None,
            mode: FetchMode::Synthetic,
        };

        let error = store
            .store_batch_with_commit_ops(
                &spec,
                &[test_envelope(batch_id)],
                &FailingFinalMetadataSync,
            )
            .unwrap_err();
        let entry = match error {
            StoreError::IndeterminateBatchCommit { entry, source } => {
                assert!(matches!(*source, StoreError::Io { .. }));
                *entry
            }
            other => panic!("expected indeterminate batch commit, got {other:?}"),
        };

        assert_eq!(entry.batch_id, batch_id);
        let batch_dir = store.batch_dir(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &date,
            &batch_id,
        );
        assert!(batch_dir.join("batch.json").exists());
        assert_eq!(
            store
                .read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR)
                .unwrap(),
            vec![entry]
        );
    }

    #[test]
    fn orphan_is_resynced_before_exposure_and_failure_blocks_until_retry() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let date = TradingDate::parse("2020-01-31").unwrap();
        let batch_id = BatchId::generate();
        let spec = BatchSpec {
            provider: crate::contract::PROVIDER_KRX,
            market: crate::contract::MARKET_KR,
            date: &date,
            batch_id,
            entitlement_reference: None,
            mode: FetchMode::Synthetic,
        };
        let entry = match store
            .store_batch_with_commit_ops(
                &spec,
                &[test_envelope(batch_id)],
                &FailingFinalMetadataSync,
            )
            .unwrap_err()
        {
            StoreError::IndeterminateBatchCommit { entry, .. } => *entry,
            other => panic!("expected indeterminate batch commit, got {other:?}"),
        };
        let operations = RecoverySyncProbe::default();
        operations.fail_file_sync.store(true, Ordering::SeqCst);

        assert!(matches!(
            store.read_manifest_with_ops(
                crate::contract::PROVIDER_KRX,
                crate::contract::MARKET_KR,
                &operations,
            ),
            Err(StoreError::Io { .. })
        ));
        operations.fail_file_sync.store(false, Ordering::SeqCst);
        operations.fail_directory_sync.store(true, Ordering::SeqCst);
        assert!(matches!(
            store.read_manifest_with_ops(
                crate::contract::PROVIDER_KRX,
                crate::contract::MARKET_KR,
                &operations,
            ),
            Err(StoreError::Io { .. })
        ));
        operations
            .fail_directory_sync
            .store(false, Ordering::SeqCst);
        assert_eq!(
            store
                .read_manifest_with_ops(
                    crate::contract::PROVIDER_KRX,
                    crate::contract::MARKET_KR,
                    &operations,
                )
                .unwrap(),
            vec![entry.clone()]
        );

        let synced_files = operations.synced_files.lock().unwrap();
        assert!(
            synced_files
                .iter()
                .any(|path| path.ends_with(entry.batch_json_file_name()))
        );
        assert!(
            synced_files
                .iter()
                .any(|path| path.ends_with("reference.json"))
        );
        assert!(!operations.synced_directories.lock().unwrap().is_empty());
    }

    #[test]
    fn pre_visible_cleanup_failure_is_typed_and_leftover_is_not_exposed() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let date = TradingDate::parse("2020-01-31").unwrap();
        let batch_id = BatchId::generate();
        let spec = BatchSpec {
            provider: crate::contract::PROVIDER_KRX,
            market: crate::contract::MARKET_KR,
            date: &date,
            batch_id,
            entitlement_reference: None,
            mode: FetchMode::Synthetic,
        };

        let error = store
            .store_batch_with_commit_ops(
                &spec,
                &[test_envelope(batch_id)],
                &FailingPreVisibleCleanup,
            )
            .unwrap_err();
        match error {
            StoreError::CleanupFailed {
                path,
                original,
                cleanup,
            } => {
                assert!(path.contains(&batch_id.to_string()));
                assert!(matches!(*original, StoreError::Io { .. }));
                assert_eq!(cleanup.to_string(), "injected cleanup failure");
            }
            other => panic!("expected typed cleanup failure, got {other:?}"),
        }
        let batch_dir = store.batch_dir(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &date,
            &batch_id,
        );
        assert!(batch_dir.exists());
        assert!(!batch_dir.join("batch.json").exists());
        assert!(
            store
                .read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn orphan_scan_never_observes_partially_written_batch_metadata() {
        let root = tempfile::tempdir().expect("temp root");
        let store = RawStore::new(root.path());
        let date = TradingDate::parse("2020-01-31").unwrap();
        let batch_id = BatchId::generate();
        let entry = ManifestEntry {
            batch_id,
            provider: crate::contract::PROVIDER_KRX.to_owned(),
            market: crate::contract::MARKET_KR.to_owned(),
            date,
            retrieved_at: UtcTimestamp::parse_rfc3339("2026-08-10T00:00:00Z").unwrap(),
            mode: FetchMode::Synthetic,
            entitlement_reference: None,
            files: Vec::new(),
        };
        let batch_dir = store.batch_dir(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &date,
            &batch_id,
        );
        fs::create_dir_all(&batch_dir).unwrap();

        let mut prepared = prepare_batch_metadata(&batch_dir, &entry).unwrap();
        assert_ne!(prepared.path(), batch_dir.join("batch.json"));
        assert!(!batch_dir.join("batch.json").exists());
        prepared.as_file_mut().set_len(0).unwrap();
        prepared.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
        prepared.write_all(b"{").unwrap();
        prepared.as_file().sync_all().unwrap();
        let partial_path = prepared.path().to_owned();
        assert_eq!(fs::read(&partial_path).unwrap(), b"{");

        let start = Arc::new(Barrier::new(2));
        let scanner_store = store.clone();
        let scanner_start = Arc::clone(&start);
        let scanner = std::thread::spawn(move || {
            scanner_start.wait();
            scanner_store.read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR)
        });
        start.wait();
        assert!(scanner.join().unwrap().unwrap().is_empty());

        drop(prepared);
        assert!(!partial_path.exists());
        let prepared = prepare_batch_metadata(&batch_dir, &entry).unwrap();
        let committed_file =
            publish_batch_metadata(prepared, &batch_dir.join("batch.json")).unwrap();
        assert_eq!(
            committed_file.metadata().unwrap().len(),
            fs::metadata(batch_dir.join("batch.json")).unwrap().len()
        );
        sync_directory(&batch_dir, "sync committed batch metadata").unwrap();
        assert_eq!(
            store
                .read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR,)
                .unwrap(),
            vec![entry.clone()]
        );

        let replacement = prepare_batch_metadata(&batch_dir, &entry).unwrap();
        let replacement_path = replacement.path().to_owned();
        assert!(matches!(
            publish_batch_metadata(replacement, &batch_dir.join("batch.json")),
            Err(StoreError::FileExists { .. })
        ));
        assert!(!replacement_path.exists());
        let committed: ManifestEntry =
            serde_json::from_slice(&fs::read(batch_dir.join("batch.json")).unwrap()).unwrap();
        assert_eq!(committed, entry);
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_flush_is_real_and_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        sync_directory(root.path(), "flush existing Windows directory").unwrap();

        let injected = PausingCommit {
            visible: Barrier::new(1),
            release: Barrier::new(1),
            fail_directory_sync: true,
        };
        assert!(matches!(
            injected.sync_batch_directories(root.path()),
            Err(StoreError::Io { .. })
        ));

        let missing = root.path().join("missing-directory");
        assert!(matches!(
            sync_directory(&missing, "flush missing Windows directory"),
            Err(StoreError::Io { .. })
        ));
    }

    #[test]
    fn reader_blocks_while_visible_metadata_is_not_yet_durable() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let batch_id = BatchId::generate();
        let date = TradingDate::parse("2020-01-31").unwrap();
        let operations = Arc::new(PausingCommit {
            visible: Barrier::new(2),
            release: Barrier::new(2),
            fail_directory_sync: false,
        });

        let writer_store = store.clone();
        let writer_operations = Arc::clone(&operations);
        let writer = std::thread::spawn(move || {
            let spec = BatchSpec {
                provider: crate::contract::PROVIDER_KRX,
                market: crate::contract::MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode: FetchMode::Synthetic,
            };
            writer_store.store_batch_with_commit_ops(
                &spec,
                &[test_envelope(batch_id)],
                writer_operations.as_ref(),
            )
        });
        operations.visible.wait();

        let reader_store = store.clone();
        let (before_tx, before_rx) = mpsc::channel();
        let (after_tx, after_rx) = mpsc::channel();
        let (reader_result_tx, reader_result_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let probe = SharedLockProbe {
                before: before_tx,
                after: after_tx,
            };
            let result = reader_store.read_manifest_with_ops(
                crate::contract::PROVIDER_KRX,
                crate::contract::MARKET_KR,
                &probe,
            );
            reader_result_tx.send(result).unwrap();
        });
        before_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(after_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
        assert!(matches!(
            reader_result_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        operations.release.wait();
        let written = writer.join().unwrap().unwrap();
        after_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let visible = reader_result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        reader.join().unwrap();
        assert_eq!(visible, vec![written]);
    }

    #[test]
    fn commit_lock_exclusive_handle_blocks_separate_nonblocking_shared_handle() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let exclusive = store
            .open_commit_lock(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR)
            .unwrap();
        let shared = store
            .open_commit_lock(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR)
            .unwrap();

        FileExt::lock_exclusive(&exclusive).unwrap();
        let contention = FileExt::try_lock_shared(&shared).unwrap_err();
        assert!(
            contention.kind() == std::io::ErrorKind::WouldBlock
                || contention.raw_os_error() == Some(33),
            "expected nonblocking lock contention, got {contention:?}"
        );

        FileExt::unlock(&exclusive).unwrap();
        FileExt::try_lock_shared(&shared).unwrap();
        FileExt::unlock(&shared).unwrap();
    }

    #[test]
    fn directory_sync_failure_is_indeterminate_and_reader_recovers_same_batch() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let batch_id = BatchId::generate();
        let date = TradingDate::parse("2020-01-31").unwrap();
        let batch_dir = store.batch_dir(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &date,
            &batch_id,
        );
        let operations = Arc::new(PausingCommit {
            visible: Barrier::new(2),
            release: Barrier::new(2),
            fail_directory_sync: true,
        });

        let writer_store = store.clone();
        let writer_operations = Arc::clone(&operations);
        let writer = std::thread::spawn(move || {
            let spec = BatchSpec {
                provider: crate::contract::PROVIDER_KRX,
                market: crate::contract::MARKET_KR,
                date: &date,
                batch_id,
                entitlement_reference: None,
                mode: FetchMode::Synthetic,
            };
            writer_store.store_batch_with_commit_ops(
                &spec,
                &[test_envelope(batch_id)],
                writer_operations.as_ref(),
            )
        });
        operations.visible.wait();

        let reader_store = store.clone();
        let (before_tx, before_rx) = mpsc::channel();
        let (after_tx, after_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let probe = SharedLockProbe {
                before: before_tx,
                after: after_tx,
            };
            result_tx
                .send(reader_store.read_manifest_with_ops(
                    crate::contract::PROVIDER_KRX,
                    crate::contract::MARKET_KR,
                    &probe,
                ))
                .unwrap();
        });
        before_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(after_rx.try_recv(), Err(mpsc::TryRecvError::Empty));

        operations.release.wait();
        let entry = match writer.join().unwrap().unwrap_err() {
            StoreError::IndeterminateBatchCommit { entry, source } => {
                assert!(matches!(*source, StoreError::Io { .. }));
                *entry
            }
            other => panic!("expected indeterminate directory sync, got {other:?}"),
        };
        after_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap(),
            vec![entry]
        );
        reader.join().unwrap();
        assert!(batch_dir.join("batch.json").exists());
    }

    #[test]
    fn reconciled_manifest_lock_orders_orphan_before_waiting_normal_writer() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let orphan = store_test_batch(&store, BatchId::generate());
        fs::remove_file(
            store.manifest_path(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR),
        )
        .unwrap();

        let (orphan_reached_tx, orphan_reached_rx) = mpsc::channel();
        let release = Arc::new(Barrier::new(2));
        let reconcile_store = store.clone();
        let reconcile_release = Arc::clone(&release);
        let reconcile = std::thread::spawn(move || {
            reconcile_store.read_reconciled_manifest_with_ops(
                crate::contract::PROVIDER_KRX,
                crate::contract::MARKET_KR,
                &PausingOrphanAppend {
                    reached: orphan_reached_tx,
                    release: reconcile_release,
                },
            )
        });
        orphan_reached_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let normal_id = BatchId::generate();
        let writer_store = store.clone();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let date = TradingDate::parse("2020-01-31").unwrap();
            let result = writer_store.store_batch_with_commit_ops(
                &BatchSpec {
                    provider: crate::contract::PROVIDER_KRX,
                    market: crate::contract::MARKET_KR,
                    date: &date,
                    batch_id: normal_id,
                    entitlement_reference: None,
                    mode: FetchMode::Synthetic,
                },
                &[test_envelope(normal_id)],
                &CommitLockAttemptProbe(attempt_tx),
            );
            result_tx.send(result).unwrap();
        });
        attempt_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            result_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        release.wait();
        assert_eq!(reconcile.join().unwrap().unwrap(), vec![orphan.clone()]);
        let normal = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        writer.join().unwrap();
        assert_eq!(
            store
                .read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR,)
                .unwrap(),
            vec![orphan, normal],
        );
    }

    #[test]
    fn reconciled_manifest_pre_append_fault_leaves_orphan_for_retry() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let orphan = store_test_batch(&store, BatchId::generate());
        let manifest_path =
            store.manifest_path(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR);
        fs::remove_file(&manifest_path).unwrap();

        let failure = store.read_reconciled_manifest_with_ops(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &FailBeforeOrphanAppend(AtomicBool::new(true)),
        );
        assert!(matches!(failure, Err(StoreError::Io { .. })));
        assert_eq!(
            store
                .read_manifest(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR,)
                .unwrap(),
            vec![orphan.clone()],
        );

        assert_eq!(
            store
                .read_reconciled_manifest(
                    crate::contract::PROVIDER_KRX,
                    crate::contract::MARKET_KR,
                )
                .unwrap(),
            vec![orphan],
        );
        assert_eq!(
            fs::read_to_string(manifest_path).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn reconciled_manifest_post_write_fault_deduplicates_identical_replay() {
        let root = tempfile::tempdir().unwrap();
        let store = RawStore::new(root.path());
        let orphan = store_test_batch(&store, BatchId::generate());
        let manifest_path =
            store.manifest_path(crate::contract::PROVIDER_KRX, crate::contract::MARKET_KR);
        fs::remove_file(&manifest_path).unwrap();

        let failure = store.read_reconciled_manifest_with_ops(
            crate::contract::PROVIDER_KRX,
            crate::contract::MARKET_KR,
            &FailManifestSync(AtomicBool::new(true)),
        );
        assert!(matches!(failure, Err(StoreError::Io { .. })));
        assert_eq!(
            fs::read_to_string(&manifest_path).unwrap().lines().count(),
            1
        );

        assert_eq!(
            store
                .read_reconciled_manifest(
                    crate::contract::PROVIDER_KRX,
                    crate::contract::MARKET_KR,
                )
                .unwrap(),
            vec![orphan],
        );
        assert_eq!(
            fs::read_to_string(manifest_path).unwrap().lines().count(),
            1
        );
    }
}
