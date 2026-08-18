//! Authoritative immutable-generation retention and reconciliation.
//!
//! Publication promotes bytes before the queue transaction commits. If the
//! client loses the COMMIT response, the database may or may not reference the
//! generation, so deleting it immediately is unsafe. This module takes a
//! repeatable database snapshot, then scans only the exact validated
//! `generations/<commit>/<publication>/` layout. Database failures are
//! fail-closed: no filesystem deletion is attempted.

use crate::error::QueueError;
use crate::queue::JobQueue;
use crate::runner::is_exact_code_commit;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

#[cfg(unix)]
use std::ffi::{CStr, CString, OsStr};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};

/// This directory is deliberately outside every UUID run directory. A
/// publication is moved into it before any destructive operation, so a
/// crash leaves an object that the next pass can inspect and finish safely.
const QUARANTINE_DIRECTORY: &str = ".reconcile-quarantine";

#[cfg(test)]
type BeforeQuarantineHook = Arc<dyn Fn(&Path) + Send + Sync + 'static>;

#[cfg(test)]
static BEFORE_QUARANTINE_HOOK: OnceLock<Mutex<Option<BeforeQuarantineHook>>> = OnceLock::new();

/// Stable log/event code for one retention pass.
pub const BACKTEST_RECONCILE_EVENT_CODE: &str = "BACKTEST_GENERATION_RECONCILE";

/// Stable log/event code when an authoritative DB snapshot cannot be loaded.
pub const BACKTEST_RECONCILE_DB_UNAVAILABLE_CODE: &str = "BACKTEST_RECONCILE_DB_UNAVAILABLE";

/// Stable diagnostic code for non-DB reconciliation failures.
pub const BACKTEST_RECONCILE_ERROR_CODE: &str = "BACKTEST_RECONCILE_ERROR";

/// Retention policy for unreferenced immutable generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconcilerConfig {
    /// An unreferenced generation must be older than this before deletion.
    pub safe_grace: Duration,
}

impl ReconcilerConfig {
    pub fn validate(self) -> Result<Self, ReconcileError> {
        if self.safe_grace.is_zero() {
            return Err(ReconcileError::InvalidConfiguration(
                "reconciler safe grace must be greater than zero".to_owned(),
            ));
        }
        Ok(self)
    }
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            // One queue lease plus a generous DB/client recovery window. The
            // deployment can increase this, but never set it to zero.
            safe_grace: Duration::from_secs(15 * 60),
        }
    }
}

/// A parsed immutable publication identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenerationIdentity {
    pub run_id: Uuid,
    pub code_commit: String,
    pub publication_id: Uuid,
}

/// The DB snapshot used by the filesystem pass. Keeping it explicit makes the
/// deletion policy unit-testable without a PostgreSQL fixture.
#[derive(Debug, Clone, Default)]
pub struct ReconcileDbSnapshot {
    pub referenced_generations: HashSet<GenerationIdentity>,
    /// Runs with an active/canceling queue attempt are retained wholesale: a
    /// runner may have promoted a generation immediately before its
    /// publication transaction. This set closes that race without treating a
    /// stale RUNNING run left by an exhausted queue job as permanently active.
    pub active_runs: HashSet<Uuid>,
}

/// Counters emitted in the daemon's structured diagnostic line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub scanned_generations: u64,
    pub referenced_retained: u64,
    pub active_retained: u64,
    pub fresh_retained: u64,
    pub deleted_generations: u64,
    pub malformed_skipped: u64,
    pub symlink_skipped: u64,
    pub deletion_errors: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("reconciler database snapshot unavailable: {0}")]
    Database(#[source] QueueError),
    #[error("reconciler database reference is invalid: {0}")]
    InvalidReference(String),
    #[error("reconciler filesystem is unavailable: {0}")]
    Filesystem(String),
    #[error("invalid reconciler configuration: {0}")]
    InvalidConfiguration(String),
}

/// Read both reference and active-run sets in one DB transaction. No deletion
/// happens until both reads and the transaction commit succeed.
async fn authoritative_snapshot(queue: &JobQueue) -> Result<ReconcileDbSnapshot, ReconcileError> {
    // PostgreSQL's default READ COMMITTED isolation takes a fresh snapshot
    // for each statement.  Reading artifacts and active runs in separate
    // statements at that level could observe the publication transaction
    // between the two reads and incorrectly make a just-committed generation
    // look both unreferenced and inactive.  `begin_repeatable_read` pins one
    // snapshot before consulting either table; a publication is then seen
    // entirely before or entirely after this pass, never in the middle.
    let mut tx = queue
        .begin_repeatable_read()
        .await
        .map_err(ReconcileError::Database)?;
    let paths: Vec<String> = sqlx::query_scalar("SELECT parquet_path FROM result_artifacts")
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| ReconcileError::Database(QueueError::Database(error)))?;
    let active: Vec<Uuid> = sqlx::query_scalar(
        "SELECT br.id
         FROM backtest_runs br
         LEFT JOIN jobs j ON j.id = br.job_id
         WHERE br.status IN ('PENDING', 'RUNNING')
           AND (j.id IS NULL OR j.status IN ('RUNNING', 'CANCELED'))",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| ReconcileError::Database(QueueError::Database(error)))?;
    tx.commit()
        .await
        .map_err(|error| ReconcileError::Database(QueueError::Database(error)))?;

    let mut referenced_generations = HashSet::new();
    for path in paths {
        match parse_generation_reference(&path)? {
            Some(identity) => {
                referenced_generations.insert(identity);
            }
            None => {
                // Legacy compatibility paths are intentionally ignored. They
                // are outside the immutable generation tree and therefore
                // never become deletion candidates in this pass.
            }
        }
    }
    Ok(ReconcileDbSnapshot {
        referenced_generations,
        active_runs: active.into_iter().collect(),
    })
}

