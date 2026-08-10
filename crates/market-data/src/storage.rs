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
//! - **Crash-durable batches**: evidence and `batch.json` are synced before the
//!   manifest is appended. Failures before that durable point clean up; a later
//!   manifest failure preserves a discoverable orphan batch for recovery.
//! - **Path traversal rejection**: provider file names must be plain names.
//! - **Append-only manifest**: JSONL writes are serialized under a file lock;
//!   reads verify stored bytes against the recorded content hash (tamper detection).

use std::collections::BTreeMap;
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
            Self::FileExists { .. }
            | Self::UnsafeFileName { .. }
            | Self::UnsafeScope { .. }
            | Self::ScopeMismatch { .. }
            | Self::UnsafePath { .. }
            | Self::ContentHashMismatch { .. }
            | Self::InvalidBatchMetadata { .. } => None,
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

    /// Persists one delivery as a new immutable batch and appends its manifest row.
    ///
    /// Failures before evidence, metadata, and their directory entries are
    /// synced remove the partial batch. Once that durable point is reached, a
    /// manifest failure preserves the batch for orphan discovery.
    pub fn store_batch(
        &self,
        spec: &BatchSpec<'_>,
        envelopes: &[RawEnvelope],
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

        let cleanup = |e: StoreError| -> StoreError {
            let _ = fs::remove_dir_all(&dir);
            e
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
        publish_batch_metadata(prepared, &dir.join(entry.batch_json_file_name()))
            .map_err(cleanup)?;

        sync_batch_directories(&dir).map_err(cleanup)?;

        self.append_manifest(provider, market, &entry)?;

        Ok(entry)
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
        let path = self.manifest_path(provider, market);
        let parent = path.parent().ok_or_else(|| {
            io_err(
                "manifest parent",
                std::io::Error::other(format!("{} has no parent", path.display())),
            )
        })?;
        fs::create_dir_all(parent).map_err(|e| io_err("create manifest dir", e))?;
        let mut line = serde_json::to_vec(entry).map_err(|source| StoreError::Serialization {
            context: "manifest entry serialize".to_owned(),
            source,
        })?;
        line.push(b'\n');
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
        FileExt::lock_exclusive(&f).map_err(|e| io_err("lock manifest", e))?;

        let result = (|| {
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes)
                .map_err(|e| io_err("read manifest before append", e))?;
            let mut existing = BTreeMap::new();
            let mut existing_order = Vec::new();
            parse_manifest_records(
                provider,
                market,
                &path,
                &bytes,
                &mut existing,
                &mut existing_order,
            )?;
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                let tail_start = bytes
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |position| position + 1);
                match serde_json::from_slice::<ManifestEntry>(&bytes[tail_start..]) {
                    Ok(tail_entry) => {
                        validate_entry_scope(provider, market, &tail_entry)?;
                        validate_manifest_file_names(&tail_entry)?;
                        f.seek(SeekFrom::End(0))
                            .map_err(|e| io_err("seek unterminated manifest end", e))?;
                        f.write_all(b"\n")
                            .map_err(|e| io_err("terminate complete manifest record", e))?;
                    }
                    Err(source) if source.is_eof() => {
                        f.set_len(tail_start as u64)
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
            f.seek(SeekFrom::End(0))
                .map_err(|e| io_err("seek manifest end", e))?;
            f.write_all(&line)
                .map_err(|e| io_err("append manifest", e))?;
            f.sync_all().map_err(|e| io_err("sync manifest", e))
        })();
        let unlock = FileExt::unlock(&f).map_err(|e| io_err("unlock manifest", e));
        result?;
        unlock?;
        sync_manifest_directories(parent)
    }

    /// All committed manifest rows plus durable orphan batches, oldest first.
    /// An unterminated final manifest fragment is ignored as a crash tail;
    /// malformed complete or middle records are permanent corruption.
    pub fn read_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        validate_scope(provider, market)?;
        let path = self.manifest_path(provider, market);
        let mut entries = BTreeMap::new();
        let mut manifest_order = Vec::new();
        if path.exists() {
            let mut f = File::open(&path)
                .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
            FileExt::lock_shared(&f).map_err(|e| io_err("lock manifest for read", e))?;
            let mut bytes = Vec::new();
            let read_result = f
                .read_to_end(&mut bytes)
                .map_err(|e| io_err("read manifest", e));
            let unlock = FileExt::unlock(&f).map_err(|e| io_err("unlock manifest", e));
            read_result?;
            unlock?;
            parse_manifest_records(
                provider,
                market,
                &path,
                &bytes,
                &mut entries,
                &mut manifest_order,
            )?;
        }
        self.discover_orphan_batches(provider, market, &mut entries)?;
        let mut orphan_ids: Vec<_> = entries
            .keys()
            .copied()
            .filter(|batch_id| !manifest_order.contains(batch_id))
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

    fn discover_orphan_batches(
        &self,
        provider: &str,
        market: &str,
        entries: &mut BTreeMap<BatchId, ManifestEntry>,
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
                let (_, canonical_batch) =
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
                merge_manifest_entry(entries, entry, &metadata_path)?;
            }
        }
        Ok(())
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
) -> Result<(), StoreError> {
    temporary
        .persist_noclobber(final_path)
        .map(|_| ())
        .map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                StoreError::FileExists {
                    path: final_path.display().to_string(),
                }
            } else {
                io_err("publish batch.json", error.error)
            }
        })
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

#[cfg(unix)]
fn sync_directory(path: &Path, context: &str) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| io_err(context, e))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path, _context: &str) -> Result<(), StoreError> {
    // std does not expose Windows directory handles with backup semantics;
    // evidence files and metadata are still sync_all'd on every platform.
    Ok(())
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
    use std::sync::{Arc, Barrier};

    use super::*;

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
        publish_batch_metadata(prepared, &batch_dir.join("batch.json")).unwrap();
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
}
