//! Immutable, provider-free Owner Equity V2 candidate artifacts.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use domain::ContentHash;
use market_data::owner_equity_v2::OwnerEquityGenerationCandidate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const ARTIFACT_VERSION: &str = "owner-equity-v2-artifact-v1";
const ARTIFACT_DIRECTORY: &str = "owner-equity-v2";
const CANDIDATE_FILE: &str = "candidate.json";
const MANIFEST_FILE: &str = "manifest.json";
const MAX_CANDIDATE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone)]
pub struct OwnerEquityArtifactInput<'a> {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation: u64,
    pub candidate: &'a OwnerEquityGenerationCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOwnerEquityArtifact {
    pub owner_user_id: Uuid,
    pub membership_id: Uuid,
    pub generation: u64,
    pub candidate: OwnerEquityGenerationCandidate,
    pub candidate_sha256: ContentHash,
    pub manifest_sha256: ContentHash,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactManifest {
    artifact_version: String,
    owner_user_id: Uuid,
    membership_id: Uuid,
    generation: u64,
    instrument_id: String,
    candidate_sha256: ContentHash,
    candidate_size_bytes: u64,
    raw_manifest_sha256: ContentHash,
    entitlement_sha256: ContentHash,
    capture_code_commit: String,
    materializer_code_commit: String,
    prior_candidate_sha256: Option<ContentHash>,
    prior_artifact_manifest_sha256: Option<ContentHash>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OwnerEquityArtifactError {
    #[error("owner equity artifact root is unsafe")]
    UnsafeRoot,
    #[error("owner equity artifact permissions are unsafe")]
    UnsafePermissions,
    #[error("owner equity artifact write failed")]
    WriteFailed,
    #[error("owner equity artifact is missing")]
    Missing,
    #[error("owner equity artifact was tampered")]
    Tampered,
    #[error("owner equity artifact conflicts with immutable content")]
    Conflict,
    #[error("owner equity artifact candidate is invalid")]
    CandidateInvalid,
}

pub fn write_owner_equity_artifact(
    root: &Path,
    input: OwnerEquityArtifactInput<'_>,
) -> Result<VerifiedOwnerEquityArtifact, OwnerEquityArtifactError> {
    validate_root(root)?;
    let candidate_bytes = input
        .candidate
        .canonical_bytes()
        .map_err(|_| OwnerEquityArtifactError::CandidateInvalid)?;
    if candidate_bytes.len() as u64 > MAX_CANDIDATE_BYTES {
        return Err(OwnerEquityArtifactError::CandidateInvalid);
    }
    let candidate_sha256 = ContentHash::from_bytes(&candidate_bytes);
    let manifest = ArtifactManifest {
        artifact_version: ARTIFACT_VERSION.to_owned(),
        owner_user_id: input.owner_user_id,
        membership_id: input.membership_id,
        generation: input.generation,
        instrument_id: input.candidate.instrument_id.to_string(),
        candidate_sha256: candidate_sha256.clone(),
        candidate_size_bytes: candidate_bytes.len() as u64,
        raw_manifest_sha256: input.candidate.source_pins.raw_manifest_sha256.clone(),
        entitlement_sha256: input.candidate.source_pins.entitlement_sha256.clone(),
        capture_code_commit: input.candidate.source_pins.capture_code_commit.to_string(),
        materializer_code_commit: input
            .candidate
            .source_pins
            .materializer_code_commit
            .to_string(),
        prior_candidate_sha256: input.candidate.source_pins.prior_candidate_sha256.clone(),
        prior_artifact_manifest_sha256: input
            .candidate
            .source_pins
            .prior_artifact_manifest_sha256
            .clone(),
    };
    let manifest_bytes =
        serde_json::to_vec(&manifest).map_err(|_| OwnerEquityArtifactError::CandidateInvalid)?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(OwnerEquityArtifactError::CandidateInvalid);
    }
    let manifest_sha256 = ContentHash::from_bytes(&manifest_bytes);
    let base = artifact_base(root)?;
    let destination = base.join(manifest_sha256.as_str().trim_start_matches("sha256:"));
    if destination.exists() {
        let verified = read_exact(&destination, &manifest_sha256)
            .map_err(|_| OwnerEquityArtifactError::Conflict)?;
        if verified.candidate != *input.candidate {
            return Err(OwnerEquityArtifactError::Conflict);
        }
        return Ok(VerifiedOwnerEquityArtifact {
            replayed: true,
            ..verified
        });
    }

    let staging = base.join(format!(".staging-{}", Uuid::new_v4().simple()));
    create_private_directory(&staging)?;
    let write_result = (|| {
        write_private_file(&staging.join(CANDIDATE_FILE), &candidate_bytes)?;
        write_private_file(&staging.join(MANIFEST_FILE), &manifest_bytes)?;
        File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(|_| OwnerEquityArtifactError::WriteFailed)?;
        fs::rename(&staging, &destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                OwnerEquityArtifactError::Conflict
            } else {
                OwnerEquityArtifactError::WriteFailed
            }
        })?;
        File::open(&base)
            .and_then(|file| file.sync_all())
            .map_err(|_| OwnerEquityArtifactError::WriteFailed)
    })();
    if write_result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    write_result?;
    let verified = read_exact(&destination, &manifest_sha256)?;
    if verified.candidate != *input.candidate {
        return Err(OwnerEquityArtifactError::Conflict);
    }
    Ok(verified)
}