/// Reconcile one artifact root against an authoritative database snapshot.
/// This function performs no database I/O and is public for deterministic
/// filesystem contract tests.
pub fn reconcile_filesystem(
    artifact_root: &Path,
    snapshot: &ReconcileDbSnapshot,
    config: ReconcilerConfig,
) -> Result<ReconcileReport, ReconcileError> {
    config.validate()?;
    validate_root(artifact_root)?;
    let quarantine_root = ensure_quarantine_root(artifact_root)?;
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(config.safe_grace)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut report = ReconcileReport::default();

    // A previous process may have crashed after the atomic rename and before
    // deletion. Recover those entries before scanning live generations. The
    // quarantine names retain the original identity, so a later DB snapshot
    // can keep an object that became referenced while it was quarantined.
    reconcile_quarantine(
        artifact_root,
        &quarantine_root,
        snapshot,
        cutoff,
        &mut report,
    )?;

    let root_entries = fs::read_dir(artifact_root).map_err(|error| {
        ReconcileError::Filesystem(format!("cannot scan artifact root: {error}"))
    })?;
    for entry in root_entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        let run_dir = entry.path();
        if entry.file_name().to_str() == Some(QUARANTINE_DIRECTORY) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&run_dir) {
            Ok(metadata) => metadata,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            report.symlink_skipped += 1;
            continue;
        }
        if !metadata.is_dir() {
            report.malformed_skipped += 1;
            continue;
        }
        let Some(run_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
        else {
            report.malformed_skipped += 1;
            continue;
        };
        let generations = run_dir.join("generations");
        let generations_metadata = match fs::symlink_metadata(&generations) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        if generations_metadata.file_type().is_symlink() {
            report.symlink_skipped += 1;
            continue;
        }
        if !generations_metadata.is_dir() {
            report.malformed_skipped += 1;
            continue;
        }
        let commit_entries = match fs::read_dir(&generations) {
            Ok(entries) => entries,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        for commit_entry in commit_entries {
            let commit_entry = match commit_entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.malformed_skipped += 1;
                    continue;
                }
            };
            let commit_dir = commit_entry.path();
            let commit_metadata = match fs::symlink_metadata(&commit_dir) {
                Ok(metadata) => metadata,
                Err(_) => {
                    report.malformed_skipped += 1;
                    continue;
                }
            };
            if commit_metadata.file_type().is_symlink() {
                report.symlink_skipped += 1;
                continue;
            }
            let Some(code_commit) = commit_entry.file_name().to_str().map(str::to_owned) else {
                report.malformed_skipped += 1;
                continue;
            };
            if !commit_metadata.is_dir() || !is_exact_code_commit(&code_commit) {
                report.malformed_skipped += 1;
                continue;
            }
            let publication_entries = match fs::read_dir(&commit_dir) {
                Ok(entries) => entries,
                Err(_) => {
                    report.malformed_skipped += 1;
                    continue;
                }
            };
            for publication_entry in publication_entries {
                let publication_entry = match publication_entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        report.malformed_skipped += 1;
                        continue;
                    }
                };
                let publication_dir = publication_entry.path();
                let publication_metadata = match fs::symlink_metadata(&publication_dir) {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        report.malformed_skipped += 1;
                        continue;
                    }
                };
                if publication_metadata.file_type().is_symlink() {
                    report.symlink_skipped += 1;
                    continue;
                }
                let Some(publication_id) = publication_entry
                    .file_name()
                    .to_str()
                    .and_then(|name| Uuid::parse_str(name).ok())
                else {
                    report.malformed_skipped += 1;
                    continue;
                };
                if !publication_metadata.is_dir() {
                    report.malformed_skipped += 1;
                    continue;
                }
                let artifacts = publication_dir.join("artifacts");
                let artifacts_metadata = match fs::symlink_metadata(&artifacts) {
                    Ok(metadata) => metadata,
                    Err(_) => {
                        report.malformed_skipped += 1;
                        continue;
                    }
                };
                if artifacts_metadata.file_type().is_symlink() {
                    report.symlink_skipped += 1;
                    continue;
                }
                if !artifacts_metadata.is_dir() {
                    report.malformed_skipped += 1;
                    continue;
                }
                report.scanned_generations += 1;
                let identity = GenerationIdentity {
                    run_id,
                    code_commit: code_commit.clone(),
                    publication_id,
                };
                if snapshot.referenced_generations.contains(&identity) {
                    report.referenced_retained += 1;
                    continue;
                }
                if snapshot.active_runs.contains(&run_id) {
                    report.active_retained += 1;
                    continue;
                }
                let Ok(modified) = publication_metadata.modified() else {
                    report.fresh_retained += 1;
                    continue;
                };
                if modified > cutoff {
                    report.fresh_retained += 1;
                    continue;
                }

                // This validation is intentionally no-follow and fd-relative
                // on Unix. It is only a precondition for the rename; the
                // object is revalidated after that rename because a publisher
                // or an attacker may have changed the final directory entry
                // in the intervening window.
                match validate_generation_at(artifact_root, &publication_dir) {
                    Ok(RemoveResult::Deleted) => {}
                    Ok(RemoveResult::SkippedSymlink) => {
                        report.symlink_skipped += 1;
                        continue;
                    }
                    Ok(RemoveResult::SkippedMalformed) => {
                        report.malformed_skipped += 1;
                        continue;
                    }
                    Err(_) => {
                        report.deletion_errors += 1;
                        continue;
                    }
                }

                #[cfg(test)]
                invoke_before_quarantine_hook(&publication_dir);

                let quarantine_name = quarantine_name(&identity);
                match move_into_quarantine(
                    artifact_root,
                    &publication_dir,
                    &quarantine_root,
                    &quarantine_name,
                ) {
                    Ok(()) => {
                        match remove_quarantined_generation(
                            artifact_root,
                            &quarantine_root,
                            std::ffi::OsStr::new(&quarantine_name),
                        ) {
                            Ok(RemoveResult::Deleted) => report.deleted_generations += 1,
                            Ok(RemoveResult::SkippedSymlink) => report.symlink_skipped += 1,
                            Ok(RemoveResult::SkippedMalformed) => report.malformed_skipped += 1,
                            Err(_) => report.deletion_errors += 1,
                        }
                    }
                    // A rename failure leaves the canonical generation in
                    // place. It is never safe to fall back to path deletion.
                    Err(_) => report.deletion_errors += 1,
                }
            }
        }
    }
    Ok(report)
}

