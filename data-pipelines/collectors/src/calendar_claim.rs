//! Durable, one-attempt-per-day guard for the operator calendar bootstrap.
//! A consumed claim without committed Raw is indeterminate and cannot recapture.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use domain::{BatchId, TradingDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("calendar bootstrap state invalid or busy")]
pub(crate) struct ClaimError;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    schema_version: u32,
    date: TradingDate,
    source_batch_id: BatchId,
}

pub(crate) struct CalendarDayLock {
    directory: PathBuf,
    _lock: File,
    date: TradingDate,
}

impl CalendarDayLock {
    pub(crate) fn acquire(raw_root: &Path, date: TradingDate) -> Result<Self, ClaimError> {
        validate_directory_chain(raw_root)?;
        let directory = raw_root.join(".calendar-bootstrap");
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| ClaimError)?;
                File::open(raw_root)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| ClaimError)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ClaimError),
        }
        let metadata = std::fs::symlink_metadata(&directory).map_err(|_| ClaimError)?;
        if !metadata.is_dir()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o7777 != 0o700
        {
            return Err(ClaimError);
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(directory.join("calendar.lock"))
            .map_err(|_| ClaimError)?;
        validate_file(&lock)?;
        lock.try_lock().map_err(|_| ClaimError)?;
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| ClaimError)?;
        Ok(Self {
            directory,
            _lock: lock,
            date,
        })
    }

    pub(crate) fn existing(&self) -> Result<Option<BatchId>, ClaimError> {
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.claim_path())
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ClaimError),
        };
        validate_file(&file)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| ClaimError)?;
        if bytes.len() > 4096 {
            return Err(ClaimError);
        }
        let claim: Claim = serde_json::from_slice(&bytes).map_err(|_| ClaimError)?;
        if claim.schema_version != 1 || claim.date != self.date {
            return Err(ClaimError);
        }
        Ok(Some(claim.source_batch_id))
    }

    /// Must be durable before the sole provider invocation. Never overwrite.
    pub(crate) fn consume(&self, source_batch_id: BatchId) -> Result<(), ClaimError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.claim_path())
            .map_err(|_| ClaimError)?;
        let bytes = serde_json::to_vec(&Claim {
            schema_version: 1,
            date: self.date,
            source_batch_id,
        })
        .map_err(|_| ClaimError)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| ClaimError)?;
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| ClaimError)
    }

    fn claim_path(&self) -> PathBuf {
        self.directory.join(format!("{}.json", self.date.to_iso()))
    }
}

fn effective_uid() -> u32 {
    // getuid has no memory or filesystem side effects.
    unsafe { libc::geteuid() }
}

fn validate_file(file: &File) -> Result<(), ClaimError> {
    let metadata = file.metadata().map_err(|_| ClaimError)?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(ClaimError);
    }
    Ok(())
}

fn validate_directory_chain(path: &Path) -> Result<(), ClaimError> {
    if !path.is_absolute() {
        return Err(ClaimError);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => {}
            _ => return Err(ClaimError),
        }
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| ClaimError)?;
        if !metadata.is_dir() {
            return Err(ClaimError);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumed_attempt_survives_restart_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let date = TradingDate::parse("2026-09-14").unwrap();
        let batch = BatchId::generate();
        let guard = CalendarDayLock::acquire(root.path(), date).unwrap();
        assert_eq!(guard.existing().unwrap(), None);
        guard.consume(batch).unwrap();
        assert!(guard.consume(BatchId::generate()).is_err());
        assert!(CalendarDayLock::acquire(root.path(), date).is_err());
        drop(guard);
        let replay = CalendarDayLock::acquire(root.path(), date).unwrap();
        assert_eq!(replay.existing().unwrap(), Some(batch));
    }

    #[test]
    fn malformed_or_linked_claim_is_not_an_unused_allowance() {
        let root = tempfile::tempdir().unwrap();
        let date = TradingDate::parse("2026-09-14").unwrap();
        let guard = CalendarDayLock::acquire(root.path(), date).unwrap();
        guard.consume(BatchId::generate()).unwrap();
        std::fs::write(guard.claim_path(), b"{bad").unwrap();
        assert!(guard.existing().is_err());
        std::fs::remove_file(guard.claim_path()).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), guard.claim_path()).unwrap();
        assert!(guard.existing().is_err());
    }
}