pub fn read_owner_equity_artifact(
    root: &Path,
    manifest_sha256: &ContentHash,
) -> Result<VerifiedOwnerEquityArtifact, OwnerEquityArtifactError> {
    validate_root(root)?;
    let base = artifact_base(root)?;
    let destination = base.join(manifest_sha256.as_str().trim_start_matches("sha256:"));
    read_exact(&destination, manifest_sha256)
}

fn artifact_base(root: &Path) -> Result<PathBuf, OwnerEquityArtifactError> {
    let base = root.join(ARTIFACT_DIRECTORY);
    if !base.exists() {
        create_private_directory(&base)?;
        File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(|_| OwnerEquityArtifactError::WriteFailed)?;
    }
    validate_directory(&base)?;
    Ok(base)
}

fn read_exact(
    directory: &Path,
    expected_manifest_sha256: &ContentHash,
) -> Result<VerifiedOwnerEquityArtifact, OwnerEquityArtifactError> {
    validate_directory(directory).map_err(|error| match error {
        OwnerEquityArtifactError::UnsafeRoot => OwnerEquityArtifactError::Missing,
        other => other,
    })?;
    let mut names = fs::read_dir(directory)
        .map_err(|_| OwnerEquityArtifactError::Missing)?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|_| OwnerEquityArtifactError::Tampered)
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    let expected_names = [
        std::ffi::OsString::from(CANDIDATE_FILE),
        std::ffi::OsString::from(MANIFEST_FILE),
    ];
    if names != expected_names {
        return Err(OwnerEquityArtifactError::Tampered);
    }
    let candidate_bytes = read_private_file(&directory.join(CANDIDATE_FILE), MAX_CANDIDATE_BYTES)?;
    let manifest_bytes = read_private_file(&directory.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?;
    if ContentHash::from_bytes(&manifest_bytes) != *expected_manifest_sha256 {
        return Err(OwnerEquityArtifactError::Tampered);
    }
    let manifest: ArtifactManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| OwnerEquityArtifactError::Tampered)?;
    if manifest.artifact_version != ARTIFACT_VERSION
        || manifest.candidate_size_bytes != candidate_bytes.len() as u64
        || manifest.candidate_sha256 != ContentHash::from_bytes(&candidate_bytes)
    {
        return Err(OwnerEquityArtifactError::Tampered);
    }
    let candidate: OwnerEquityGenerationCandidate =
        serde_json::from_slice(&candidate_bytes).map_err(|_| OwnerEquityArtifactError::Tampered)?;
    if candidate
        .canonical_bytes()
        .map_err(|_| OwnerEquityArtifactError::Tampered)?
        != candidate_bytes
        || candidate.instrument_id.to_string() != manifest.instrument_id
        || candidate.source_pins.raw_manifest_sha256 != manifest.raw_manifest_sha256
        || candidate.source_pins.entitlement_sha256 != manifest.entitlement_sha256
        || candidate.source_pins.capture_code_commit.as_str() != manifest.capture_code_commit
        || candidate.source_pins.materializer_code_commit.as_str()
            != manifest.materializer_code_commit
        || candidate.source_pins.prior_candidate_sha256 != manifest.prior_candidate_sha256
        || candidate.source_pins.prior_artifact_manifest_sha256
            != manifest.prior_artifact_manifest_sha256
    {
        return Err(OwnerEquityArtifactError::Tampered);
    }
    Ok(VerifiedOwnerEquityArtifact {
        owner_user_id: manifest.owner_user_id,
        membership_id: manifest.membership_id,
        generation: manifest.generation,
        candidate,
        candidate_sha256: manifest.candidate_sha256,
        manifest_sha256: expected_manifest_sha256.clone(),
        replayed: false,
    })
}