/// Load the authoritative DB snapshot, then reconcile. The two stages are
/// deliberately ordered so an unavailable DB can never result in deletion.
pub async fn reconcile_artifacts(
    queue: &JobQueue,
    artifact_root: &Path,
    config: ReconcilerConfig,
) -> Result<ReconcileReport, ReconcileError> {
    let snapshot = authoritative_snapshot(queue).await?;
    reconcile_filesystem(artifact_root, &snapshot, config)
}

fn validate_root(root: &Path) -> Result<(), ReconcileError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        ReconcileError::Filesystem(format!("artifact root unavailable: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ReconcileError::Filesystem(
            "artifact root must be a non-symlink directory".to_owned(),
        ));
    }
    Ok(())
}

/// Parse only the immutable API-root-relative shape. Legacy paths are
/// returned as `None`; malformed paths that claim to be generations fail
/// closed rather than broadening the deletion matcher.
pub fn parse_generation_reference(
    path: &str,
) -> Result<Option<GenerationIdentity>, ReconcileError> {
    let parts: Vec<&str> = path.split('/').collect();
    let generation_root = parts
        .windows(2)
        .position(|parts| parts == ["backtest", "runs"]);
    let Some(generation_root) = generation_root else {
        return Ok(None);
    };
    let candidate = &parts[generation_root..];
    if candidate.len() < 4 || candidate[3] != "generations" {
        // The older canonical path also starts with `backtest/runs/<uuid>`;
        // it is outside this reconciler's deletion scope.
        return Ok(None);
    }
    if path.contains('\\') {
        return Err(ReconcileError::InvalidReference(format!(
            "immutable generation path uses unsupported backslash separators: {path:?}"
        )));
    }
    if parts.iter().any(|part| *part == "." || *part == "..") {
        return Err(ReconcileError::InvalidReference(format!(
            "unsafe immutable generation path {path:?}"
        )));
    }
    let parts = candidate;
    if parts.len() != 8
        || parts[3] != "generations"
        || parts[6] != "artifacts"
        || parts[7].is_empty()
    {
        return Err(ReconcileError::InvalidReference(format!(
            "invalid immutable generation path {path:?}"
        )));
    }
    if parts
        .iter()
        .any(|part| *part == "." || *part == ".." || part.is_empty())
    {
        return Err(ReconcileError::InvalidReference(format!(
            "unsafe immutable generation path {path:?}"
        )));
    }
    let run_id = Uuid::parse_str(parts[2]).map_err(|_| {
        ReconcileError::InvalidReference(format!("generation path has invalid run id: {path:?}"))
    })?;
    if !is_exact_code_commit(parts[4]) {
        return Err(ReconcileError::InvalidReference(format!(
            "generation path has invalid code commit: {path:?}"
        )));
    }
    let publication_id = Uuid::parse_str(parts[5]).map_err(|_| {
        ReconcileError::InvalidReference(format!(
            "generation path has invalid publication id: {path:?}"
        ))
    })?;
    if publication_id.is_nil() {
        return Err(ReconcileError::InvalidReference(format!(
            "generation path has nil publication id: {path:?}"
        )));
    }
    Ok(Some(GenerationIdentity {
        run_id,
        code_commit: parts[4].to_owned(),
        publication_id,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoveResult {
    Deleted,
    SkippedSymlink,
    SkippedMalformed,
}

/// Validate and remove only regular files/directories below one generation.
///
/// The old implementation validated a path and then recursively reopened that
/// path. A directory could be exchanged for a symlink in that gap; the next
/// `read_dir(path)` would then walk outside the artifact root. The lifecycle
/// below has two separate safety boundaries:
///
/// 1. the exact publication directory entry is atomically renamed into a
///    service-owned quarantine directory, and
/// 2. the quarantined object is revalidated and removed through directory
///    handles with no-follow flags, never by reopening a mutable path.
///
/// A failed rename or revalidation leaves the object in place (or in
/// quarantine) for a later pass. There is no unsafe fallback.
fn ensure_quarantine_root(artifact_root: &Path) -> Result<PathBuf, ReconcileError> {
    let quarantine_root = artifact_root.join(QUARANTINE_DIRECTORY);
    match fs::symlink_metadata(&quarantine_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ReconcileError::Filesystem(
                    "reconciler quarantine must be a non-symlink directory".to_owned(),
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&quarantine_root).map_err(|error| {
                ReconcileError::Filesystem(format!("cannot create reconciler quarantine: {error}"))
            })?;
        }
        Err(error) => {
            return Err(ReconcileError::Filesystem(format!(
                "cannot inspect reconciler quarantine: {error}"
            )));
        }
    }

    // The quarantine is service-owned state. Restricting it to the service
    // user also prevents an unrelated user from racing a no-replace rename.
    #[cfg(unix)]
    {
        // chmod through the no-follow handle. A path-based metadata/chmod
        // pair would itself be vulnerable to swapping this service-owned
        // directory for a symlink to an unrelated directory.
        let quarantine_fd = open_directory_nofollow(&quarantine_root).map_err(|error| {
            ReconcileError::Filesystem(format!(
                "cannot open reconciler quarantine without following symlinks: {error}"
            ))
        })?;
        // SAFETY: quarantine_fd refers to the opened directory and remains
        // alive for the duration of the fchmod call.
        if unsafe { libc::fchmod(quarantine_fd.as_raw_fd(), 0o700) } < 0 {
            return Err(ReconcileError::Filesystem(format!(
                "cannot secure reconciler quarantine permissions: {}",
                errno_result()
            )));
        }
    }
    // Re-check after creation/chmod. The rename and recovery helpers perform
    // another no-follow open immediately before use, closing the remaining
    // path-level race without trusting this metadata check.
    let metadata = fs::symlink_metadata(&quarantine_root).map_err(|error| {
        ReconcileError::Filesystem(format!("cannot revalidate reconciler quarantine: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ReconcileError::Filesystem(
            "reconciler quarantine changed into a non-directory".to_owned(),
        ));
    }
    Ok(quarantine_root)
}

fn quarantine_name(identity: &GenerationIdentity) -> String {
    // Simple UUIDs keep this a fixed, unambiguous four-field format while
    // retaining the identity needed for crash recovery and reference checks.
    format!(
        "q-{}-{}-{}-{}",
        identity.run_id.simple(),
        identity.code_commit,
        identity.publication_id.simple(),
        Uuid::new_v4().simple()
    )
}

fn parse_quarantine_name(name: &std::ffi::OsStr) -> Option<GenerationIdentity> {
    let name = name.to_str()?;
    let mut fields = name.split('-');
    if fields.next()? != "q" {
        return None;
    }
    let run_id = Uuid::parse_str(fields.next()?).ok()?;
    let code_commit = fields.next()?.to_owned();
    let publication_id = Uuid::parse_str(fields.next()?).ok()?;
    let _nonce = Uuid::parse_str(fields.next()?).ok()?;
    if fields.next().is_some() || publication_id.is_nil() || !is_exact_code_commit(&code_commit) {
        return None;
    }
    Some(GenerationIdentity {
        run_id,
        code_commit,
        publication_id,
    })
}

fn process_quarantine_entry(
    artifact_root: &Path,
    quarantine_root: &Path,
    snapshot: &ReconcileDbSnapshot,
    cutoff: SystemTime,
    report: &mut ReconcileReport,
    name: &std::ffi::OsStr,
    modified: Option<SystemTime>,
) {
    let Some(identity) = parse_quarantine_name(name) else {
        // Unknown names are never deletion candidates. This makes the
        // directory safe to inspect after a partial write or an upgrade.
        report.malformed_skipped += 1;
        return;
    };
    report.scanned_generations += 1;
    if snapshot.referenced_generations.contains(&identity) {
        report.referenced_retained += 1;
        return;
    }
    if snapshot.active_runs.contains(&identity.run_id) {
        report.active_retained += 1;
        return;
    }
    let Some(modified) = modified else {
        report.fresh_retained += 1;
        return;
    };
    if modified > cutoff {
        report.fresh_retained += 1;
        return;
    }
    match remove_quarantined_generation(artifact_root, quarantine_root, name) {
        Ok(RemoveResult::Deleted) => report.deleted_generations += 1,
        Ok(RemoveResult::SkippedSymlink) => report.symlink_skipped += 1,
        Ok(RemoveResult::SkippedMalformed) => report.malformed_skipped += 1,
        Err(_) => report.deletion_errors += 1,
    }
}

#[cfg(unix)]
fn reconcile_quarantine(
    artifact_root: &Path,
    quarantine_root: &Path,
    snapshot: &ReconcileDbSnapshot,
    cutoff: SystemTime,
    report: &mut ReconcileReport,
) -> Result<(), ReconcileError> {
    let quarantine_fd =
        open_directory_relative(artifact_root, quarantine_root).map_err(|error| {
            ReconcileError::Filesystem(format!("cannot scan reconciler quarantine: {error}"))
        })?;
    for name in read_directory_names(quarantine_fd.as_raw_fd()).map_err(|error| {
        ReconcileError::Filesystem(format!("cannot read reconciler quarantine: {error}"))
    })? {
        let kind = match classify_at(quarantine_fd.as_raw_fd(), &name) {
            Ok(kind) => kind,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        if kind == NodeKind::Symlink {
            report.symlink_skipped += 1;
            continue;
        }
        let modified = if kind == NodeKind::Directory {
            open_directory_at(quarantine_fd.as_raw_fd(), &name)
                .ok()
                .and_then(|entry| modified_from_fd(entry.as_raw_fd()).ok())
        } else {
            None
        };
        process_quarantine_entry(
            artifact_root,
            quarantine_root,
            snapshot,
            cutoff,
            report,
            &name,
            modified,
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn reconcile_quarantine(
    artifact_root: &Path,
    quarantine_root: &Path,
    snapshot: &ReconcileDbSnapshot,
    cutoff: SystemTime,
    report: &mut ReconcileReport,
) -> Result<(), ReconcileError> {
    let entries = fs::read_dir(quarantine_root).map_err(|error| {
        ReconcileError::Filesystem(format!("cannot scan reconciler quarantine: {error}"))
    })?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        let entry_path = entry.path();
        let metadata = match fs::symlink_metadata(&entry_path) {
            Ok(metadata) => metadata,
            Err(_) => {
                report.malformed_skipped += 1;
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            report.symlink_skipped += 1;
            continue;
        }
        process_quarantine_entry(
            artifact_root,
            quarantine_root,
            snapshot,
            cutoff,
            report,
            &entry.file_name(),
            metadata.modified().ok(),
        );
    }
    Ok(())
}

#[cfg(test)]
fn invoke_before_quarantine_hook(path: &Path) {
    let hook = BEFORE_QUARANTINE_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("quarantine hook mutex")
        .clone();
    if let Some(hook) = hook {
        hook(path);
    }
}

#[cfg(test)]
fn set_before_quarantine_hook(hook: Option<BeforeQuarantineHook>) {
    *BEFORE_QUARANTINE_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("quarantine hook mutex") = hook;
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Directory,
    Regular,
    Symlink,
    Other,
}

#[cfg(unix)]
fn errno_result() -> std::io::Error {
    std::io::Error::last_os_error()
}

#[cfg(unix)]
fn c_name(name: &OsStr) -> Result<CString, std::io::Error> {
    CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "directory entry contains an interior NUL",
        )
    })
}

#[cfg(unix)]
fn c_path(path: &Path) -> Result<CString, std::io::Error> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path contains an interior NUL",
        )
    })
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<OwnedFd, std::io::Error> {
    let path = c_path(path)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: `path` is a NUL-terminated owned CString and the flags request
    // a directory handle without following a final symlink.
    let fd = unsafe { libc::open(path.as_ptr(), flags) };
    if fd < 0 {
        Err(errno_result())
    } else {
        // SAFETY: fd is freshly returned by open and is owned by this value.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_directory_at(parent: RawFd, name: &OsStr) -> Result<OwnedFd, std::io::Error> {
    let name = c_name(name)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    // SAFETY: parent is an open directory fd and name is a NUL-terminated
    // directory entry. O_NOFOLLOW prevents a swapped child symlink.
    let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if fd < 0 {
        Err(errno_result())
    } else {
        // SAFETY: fd is freshly returned by openat and is owned by this value.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_directory_relative(root: &Path, path: &Path) -> Result<OwnedFd, std::io::Error> {
    let relative = path.strip_prefix(root).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "reconciler path is outside its artifact root",
        )
    })?;
    let mut current = open_directory_nofollow(root)?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "reconciler path contains a non-normal component",
            ));
        };
        current = open_directory_at(current.as_raw_fd(), name)?;
    }
    Ok(current)
}

