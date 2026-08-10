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
//! - **No partial batches**: `store_batch` is all-or-nothing — on any failure the
//!   batch dir is removed and nothing reaches the manifest.
//! - **Path traversal rejection**: provider file names must be plain names.
//! - **Append-only manifest**: JSONL appended with `OpenOptions::append`; reads
//!   verify stored bytes against the recorded content hash (tamper detection).

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};

use crate::contract::{date_partition, FetchMode, RawEnvelope, ResponseKind, StoredFile};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Filesystem or content-verification failure.
    Io { context: String, detail: String },
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
            Self::Io { context, detail } => write!(f, "raw store io failure ({context}): {detail}"),
        }
    }
}

impl std::error::Error for StoreError {}

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
    /// All-or-nothing: on any failure the batch dir is removed and the manifest
    /// is untouched.
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
        let parent = dir.parent().ok_or_else(|| StoreError::Io {
            context: "batch-dir-parent".to_owned(),
            detail: dir.display().to_string(),
        })?;
        fs::create_dir_all(parent).map_err(|e| io_err("create date partition", e))?;
        fs::create_dir(&dir)
            .map_err(|e| io_err(&format!("create batch dir {}", dir.display()), e))?;

        let cleanup = |e: StoreError| -> StoreError {
            let _ = fs::remove_dir_all(&dir);
            e
        };

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

        let json = serde_json::to_string_pretty(&entry).map_err(|e| {
            cleanup(StoreError::Io {
                context: "batch.json serialize".to_owned(),
                detail: e.to_string(),
            })
        })?;
        let mut meta = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(entry.batch_json_file_name()))
            .map_err(|e| cleanup(io_err("create batch.json", e)))?;
        meta.write_all(json.as_bytes())
            .map_err(|e| cleanup(io_err("write batch.json", e)))?;

        self.append_manifest(provider, market, &entry)
            .map_err(cleanup)?;

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
        let parent = path.parent().ok_or_else(|| StoreError::Io {
            context: "manifest parent".to_owned(),
            detail: path.display().to_string(),
        })?;
        fs::create_dir_all(parent).map_err(|e| io_err("create manifest dir", e))?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
        let mut line = serde_json::to_string(entry).map_err(|e| StoreError::Io {
            context: "manifest entry serialize".to_owned(),
            detail: e.to_string(),
        })?;
        line.push('\n');
        f.write_all(line.as_bytes())
            .map_err(|e| io_err("append manifest", e))
    }

    /// All manifest rows, oldest first. An absent manifest reads as empty.
    pub fn read_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<ManifestEntry>, StoreError> {
        validate_scope(provider, market)?;
        let path = self.manifest_path(provider, market);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let f = File::open(&path)
            .map_err(|e| io_err(&format!("open manifest {}", path.display()), e))?;
        let reader = BufReader::new(f);
        let mut entries = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| io_err(&format!("read manifest line {idx}"), e))?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: ManifestEntry = serde_json::from_str(&line).map_err(|e| StoreError::Io {
                context: format!("manifest line {idx}"),
                detail: e.to_string(),
            })?;
            validate_entry_scope(provider, market, &entry)?;
            validate_manifest_file_names(&entry)?;
            entries.push(entry);
        }
        Ok(entries)
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
        let canonical_dir = fs::canonicalize(&dir)
            .map_err(|e| io_err(&format!("canonicalize batch dir {}", dir.display()), e))?;
        let mut out = Vec::with_capacity(entry.files.len());
        for file in &entry.files {
            let path = dir.join(&file.file_name);
            let storage_path = fs::canonicalize(&path)
                .map_err(|e| io_err(&format!("canonicalize {}", path.display()), e))?;
            if !storage_path.starts_with(&canonical_dir) {
                return Err(StoreError::UnsafePath {
                    path: storage_path.display().to_string(),
                    reason: format!("escapes batch directory {}", canonical_dir.display()),
                });
            }
            let bytes = fs::read(&storage_path)
                .map_err(|e| io_err(&format!("read {}", path.display()), e))?;
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
                storage_path: path,
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
        detail: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
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
}