fn validate_root(root: &Path) -> Result<(), OwnerEquityArtifactError> {
    if !root.is_absolute()
        || root == Path::new("/")
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || root
            .canonicalize()
            .map_err(|_| OwnerEquityArtifactError::UnsafeRoot)?
            != root
    {
        return Err(OwnerEquityArtifactError::UnsafeRoot);
    }
    validate_directory(root)
}

fn validate_directory(path: &Path) -> Result<(), OwnerEquityArtifactError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| OwnerEquityArtifactError::UnsafeRoot)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OwnerEquityArtifactError::UnsafeRoot);
    }
    validate_permissions(&metadata)
}

fn create_private_directory(path: &Path) -> Result<(), OwnerEquityArtifactError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|_| OwnerEquityArtifactError::WriteFailed)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path).map_err(|_| OwnerEquityArtifactError::WriteFailed)
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), OwnerEquityArtifactError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| OwnerEquityArtifactError::WriteFailed)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| OwnerEquityArtifactError::WriteFailed)
}

fn read_private_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, OwnerEquityArtifactError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| OwnerEquityArtifactError::Missing)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(OwnerEquityArtifactError::Tampered);
    }
    validate_permissions(&metadata)?;
    fs::read(path).map_err(|_| OwnerEquityArtifactError::Missing)
}