#[cfg(unix)]
fn open_parent_relative(root: &Path, path: &Path) -> Result<OwnedFd, std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "reconciler candidate has no parent",
        )
    })?;
    open_directory_relative(root, parent)
}

#[cfg(unix)]
fn stat_mode(mode: libc::mode_t) -> NodeKind {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => NodeKind::Directory,
        libc::S_IFREG => NodeKind::Regular,
        libc::S_IFLNK => NodeKind::Symlink,
        _ => NodeKind::Other,
    }
}

#[cfg(unix)]
fn classify_fd(fd: RawFd) -> Result<NodeKind, std::io::Error> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage and fd is borrowed for the call.
    let result = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
    if result < 0 {
        return Err(errno_result());
    }
    // SAFETY: fstat initialized stat on success.
    Ok(stat_mode(unsafe { stat.assume_init() }.st_mode))
}

#[cfg(unix)]
fn modified_from_fd(fd: RawFd) -> Result<SystemTime, std::io::Error> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(errno_result());
    }
    // SAFETY: duplicate is a fresh fd owned by this File value.
    let file = unsafe { std::fs::File::from_raw_fd(duplicate) };
    file.metadata()?.modified()
}

#[cfg(unix)]
fn classify_at(parent: RawFd, name: &OsStr) -> Result<NodeKind, std::io::Error> {
    let name = c_name(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage; AT_SYMLINK_NOFOLLOW means the
    // final entry is classified without resolving a symlink.
    let result = unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result < 0 {
        return Err(errno_result());
    }
    // SAFETY: fstatat initialized stat on success.
    Ok(stat_mode(unsafe { stat.assume_init() }.st_mode))
}

#[cfg(unix)]
fn same_object(fd: RawFd, parent: RawFd, name: &OsStr) -> Result<bool, std::io::Error> {
    let name = c_name(name)?;
    let mut first = std::mem::MaybeUninit::<libc::stat>::uninit();
    let mut second = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: both stat buffers are writable and the fds are borrowed.
    if unsafe { libc::fstat(fd, first.as_mut_ptr()) } < 0 {
        return Err(errno_result());
    }
    // SAFETY: AT_SYMLINK_NOFOLLOW classifies the current directory entry.
    if unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            second.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        return Err(errno_result());
    }
    // SAFETY: both stat calls succeeded and initialized their values.
    let (first, second) = unsafe { (first.assume_init(), second.assume_init()) };
    Ok(first.st_dev == second.st_dev && first.st_ino == second.st_ino)
}

#[cfg(unix)]
fn read_directory_names(fd: RawFd) -> Result<Vec<std::ffi::OsString>, std::io::Error> {
    // fdopendir takes ownership, so duplicate the handle first. The original
    // remains available for all fd-relative operations in the caller.
    // Duplicating a directory fd shares its seek position; rewind it so a
    // prior validation pass cannot make a later removal pass appear empty.
    // SAFETY: fd is an open directory handle.
    if unsafe { libc::lseek(fd, 0, libc::SEEK_SET) } < 0 {
        return Err(errno_result());
    }
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(errno_result());
    }
    // SAFETY: duplicate is a valid directory fd and ownership transfers to
    // the DIR stream on success.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        // SAFETY: fdopendir failed, so duplicate remains ours.
        unsafe { libc::close(duplicate) };
        return Err(errno_result());
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: stream is valid until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: d_name is a NUL-terminated field supplied by readdir.
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes != b"." && bytes != b".." {
            names.push(std::ffi::OsString::from_vec(bytes.to_vec()));
        }
    }
    // SAFETY: stream is valid and closes the duplicate fd it owns.
    if unsafe { libc::closedir(stream) } < 0 {
        return Err(errno_result());
    }
    Ok(names)
}

#[cfg(unix)]
fn validate_tree_fd(fd: RawFd) -> Result<RemoveResult, std::io::Error> {
    match classify_fd(fd)? {
        NodeKind::Regular => return Ok(RemoveResult::Deleted),
        NodeKind::Directory => {}
        NodeKind::Symlink => return Ok(RemoveResult::SkippedSymlink),
        NodeKind::Other => return Ok(RemoveResult::SkippedMalformed),
    }
    for name in read_directory_names(fd)? {
        match classify_at(fd, &name)? {
            NodeKind::Regular => {}
            NodeKind::Directory => {
                let child = open_directory_at(fd, &name)?;
                match validate_tree_fd(child.as_raw_fd())? {
                    RemoveResult::Deleted => {}
                    result => return Ok(result),
                }
            }
            NodeKind::Symlink => return Ok(RemoveResult::SkippedSymlink),
            NodeKind::Other => return Ok(RemoveResult::SkippedMalformed),
        }
    }
    Ok(RemoveResult::Deleted)
}

#[cfg(unix)]
fn validate_generation_fd(fd: RawFd) -> Result<RemoveResult, std::io::Error> {
    if classify_fd(fd)? != NodeKind::Directory {
        return Ok(RemoveResult::SkippedMalformed);
    }
    match classify_at(fd, OsStr::new("artifacts"))? {
        NodeKind::Directory => validate_tree_fd(fd),
        NodeKind::Symlink => Ok(RemoveResult::SkippedSymlink),
        NodeKind::Regular | NodeKind::Other => Ok(RemoveResult::SkippedMalformed),
    }
}