#[cfg(unix)]
fn validate_permissions(metadata: &fs::Metadata) -> Result<(), OwnerEquityArtifactError> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
        Err(OwnerEquityArtifactError::UnsafePermissions)
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn validate_permissions(_metadata: &fs::Metadata) -> Result<(), OwnerEquityArtifactError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{BatchId, CodeCommit, InstrumentId, TradingDate};
    use market_data::owner_equity_v2::{
        OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OwnerEquityBar,
        OwnerEquityCaptureKind, OwnerEquitySourcePins, PRICE_SEMANTICS,
    };
    use tempfile::tempdir;

    fn candidate() -> OwnerEquityGenerationCandidate {
        let start = TradingDate::parse("2026-01-01").unwrap();
        let bars = (0..121)
            .map(|offset| OwnerEquityBar {
                session_date: start.checked_add_days(offset).unwrap(),
                open: 100,
                high: 105,
                low: 95,
                close: 101,
                volume: 1_000,
            })
            .collect::<Vec<_>>();
        OwnerEquityGenerationCandidate {
            candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
            contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
            capture_kind: OwnerEquityCaptureKind::Initial,
            instrument_id: InstrumentId::parse("005930.KRX").unwrap(),
            display_name: Some("fixture".to_owned()),
            requested_start: start,
            requested_end: bars.last().unwrap().session_date,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            observed_sessions: 121,
            first_observed_date: start,
            last_observed_date: bars.last().unwrap().session_date,
            bars,
            source_pins: OwnerEquitySourcePins {
                capture_identity_sha256: ContentHash::from_bytes(b"identity"),
                raw_batch_id: BatchId::from_uuid(Uuid::from_u128(1)),
                raw_manifest_sha256: ContentHash::from_bytes(b"raw manifest"),
                batch_json_sha256: ContentHash::from_bytes(b"batch"),
                entitlement_reference: "fixture://entitlement".to_owned(),
                entitlement_sha256: ContentHash::from_bytes(b"entitlement"),
                capture_code_commit: CodeCommit::parse("0123456789abcdef0123456789abcdef01234567")
                    .unwrap(),
                materializer_code_commit: CodeCommit::parse(
                    "0123456789abcdef0123456789abcdef01234567",
                )
                .unwrap(),
                prior_candidate_sha256: None,
                prior_artifact_manifest_sha256: None,
                files: vec![],
            },
            price_semantics: PRICE_SEMANTICS.to_owned(),
            owner_only: true,
            vendor_snapshot: true,
            strict_pit: false,
            warnings: vec![],
            claims_not_made: vec![],
        }
    }

    fn input(candidate: &OwnerEquityGenerationCandidate) -> OwnerEquityArtifactInput<'_> {
        OwnerEquityArtifactInput {
            owner_user_id: Uuid::from_u128(2),
            membership_id: Uuid::from_u128(3),
            generation: 1,
            candidate,
        }
    }

    #[test]
    fn immutable_write_read_and_same_input_replay_are_exact() {
        let root = tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750)).unwrap();
        }
        let candidate = candidate();
        let first = write_owner_equity_artifact(root.path(), input(&candidate)).unwrap();
        assert!(!first.replayed);
        let second = write_owner_equity_artifact(root.path(), input(&candidate)).unwrap();
        assert!(second.replayed);
        assert_eq!(first.manifest_sha256, second.manifest_sha256);
        let read = read_owner_equity_artifact(root.path(), &first.manifest_sha256).unwrap();
        assert_eq!(read.candidate, candidate);
        assert_eq!(read.owner_user_id, Uuid::from_u128(2));
        assert_eq!(read.membership_id, Uuid::from_u128(3));
        assert_eq!(read.generation, 1);
    }

    #[test]
    fn tamper_and_extra_files_fail_closed() {
        let root = tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750)).unwrap();
        }
        let candidate = candidate();
        let written = write_owner_equity_artifact(root.path(), input(&candidate)).unwrap();
        let directory = root.path().join(ARTIFACT_DIRECTORY).join(
            written
                .manifest_sha256
                .as_str()
                .trim_start_matches("sha256:"),
        );
        fs::write(directory.join(CANDIDATE_FILE), b"{}").unwrap();
        assert_eq!(
            read_owner_equity_artifact(root.path(), &written.manifest_sha256),
            Err(OwnerEquityArtifactError::Tampered)
        );
        assert_eq!(
            write_owner_equity_artifact(root.path(), input(&candidate)),
            Err(OwnerEquityArtifactError::Conflict)
        );

        let other = tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(other.path(), fs::Permissions::from_mode(0o750)).unwrap();
        }
        let written = write_owner_equity_artifact(other.path(), input(&candidate)).unwrap();
        let directory = other.path().join(ARTIFACT_DIRECTORY).join(
            written
                .manifest_sha256
                .as_str()
                .trim_start_matches("sha256:"),
        );
        fs::write(directory.join("extra"), b"x").unwrap();
        assert_eq!(
            read_owner_equity_artifact(other.path(), &written.manifest_sha256),
            Err(OwnerEquityArtifactError::Tampered)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_group_writable_roots_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let parent = tempdir().unwrap();
        let real = parent.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = parent.path().join("link");
        symlink(&real, &link).unwrap();
        assert_eq!(
            write_owner_equity_artifact(&link, input(&candidate())),
            Err(OwnerEquityArtifactError::UnsafeRoot)
        );

        let writable = tempdir().unwrap();
        fs::set_permissions(writable.path(), fs::Permissions::from_mode(0o770)).unwrap();
        assert_eq!(
            write_owner_equity_artifact(writable.path(), input(&candidate())),
            Err(OwnerEquityArtifactError::UnsafePermissions)
        );
    }
}