#[cfg(unix)]
fn validate_generation_at(root: &Path, path: &Path) -> Result<RemoveResult, std::io::Error> {
    let parent = open_parent_relative(root, path)?;
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "reconciler candidate has no name",
        )
    })?;
    match classify_at(parent.as_raw_fd(), name)? {
        NodeKind::Directory => {
            let candidate = open_directory_at(parent.as_raw_fd(), name)?;
            validate_generation_fd(candidate.as_raw_fd())
        }
        NodeKind::Symlink => Ok(RemoveResult::SkippedSymlink),
        NodeKind::Regular | NodeKind::Other => Ok(RemoveResult::SkippedMalformed),
    }
}

#[cfg(not(unix))]
fn validate_generation_at(_root: &Path, path: &Path) -> Result<RemoveResult, std::io::Error> {
    // Windows removal is intentionally disabled below unless a future
    // implementation can provide equivalent handle-relative no-follow
    // primitives. Inspection remains conservative and never follows a final
    // symlink.
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(RemoveResult::SkippedSymlink);
    }
    if !metadata.is_dir() {
        return Ok(RemoveResult::SkippedMalformed);
    }
    let artifacts = path.join("artifacts");
    let artifacts_metadata = fs::symlink_metadata(artifacts)?;
    if artifacts_metadata.file_type().is_symlink() {
        return Ok(RemoveResult::SkippedSymlink);
    }
    if !artifacts_metadata.is_dir() {
        return Ok(RemoveResult::SkippedMalformed);
    }
    Ok(RemoveResult::Deleted)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_noreplace_at(
    source_parent: RawFd,
    source_name: &CStr,
    destination_parent: RawFd,
    destination_name: &CStr,
) -> Result<(), std::io::Error> {
    // Calling the kernel entry point avoids a dependency on a libc
    // `renameat2` symbol, which glibc exports but musl does not. The argument
    // types match renameat2(2); both paths are NUL-terminated borrowed C
    // strings and both descriptors remain open for the duration of the call.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2 as libc::c_long,
            source_parent as libc::c_int,
            source_name.as_ptr(),
            destination_parent as libc::c_int,
            destination_name.as_ptr(),
            libc::RENAME_NOREPLACE as libc::c_uint,
        )
    };
    if result == 0 {
        return Ok(());
    }

    let error = errno_result();
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        // Never degrade to renameat after probing the destination: that would
        // reintroduce a check/rename race and could overwrite quarantine state.
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("atomic no-replace rename is unavailable: {error}"),
        ));
    }
    Err(error)
}

#[cfg(unix)]
fn move_into_quarantine(
    artifact_root: &Path,
    source: &Path,
    quarantine_root: &Path,
    name: &str,
) -> Result<(), std::io::Error> {
    let source_parent = open_parent_relative(artifact_root, source)?;
    let quarantine_parent = open_directory_relative(artifact_root, quarantine_root)?;
    let source_name = c_name(source.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "candidate has no name")
    })?)?;
    let quarantine_name = CString::new(name).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "quarantine name contains NUL",
        )
    })?;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // renameat2(RENAME_NOREPLACE) makes the random quarantine name an
        // actual no-overwrite boundary rather than a probabilistic promise.
        rename_noreplace_at(
            source_parent.as_raw_fd(),
            source_name.as_c_str(),
            quarantine_parent.as_raw_fd(),
            quarantine_name.as_c_str(),
        )
    }

    #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
    {
        // The destination directory is mode 0700 and the name is a fresh UUID.
        // Refuse an observed collision; unlike `fs::rename`, renameat keeps
        // the source and destination anchored to directory handles.
        if classify_at(quarantine_parent.as_raw_fd(), OsStr::new(name)).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "quarantine name already exists",
            ));
        }
        // SAFETY: both fds are open directories; names are owned C strings.
        let result = unsafe {
            libc::renameat(
                source_parent.as_raw_fd(),
                source_name.as_ptr(),
                quarantine_parent.as_raw_fd(),
                quarantine_name.as_ptr(),
            )
        };
        if result < 0 {
            return Err(errno_result());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
fn move_into_quarantine(
    _artifact_root: &Path,
    source: &Path,
    quarantine_root: &Path,
    name: &str,
) -> Result<(), std::io::Error> {
    fs::rename(source, quarantine_root.join(name))
}

#[cfg(unix)]
fn unlink_at(parent: RawFd, name: &OsStr, flags: libc::c_int) -> Result<(), std::io::Error> {
    let name = c_name(name)?;
    // SAFETY: parent is an open directory; unlinkat never resolves a child
    // symlink when removing the entry itself.
    if unsafe { libc::unlinkat(parent, name.as_ptr(), flags) } < 0 {
        Err(errno_result())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn remove_tree_fd(fd: RawFd) -> Result<RemoveResult, std::io::Error> {
    if classify_fd(fd)? != NodeKind::Directory {
        return Ok(RemoveResult::SkippedMalformed);
    }
    for name in read_directory_names(fd)? {
        match classify_at(fd, &name)? {
            NodeKind::Symlink => return Ok(RemoveResult::SkippedSymlink),
            NodeKind::Regular => {
                // Recheck immediately before unlink. If a concurrent swap
                // turns this into a symlink, retain the quarantine object.
                if classify_at(fd, &name)? != NodeKind::Regular {
                    return Ok(RemoveResult::SkippedSymlink);
                }
                unlink_at(fd, &name, 0)?;
            }
            NodeKind::Directory => {
                let child = open_directory_at(fd, &name)?;
                match validate_tree_fd(child.as_raw_fd())? {
                    RemoveResult::Deleted => {}
                    result => return Ok(result),
                }
                match remove_tree_fd(child.as_raw_fd())? {
                    RemoveResult::Deleted => {}
                    result => return Ok(result),
                }
                // AT_REMOVEDIR cannot follow a swapped symlink and therefore
                // fails closed if the directory entry changed after open.
                unlink_at(fd, &name, libc::AT_REMOVEDIR)?;
            }
            NodeKind::Other => return Ok(RemoveResult::SkippedMalformed),
        }
    }
    Ok(RemoveResult::Deleted)
}

#[cfg(unix)]
fn remove_quarantined_generation(
    artifact_root: &Path,
    quarantine_root: &Path,
    name: &std::ffi::OsStr,
) -> Result<RemoveResult, std::io::Error> {
    let quarantine_parent = open_directory_relative(artifact_root, quarantine_root)?;
    match classify_at(quarantine_parent.as_raw_fd(), name)? {
        NodeKind::Symlink => return Ok(RemoveResult::SkippedSymlink),
        NodeKind::Directory => {}
        NodeKind::Regular | NodeKind::Other => return Ok(RemoveResult::SkippedMalformed),
    }
    let candidate = open_directory_at(quarantine_parent.as_raw_fd(), name)?;
    match validate_generation_fd(candidate.as_raw_fd())? {
        RemoveResult::Deleted => {}
        result => return Ok(result),
    }
    match remove_tree_fd(candidate.as_raw_fd())? {
        RemoveResult::Deleted => {}
        result => return Ok(result),
    }
    if !same_object(candidate.as_raw_fd(), quarantine_parent.as_raw_fd(), name)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "quarantined object changed during removal",
        ));
    }
    unlink_at(quarantine_parent.as_raw_fd(), name, libc::AT_REMOVEDIR)?;
    Ok(RemoveResult::Deleted)
}

#[cfg(not(unix))]
fn remove_quarantined_generation(
    _artifact_root: &Path,
    _quarantine_root: &Path,
    _name: &std::ffi::OsStr,
) -> Result<RemoveResult, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "reconciler no-follow removal is unavailable on this platform",
    ))
}

impl fmt::Display for GenerationIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.run_id, self.code_commit, self.publication_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::path::PathBuf;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn generation(root: &Path, run: Uuid, commit: &str, publication: Uuid) -> PathBuf {
        root.join(run.to_string())
            .join("generations")
            .join(commit)
            .join(publication.to_string())
    }

    fn make_generation(root: &Path, run: Uuid, commit: &str, publication: Uuid) -> PathBuf {
        let path = generation(root, run, commit, publication);
        fs::create_dir_all(path.join("artifacts")).unwrap();
        fs::write(path.join("artifacts/equity.parquet"), b"immutable").unwrap();
        path
    }

    fn old(path: &Path) {
        let old = SystemTime::now() - Duration::from_secs(3_600);
        let file = OpenOptions::new()
            .write(true)
            .open(path.join("artifacts/equity.parquet"))
            .unwrap();
        file.set_len(9).unwrap();
        file.set_modified(old).unwrap();
        fs::File::open(path).unwrap();
        // Directory mtime is what the reconciler uses; touching a child alone
        // is not portable, so use a platform-specific utimensat when present.
        #[cfg(unix)]
        {
            let seconds = old
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() as libc::time_t;
            let times = [
                libc::timespec {
                    tv_sec: seconds,
                    tv_nsec: 0,
                },
                libc::timespec {
                    tv_sec: seconds,
                    tv_nsec: 0,
                },
            ];
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            let _ = unsafe { libc::utimensat(libc::AT_FDCWD, name.as_ptr(), times.as_ptr(), 0) };
        }
    }

    #[test]
    fn referenced_and_active_generations_are_retained() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let referenced_id = Uuid::new_v4();
        let active_id = Uuid::new_v4();
        let referenced = make_generation(root.path(), run, COMMIT, referenced_id);
        let active = make_generation(root.path(), run, COMMIT, active_id);
        old(&referenced);
        old(&active);
        let mut snapshot = ReconcileDbSnapshot::default();
        snapshot.referenced_generations.insert(GenerationIdentity {
            run_id: run,
            code_commit: COMMIT.to_owned(),
            publication_id: referenced_id,
        });
        snapshot.active_runs.insert(run);
        let report = reconcile_filesystem(
            root.path(),
            &snapshot,
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 0);
        assert!(referenced.exists());
        assert!(active.exists());
    }

    #[test]
    fn fresh_orphan_is_retained_and_old_orphan_is_deleted() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let fresh = make_generation(root.path(), run, COMMIT, Uuid::new_v4());
        let old_path = make_generation(root.path(), run, COMMIT, Uuid::new_v4());
        old(&old_path);
        let report = reconcile_filesystem(
            root.path(),
            &ReconcileDbSnapshot::default(),
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 1);
        assert!(fresh.exists());
        assert!(!old_path.exists());
    }

    #[test]
    fn malformed_and_symlink_generations_are_skipped() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let malformed = root.path().join(run.to_string()).join("generations/bad");
        fs::create_dir_all(&malformed).unwrap();
        let target = make_generation(root.path(), run, COMMIT, Uuid::new_v4());
        old(&target);
        #[cfg(unix)]
        {
            let link = root.path().join(run.to_string()).join("generations/link");
            std::os::unix::fs::symlink(&target, link).unwrap();
        }
        let report = reconcile_filesystem(
            root.path(),
            &ReconcileDbSnapshot::default(),
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 1);
        assert!(!target.exists());
        assert!(malformed.exists());
        #[cfg(unix)]
        assert!(
            fs::symlink_metadata(root.path().join(run.to_string()).join("generations/link"))
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn internal_symlink_prevents_generation_deletion() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let target = make_generation(root.path(), run, COMMIT, Uuid::new_v4());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"do not touch").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            target.join("artifacts/escape"),
        )
        .unwrap();
        old(&target);
        let report = reconcile_filesystem(
            root.path(),
            &ReconcileDbSnapshot::default(),
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 0);
        assert!(target.exists());
        assert!(outside.path().join("secret").exists());
    }

    #[test]
    fn quarantine_recovery_deletes_an_old_entry_after_a_crash() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let identity = GenerationIdentity {
            run_id: run,
            code_commit: COMMIT.to_owned(),
            publication_id: publication,
        };
        let generation = make_generation(root.path(), run, COMMIT, publication);
        let quarantine = ensure_quarantine_root(root.path()).unwrap();
        let name = quarantine_name(&identity);
        let quarantined = quarantine.join(&name);
        fs::rename(&generation, &quarantined).unwrap();
        old(&quarantined);

        let report = reconcile_filesystem(
            root.path(),
            &ReconcileDbSnapshot::default(),
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 1);
        assert!(!quarantined.exists());
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_move_preserves_the_same_directory_entry() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().unwrap();
        let source = make_generation(root.path(), Uuid::new_v4(), COMMIT, Uuid::new_v4());
        let quarantine = ensure_quarantine_root(root.path()).unwrap();
        let before = fs::symlink_metadata(&source).unwrap();
        let name = "successful-no-replace-move";

        move_into_quarantine(root.path(), &source, &quarantine, name).unwrap();

        let destination = quarantine.join(name);
        let after = fs::symlink_metadata(&destination).unwrap();
        assert!(!source.exists());
        assert_eq!(before.dev(), after.dev());
        assert_eq!(before.ino(), after.ino());
        assert_eq!(
            fs::read(destination.join("artifacts/equity.parquet")).unwrap(),
            b"immutable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn quarantine_move_never_overwrites_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = make_generation(root.path(), Uuid::new_v4(), COMMIT, Uuid::new_v4());
        let quarantine = ensure_quarantine_root(root.path()).unwrap();
        let name = "existing-quarantine-entry";
        let destination = quarantine.join(name);
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("sentinel"), b"keep-existing").unwrap();

        let error = move_into_quarantine(root.path(), &source, &quarantine, name).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(source.exists());
        assert_eq!(
            fs::read(destination.join("sentinel")).unwrap(),
            b"keep-existing"
        );
    }

    #[test]
    fn quarantine_recovery_retains_referenced_and_active_entries() {
        let root = tempfile::tempdir().unwrap();
        let run = Uuid::new_v4();
        let referenced_id = Uuid::new_v4();
        let active_id = Uuid::new_v4();
        let quarantine = ensure_quarantine_root(root.path()).unwrap();
        let referenced_identity = GenerationIdentity {
            run_id: run,
            code_commit: COMMIT.to_owned(),
            publication_id: referenced_id,
        };
        let active_identity = GenerationIdentity {
            run_id: run,
            code_commit: COMMIT.to_owned(),
            publication_id: active_id,
        };
        let referenced = make_generation(root.path(), run, COMMIT, referenced_id);
        let active = make_generation(root.path(), run, COMMIT, active_id);
        let referenced_quarantine = quarantine.join(quarantine_name(&referenced_identity));
        let active_quarantine = quarantine.join(quarantine_name(&active_identity));
        fs::rename(referenced, &referenced_quarantine).unwrap();
        fs::rename(active, &active_quarantine).unwrap();
        old(&referenced_quarantine);
        old(&active_quarantine);

        let mut snapshot = ReconcileDbSnapshot::default();
        snapshot.referenced_generations.insert(referenced_identity);
        snapshot.active_runs.insert(run);
        let report = reconcile_filesystem(
            root.path(),
            &snapshot,
            ReconcilerConfig {
                safe_grace: Duration::from_secs(1),
            },
        )
        .unwrap();
        assert_eq!(report.deleted_generations, 0);
        assert!(referenced_quarantine.exists());
        assert!(active_quarantine.exists());
    }

    #[cfg(unix)]
    #[test]
    fn synchronized_symlink_swap_is_quarantined_without_touching_outside() {
        use std::sync::mpsc;
        use std::thread;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_sentinel = outside.path().join("sentinel");
        fs::write(&outside_sentinel, b"must survive").unwrap();
        let run = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let candidate = make_generation(root.path(), run, COMMIT, publication);
        old(&candidate);

        let (validated_tx, validated_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let release_rx = Arc::new(Mutex::new(release_rx));
        let expected_candidate = candidate.clone();
        let hook_release_rx = Arc::clone(&release_rx);
        set_before_quarantine_hook(Some(Arc::new(move |path| {
            if path == expected_candidate {
                validated_tx.send(()).unwrap();
                hook_release_rx.lock().unwrap().recv().unwrap();
            }
        })));

        let root_path = root.path().to_path_buf();
        let reconcile_thread = thread::spawn(move || {
            reconcile_filesystem(
                &root_path,
                &ReconcileDbSnapshot::default(),
                ReconcilerConfig {
                    safe_grace: Duration::from_secs(1),
                },
            )
        });
        validated_rx.recv().unwrap();

        // This is the race window that used to make recursive path deletion
        // follow the outside directory. The atomic rename moves this symlink
        // as an inert directory entry into quarantine; no read_dir call ever
        // resolves it.
        let parked_artifacts = candidate.with_file_name("artifacts.parked");
        fs::rename(candidate.join("artifacts"), &parked_artifacts).unwrap();
        std::os::unix::fs::symlink(outside.path(), candidate.join("artifacts")).unwrap();
        release_tx.send(()).unwrap();

        let report = reconcile_thread.join().unwrap().unwrap();
        set_before_quarantine_hook(None);
        assert_eq!(report.deleted_generations, 0);
        assert_eq!(fs::read(&outside_sentinel).unwrap(), b"must survive");
        assert!(!candidate.exists());
        let quarantine = root.path().join(QUARANTINE_DIRECTORY);
        let quarantined = fs::read_dir(quarantine)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .expect("symlink-swapped generation in quarantine");
        assert!(
            fs::symlink_metadata(quarantined.join("artifacts"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn generation_reference_parser_rejects_unsafe_paths() {
        let run = Uuid::new_v4();
        let publication = Uuid::new_v4();
        let valid = format!(
            "backtest/runs/{run}/generations/{COMMIT}/{publication}/artifacts/equity.parquet"
        );
        assert!(parse_generation_reference(&valid).unwrap().is_some());
        assert!(
            parse_generation_reference(&format!("/srv/lagrange/data/{valid}"))
                .unwrap()
                .is_some()
        );
        assert!(
            parse_generation_reference("backtest/runs/foo/generations/x/y/artifacts/a").is_err()
        );
        assert!(
            parse_generation_reference("backtest/runs/{run}/artifacts/equity.parquet")
                .unwrap()
                .is_none()
        );
    }
}
