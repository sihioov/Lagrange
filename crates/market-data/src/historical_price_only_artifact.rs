//! Private, filesystem-backed historical price-only artifact boundary.
//!
//! This module intentionally accepts only the opaque in-memory candidate and
//! never serializes Raw request or provider payload metadata.

use std::{
    collections::BTreeSet,
    io::{Read, Write},
};
use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::contract::ResponseKind;
use crate::historical_price_only::HistoricalPriceOnlyCandidate;
use domain::{BatchId, ContentHash, FixedPoint, InstrumentId, TradingDate, UtcTimestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::curate::Capability;
use crate::historical_price_only::{
    HISTORICAL_PRICE_ONLY_FACTOR_SCALE, HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION,
    HISTORICAL_PRICE_ONLY_PRICE_SCALE, HistoricalPriceOnlyAudience, HistoricalPriceOnlyBar,
};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;

#[allow(dead_code)]
const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_BARS_BYTES: usize = 64 * 1024 * 1024;
const MAX_BAR_LINE_BYTES: usize = 16 * 1024;

/// A verified, owner-only historical price-only artifact.
///
/// Construction is deliberately private to this module; callers receive it
/// only after the artifact writer or reader has completed validation.
#[derive(Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct VerifiedHistoricalPriceOnlyArtifact {
    path: PathBuf,
    candidate_content_sha256: ContentHash,
    approval_summary: HistoricalPriceOnlyArtifactApprovalSummary,
    approved_bars: Vec<HistoricalPriceOnlyBar>,
}

/// The non-sensitive, immutable facts an independent approval checker may
/// compare against its own registry after the artifact reader has validated
/// the full on-disk envelope.
///
/// This deliberately has no filesystem path, Raw request metadata, provider
/// payload, batch identifier, or bar payload accessor.  It is not a dataset
/// pin and it conveys neither registration nor publication authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoricalPriceOnlyArtifactApprovalSummary {
    pub(crate) artifact_manifest_sha256: ContentHash,
    pub(crate) stage5_manifest_sha256: ContentHash,
    pub(crate) action_manifest_sha256: ContentHash,
    pub(crate) cash_dividend_treatment_id: String,
    pub(crate) ignored_cash_dividend_row_count: usize,
    pub(crate) ignored_cash_dividend_rows_sha256: ContentHash,
    pub(crate) ignored_cash_dividend_source_file_sha256: ContentHash,
    pub(crate) ignored_cash_dividend_acquired_at: UtcTimestamp,
    pub(crate) schema_id: String,
    pub(crate) schema_version: u32,
    pub(crate) audience: String,
    pub(crate) vendor_snapshot: bool,
    pub(crate) strict_pit: bool,
    pub(crate) capability: String,
    pub(crate) materialization_status: String,
    pub(crate) registration_status: String,
    pub(crate) publication_status: String,
    pub(crate) range_start: TradingDate,
    pub(crate) range_end: TradingDate,
    pub(crate) instruments: Vec<String>,
    pub(crate) instrument_count: usize,
    pub(crate) session_count: usize,
    pub(crate) bar_count: usize,
}

impl HistoricalPriceOnlyArtifactApprovalSummary {
    pub fn artifact_manifest_sha256(&self) -> &ContentHash {
        &self.artifact_manifest_sha256
    }

    pub fn stage5_manifest_sha256(&self) -> &ContentHash {
        &self.stage5_manifest_sha256
    }

    pub fn action_manifest_sha256(&self) -> &ContentHash {
        &self.action_manifest_sha256
    }

    pub fn cash_dividend_treatment_id(&self) -> &str {
        &self.cash_dividend_treatment_id
    }

    pub const fn ignored_cash_dividend_row_count(&self) -> usize {
        self.ignored_cash_dividend_row_count
    }

    pub fn ignored_cash_dividend_rows_sha256(&self) -> &ContentHash {
        &self.ignored_cash_dividend_rows_sha256
    }

    pub fn ignored_cash_dividend_source_file_sha256(&self) -> &ContentHash {
        &self.ignored_cash_dividend_source_file_sha256
    }

    pub const fn ignored_cash_dividend_acquired_at(&self) -> UtcTimestamp {
        self.ignored_cash_dividend_acquired_at
    }

    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn audience(&self) -> &str {
        &self.audience
    }

    pub const fn vendor_snapshot(&self) -> bool {
        self.vendor_snapshot
    }

    pub const fn strict_pit(&self) -> bool {
        self.strict_pit
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn materialization_status(&self) -> &str {
        &self.materialization_status
    }

    pub fn registration_status(&self) -> &str {
        &self.registration_status
    }

    pub fn publication_status(&self) -> &str {
        &self.publication_status
    }

    pub const fn range_start(&self) -> TradingDate {
        self.range_start
    }

    pub const fn range_end(&self) -> TradingDate {
        self.range_end
    }

    pub fn instruments(&self) -> &[String] {
        &self.instruments
    }

    pub const fn instrument_count(&self) -> usize {
        self.instrument_count
    }

    pub const fn session_count(&self) -> usize {
        self.session_count
    }

    pub const fn bar_count(&self) -> usize {
        self.bar_count
    }
}

impl fmt::Debug for VerifiedHistoricalPriceOnlyArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedHistoricalPriceOnlyArtifact")
            .field("candidate_content_sha256", &self.candidate_content_sha256)
            .finish()
    }
}

#[allow(dead_code)]
impl VerifiedHistoricalPriceOnlyArtifact {
    /// Opaque lineage pin from the original candidate; it is not recomputed by
    /// an artifact reader because the candidate preimage includes excluded Raw
    /// request metadata.
    fn path(&self) -> &Path {
        &self.path
    }

    pub fn candidate_content_sha256(&self) -> &ContentHash {
        &self.candidate_content_sha256
    }

    /// Returns only the validated, non-sensitive facts needed by a separate
    /// approval registry checker.  This has no registration or publication
    /// side effect.
    pub fn approval_summary(&self) -> &HistoricalPriceOnlyArtifactApprovalSummary {
        &self.approval_summary
    }

    pub(crate) fn approved_bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.approved_bars
    }
}

/// Converts an already-validated candidate into its private artifact.
///
/// Filesystem publication and complete semantic validation are added below.
#[allow(dead_code)]
pub fn write_historical_price_only_artifact(
    operator_root: &Path,
    candidate: &HistoricalPriceOnlyCandidate,
) -> Result<VerifiedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactError> {
    #[cfg(not(unix))]
    {
        let _ = (operator_root, candidate);
        return Err(HistoricalPriceOnlyArtifactError::UnsupportedPlatform);
    }
    #[cfg(unix)]
    {
        let artifact = project_historical_price_only_artifact(candidate)?;
        write_historical_price_only_artifact_with_ops(
            operator_root,
            candidate,
            &artifact,
            &SystemArtifactWriteOps,
        )
    }
}

/// Reopens a previously materialized artifact.  It does not recompute opaque
/// candidate lineage from Raw preimages.
#[allow(dead_code)]
pub fn read_historical_price_only_artifact(
    operator_root: &Path,
    candidate_content_sha256: &ContentHash,
) -> Result<VerifiedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactError> {
    #[cfg(not(unix))]
    {
        let _ = (operator_root, candidate_content_sha256);
        return Err(HistoricalPriceOnlyArtifactError::UnsupportedPlatform);
    }
    #[cfg(unix)]
    read_historical_price_only_artifact_with_ops(
        operator_root,
        candidate_content_sha256,
        &SystemArtifactReadOps,
    )
}

#[cfg(unix)]
trait ArtifactReadOps {
    fn after_open(&self, _stage: ArtifactReadStage) {}
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactReadStage {
    Root,
    Control,
    Version,
    Candidate,
    Manifest,
    Bars,
}

#[cfg(unix)]
struct TrustedRoot {
    directories: Vec<std::os::fd::OwnedFd>,
    names: Vec<Vec<u8>>,
    snapshots: Vec<StatSnapshot>,
}

#[cfg(unix)]
impl TrustedRoot {
    fn fd(&self) -> &std::os::fd::OwnedFd {
        self.directories.last().expect("root descriptor is held")
    }

    fn parent_fd(&self) -> &std::os::fd::OwnedFd {
        &self.directories[self.directories.len() - 2]
    }

    fn name(&self) -> &[u8] {
        self.names.last().expect("root name is held")
    }

    fn snapshot(&self) -> &StatSnapshot {
        self.snapshots.last().expect("root snapshot is held")
    }

    fn revalidate(&self) -> Result<(), HistoricalPriceOnlyArtifactError> {
        for index in 0..self.names.len() {
            revalidate_named_identity(
                &self.directories[index],
                &self.names[index],
                &self.snapshots[index],
            )?;
        }
        revalidate_named_identity(self.parent_fd(), self.name(), self.snapshot())
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct SystemArtifactReadOps;

#[cfg(unix)]
impl ArtifactReadOps for SystemArtifactReadOps {}

#[cfg(unix)]
fn read_historical_price_only_artifact_with_ops(
    operator_root: &Path,
    candidate_content_sha256: &ContentHash,
    ops: &dyn ArtifactReadOps,
) -> Result<VerifiedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactError> {
    read_historical_price_only_artifact_with_expected(
        operator_root,
        candidate_content_sha256,
        ops,
        None,
    )
}

#[cfg(unix)]
fn read_historical_price_only_artifact_with_expected(
    operator_root: &Path,
    candidate_content_sha256: &ContentHash,
    ops: &dyn ArtifactReadOps,
    expected: Option<&HistoricalPriceOnlyArtifactBytes>,
) -> Result<VerifiedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::fstat;
    use rustix::process::geteuid;

    let root = open_trusted_root(operator_root)?;
    let root_fd = root.fd();
    let owner = geteuid().as_raw();
    let root_stat = fstat(root_fd).map_err(io_error)?;
    validate_directory_stat(&root_stat, owner, None)?;
    if stat_snapshot(&root_stat) != *root.snapshot() {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    root.revalidate()?;
    ops.after_open(ArtifactReadStage::Root);

    let (control_fd, control_stat) = open_directory_at(
        root_fd,
        b"kis-historical-price-only-beta",
        owner,
        Some(0o700),
    )?;
    ops.after_open(ArtifactReadStage::Control);
    let (version_fd, version_stat) = open_directory_at(&control_fd, b"v2", owner, Some(0o700))?;
    ops.after_open(ArtifactReadStage::Version);

    let candidate_name = candidate_directory_name(candidate_content_sha256)?;
    let (candidate_fd, candidate_stat) =
        open_directory_at(&version_fd, candidate_name.as_bytes(), owner, Some(0o700))?;
    ops.after_open(ArtifactReadStage::Candidate);

    let entries = enumerate_candidate_entries(&candidate_fd)?;
    if entries != [b"bars.ndjson".as_slice(), b"manifest.json".as_slice()] {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }

    let (manifest, manifest_snapshot) =
        open_leaf(&candidate_fd, b"manifest.json", owner, MAX_MANIFEST_BYTES)?;
    ops.after_open(ArtifactReadStage::Manifest);
    let manifest_bytes = read_bounded(&manifest, manifest_snapshot.size, MAX_MANIFEST_BYTES)?;
    if expected.is_some_and(|bytes| bytes.manifest_json != manifest_bytes) {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    revalidate_leaf(
        &candidate_fd,
        b"manifest.json",
        &manifest,
        &manifest_snapshot,
    )?;
    let validated_manifest = validate_manifest_bytes(candidate_content_sha256, &manifest_bytes)?;
    let unsigned = &validated_manifest.unsigned;

    let (bars, bars_snapshot) = open_leaf(&candidate_fd, b"bars.ndjson", owner, MAX_BARS_BYTES)?;
    ops.after_open(ArtifactReadStage::Bars);
    if unsigned.bars.size_bytes
        != u64::try_from(bars_snapshot.size)
            .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let approved_bars = stream_validate_bars(
        &bars,
        bars_snapshot.size,
        &unsigned.sessions,
        &unsigned.instruments,
        &unsigned.bars.sha256,
        expected.map(|bytes| bytes.bars_ndjson.as_slice()),
    )?;
    revalidate_leaf(&candidate_fd, b"bars.ndjson", &bars, &bars_snapshot)?;

    // Recheck the directory's exact entry set and every held pathname before
    // returning the lexical path.  The descriptors remain the source of all
    // reads; these checks only prove that the names still identify them.
    rewind_directory(&candidate_fd)?;
    if enumerate_candidate_entries(&candidate_fd)?
        != [b"bars.ndjson".as_slice(), b"manifest.json".as_slice()]
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    revalidate_leaf(
        &candidate_fd,
        b"manifest.json",
        &manifest,
        &manifest_snapshot,
    )?;
    revalidate_leaf(&candidate_fd, b"bars.ndjson", &bars, &bars_snapshot)?;
    root.revalidate()?;
    revalidate_named_identity(
        root_fd,
        b"kis-historical-price-only-beta",
        &stat_snapshot(&control_stat),
    )?;
    revalidate_named_identity(&control_fd, b"v2", &stat_snapshot(&version_stat))?;
    revalidate_named_identity(
        &version_fd,
        candidate_name.as_bytes(),
        &stat_snapshot(&candidate_stat),
    )?;

    let path = operator_root
        .join("kis-historical-price-only-beta")
        .join("v2")
        .join(candidate_name);
    Ok(VerifiedHistoricalPriceOnlyArtifact {
        path,
        candidate_content_sha256: candidate_content_sha256.clone(),
        approval_summary: validated_manifest.approval_summary,
        approved_bars,
    })
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactWriteStage {
    Root,
    Control,
    Version,
    Staging,
    Manifest,
    Bars,
    BeforeRename,
    AfterRename,
}

#[cfg(unix)]
trait ArtifactWriteOps {
    fn checkpoint(
        &self,
        _stage: ArtifactWriteStage,
    ) -> Result<(), HistoricalPriceOnlyArtifactError> {
        Ok(())
    }

    fn sync_published_version(
        &self,
        version: &std::os::fd::OwnedFd,
    ) -> Result<(), HistoricalPriceOnlyArtifactError> {
        rustix::fs::fsync(version).map_err(io_error)
    }

    fn publish(
        &self,
        version: &std::os::fd::OwnedFd,
        staging: &[u8],
        destination: &[u8],
    ) -> Result<PublishOutcome, HistoricalPriceOnlyArtifactError> {
        publish_noreplace(version, staging, destination)
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct SystemArtifactWriteOps;

#[cfg(unix)]
impl ArtifactWriteOps for SystemArtifactWriteOps {}

#[cfg(unix)]
struct StagingDirectory {
    fd: std::os::fd::OwnedFd,
    name: Vec<u8>,
    snapshot: StatSnapshot,
}

#[cfg(unix)]
fn write_historical_price_only_artifact_with_ops(
    operator_root: &Path,
    candidate: &HistoricalPriceOnlyCandidate,
    expected: &HistoricalPriceOnlyArtifactBytes,
    ops: &dyn ArtifactWriteOps,
) -> Result<VerifiedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{fstat, fsync};
    use rustix::process::geteuid;

    let candidate_name = candidate_directory_name(candidate.content_hash())?;
    let root = open_trusted_root(operator_root)?;
    let owner = geteuid().as_raw();
    ops.checkpoint(ArtifactWriteStage::Root)?;

    let (control_fd, control_stat) =
        open_or_create_directory_at(root.fd(), b"kis-historical-price-only-beta", owner)?;
    ops.checkpoint(ArtifactWriteStage::Control)?;
    let (version_fd, version_stat) = open_or_create_directory_at(&control_fd, b"v2", owner)?;
    ops.checkpoint(ArtifactWriteStage::Version)?;

    let staging = create_staging_directory(&version_fd, owner)?;
    if let Err(error) = ops.checkpoint(ArtifactWriteStage::Staging) {
        return fail_before_publish(&version_fd, &staging, None, error);
    }

    let written = match populate_staging(&staging, owner, expected, ops) {
        Ok(written) => written,
        Err(error) => {
            return fail_before_publish(&version_fd, &staging, Some(&error.written), *error.error);
        }
    };

    if let Err(error) = ops.checkpoint(ArtifactWriteStage::BeforeRename) {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    if let Err(error) = root.revalidate() {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    if let Err(error) = revalidate_named_identity(
        root.fd(),
        b"kis-historical-price-only-beta",
        &stat_snapshot(&control_stat),
    ) {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    if let Err(error) = revalidate_named_identity(&control_fd, b"v2", &stat_snapshot(&version_stat))
    {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    if let Err(error) = revalidate_staging(&version_fd, &staging, &written) {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    match fstat(&staging.fd)
        .map_err(io_error)
        .map(|stat| stat_snapshot(&stat))
    {
        Ok(snapshot) if same_stable_identity(&snapshot, &staging.snapshot) => {}
        Ok(_) => {
            return fail_before_publish(
                &version_fd,
                &staging,
                Some(&written),
                HistoricalPriceOnlyArtifactError::UnsafePath,
            );
        }
        Err(error) => {
            return fail_before_publish(&version_fd, &staging, Some(&written), error);
        }
    }
    if let Err(error) = fsync(&staging.fd).map_err(io_error) {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }
    if let Err(error) = revalidate_staging(&version_fd, &staging, &written) {
        return fail_before_publish(&version_fd, &staging, Some(&written), error);
    }

    let publication = match ops.publish(&version_fd, &staging.name, candidate_name.as_bytes()) {
        Ok(publication) => publication,
        Err(error) => {
            return fail_before_publish(&version_fd, &staging, Some(&written), error);
        }
    };
    match publication {
        PublishOutcome::DestinationExists => {
            cleanup_and_sync(&version_fd, &staging, Some(&written))?;
            match read_historical_price_only_artifact_with_expected(
                operator_root,
                candidate.content_hash(),
                &SystemArtifactReadOps,
                Some(expected),
            ) {
                Ok(verified) => Ok(verified),
                Err(_) => Err(HistoricalPriceOnlyArtifactError::Conflict {
                    candidate_content_sha256: candidate.content_hash().clone(),
                }),
            }
        }
        PublishOutcome::Published => {
            if ops.checkpoint(ArtifactWriteStage::AfterRename).is_err()
                || ops.sync_published_version(&version_fd).is_err()
            {
                return Err(HistoricalPriceOnlyArtifactError::IndeterminateCommit);
            }
            match read_historical_price_only_artifact_with_expected(
                operator_root,
                candidate.content_hash(),
                &SystemArtifactReadOps,
                Some(expected),
            ) {
                Ok(verified) => Ok(verified),
                Err(_) => Err(HistoricalPriceOnlyArtifactError::IndeterminateCommit),
            }
        }
    }
}

#[cfg(unix)]
fn open_or_create_directory_at(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
) -> Result<(std::os::fd::OwnedFd, rustix::fs::Stat), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{Mode, fsync, mkdirat};

    match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
        Ok(()) => fsync(parent).map_err(io_error)?,
        Err(error) if error == rustix::io::Errno::EXIST => {}
        Err(error) => return Err(io_error(error)),
    }
    open_directory_at(parent, name, owner, Some(0o700))
}

#[cfg(unix)]
fn create_staging_directory(
    version: &impl std::os::fd::AsFd,
    owner: rustix::process::RawUid,
) -> Result<StagingDirectory, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{Mode, fsync, mkdirat};

    static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);
    for _ in 0..128 {
        let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".stage-pid-{}-{}", std::process::id(), sequence).into_bytes();
        match mkdirat(version, name.as_slice(), Mode::from_raw_mode(0o700)) {
            Ok(()) => {
                let (fd, stat) = open_directory_at(version, &name, owner, Some(0o700))?;
                let staging = StagingDirectory {
                    fd,
                    name,
                    snapshot: stat_snapshot(&stat),
                };
                if let Err(error) = fsync(version).map_err(io_error) {
                    return if cleanup_staging_tree(version, &staging, None).is_err() {
                        Err(HistoricalPriceOnlyArtifactError::CleanupFailed)
                    } else {
                        Err(error)
                    };
                }
                return Ok(staging);
            }
            Err(error) if error == rustix::io::Errno::EXIST => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(HistoricalPriceOnlyArtifactError::StagingNameExhausted)
}

#[cfg(unix)]
fn populate_staging(
    staging: &StagingDirectory,
    owner: rustix::process::RawUid,
    expected: &HistoricalPriceOnlyArtifactBytes,
    ops: &dyn ArtifactWriteOps,
) -> Result<WrittenStaging, StagingPopulationError> {
    let manifest = match write_staging_file(
        &staging.fd,
        b"manifest.json",
        &expected.manifest_json,
        owner,
        MAX_MANIFEST_BYTES,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(StagingPopulationError {
                error: error.error,
                written: Box::new(WrittenStaging {
                    manifest: error.snapshot,
                    bars: None,
                }),
            });
        }
    };
    let mut written = WrittenStaging {
        manifest: Some(manifest),
        bars: None,
    };
    if let Err(error) = ops.checkpoint(ArtifactWriteStage::Manifest) {
        return Err(StagingPopulationError {
            error: Box::new(error),
            written: Box::new(written),
        });
    }
    let bars = match write_staging_file(
        &staging.fd,
        b"bars.ndjson",
        &expected.bars_ndjson,
        owner,
        MAX_BARS_BYTES,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            written.bars = error.snapshot;
            return Err(StagingPopulationError {
                error: error.error,
                written: Box::new(written),
            });
        }
    };
    written.bars = Some(bars);
    if let Err(error) = ops.checkpoint(ArtifactWriteStage::Bars) {
        return Err(StagingPopulationError {
            error: Box::new(error),
            written: Box::new(written),
        });
    }
    let entries = match enumerate_candidate_entries(&staging.fd) {
        Ok(entries) => entries,
        Err(error) => {
            return Err(StagingPopulationError {
                error: Box::new(error),
                written: Box::new(written),
            });
        }
    };
    if entries != [b"bars.ndjson".as_slice(), b"manifest.json".as_slice()] {
        return Err(StagingPopulationError {
            error: Box::new(HistoricalPriceOnlyArtifactError::InvalidArtifact),
            written: Box::new(written),
        });
    }
    Ok(written)
}

#[cfg(unix)]
struct WrittenStaging {
    manifest: Option<StatSnapshot>,
    bars: Option<StatSnapshot>,
}

#[cfg(unix)]
struct StagingPopulationError {
    error: Box<HistoricalPriceOnlyArtifactError>,
    written: Box<WrittenStaging>,
}

#[cfg(unix)]
struct StagingWriteError {
    error: Box<HistoricalPriceOnlyArtifactError>,
    snapshot: Option<StatSnapshot>,
}

#[cfg(unix)]
fn write_staging_file(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    bytes: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<StatSnapshot, StagingWriteError> {
    use rustix::fs::{Mode, OFlags, fstat, fsync, openat};

    if bytes.len() > max_bytes {
        return Err(StagingWriteError {
            error: Box::new(HistoricalPriceOnlyArtifactError::InvalidArtifact),
            snapshot: None,
        });
    }
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| StagingWriteError {
        error: Box::new(io_error(error)),
        snapshot: None,
    })?;
    let mut file = std::fs::File::from(fd);
    let initial = fstat(&file).map_err(|error| StagingWriteError {
        error: Box::new(io_error(error)),
        snapshot: None,
    })?;
    if let Err(error) = validate_leaf_stat(&initial, owner, max_bytes) {
        return Err(StagingWriteError {
            error: Box::new(error),
            snapshot: None,
        });
    }
    let initial = stat_snapshot(&initial);
    if let Err(error) = revalidate_named(parent, name, &initial) {
        return Err(StagingWriteError {
            error: Box::new(error),
            snapshot: None,
        });
    }
    if let Err(error) = file.write_all(bytes) {
        return Err(StagingWriteError {
            error: Box::new(HistoricalPriceOnlyArtifactError::Io(error)),
            snapshot: safe_current_leaf_snapshot(&file, parent, name, owner, max_bytes),
        });
    }
    if let Err(error) = fsync(&file).map_err(io_error) {
        return Err(StagingWriteError {
            error: Box::new(error),
            snapshot: safe_current_leaf_snapshot(&file, parent, name, owner, max_bytes),
        });
    }
    let final_stat = match fstat(&file).map_err(io_error) {
        Ok(stat) => stat,
        Err(error) => {
            return Err(StagingWriteError {
                error: Box::new(error),
                snapshot: safe_current_leaf_snapshot(&file, parent, name, owner, max_bytes),
            });
        }
    };
    if let Err(error) = validate_leaf_stat(&final_stat, owner, max_bytes) {
        return Err(StagingWriteError {
            error: Box::new(error),
            snapshot: None,
        });
    }
    let final_snapshot = stat_snapshot(&final_stat);
    if final_snapshot.size != bytes.len() as i128 {
        return Err(StagingWriteError {
            error: Box::new(HistoricalPriceOnlyArtifactError::InvalidArtifact),
            snapshot: Some(final_snapshot),
        });
    }
    if let Err(error) = revalidate_named(parent, name, &final_snapshot) {
        return Err(StagingWriteError {
            error: Box::new(error),
            snapshot: None,
        });
    }
    Ok(final_snapshot)
}

#[cfg(unix)]
fn safe_current_leaf_snapshot(
    file: &std::fs::File,
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Option<StatSnapshot> {
    use rustix::fs::fstat;

    let stat = fstat(file).ok()?;
    validate_leaf_stat(&stat, owner, max_bytes).ok()?;
    let snapshot = stat_snapshot(&stat);
    revalidate_named(parent, name, &snapshot).ok()?;
    Some(snapshot)
}

#[cfg(unix)]
fn revalidate_staging(
    version: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: &WrittenStaging,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::fstat;

    let (Some(manifest), Some(bars)) = (&written.manifest, &written.bars) else {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    };

    revalidate_named_identity(version, &staging.name, &staging.snapshot)?;
    if !same_stable_identity(
        &stat_snapshot(&fstat(&staging.fd).map_err(io_error)?),
        &staging.snapshot,
    ) {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    // The directory stream has a shared offset across dup(2), so every
    // publication-boundary enumeration must rewind the held descriptor first.
    // Keep this exact-entry check adjacent to the leaf snapshot checks: no
    // staging contents may be published unless the complete expected tree is
    // still present.
    if enumerate_candidate_entries(&staging.fd)?
        != [b"bars.ndjson".as_slice(), b"manifest.json".as_slice()]
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    revalidate_named(&staging.fd, b"manifest.json", manifest)?;
    revalidate_named(&staging.fd, b"bars.ndjson", bars)
}

#[cfg(unix)]
fn fail_before_publish<T>(
    version: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
    error: HistoricalPriceOnlyArtifactError,
) -> Result<T, HistoricalPriceOnlyArtifactError> {
    if cleanup_and_sync(version, staging, written).is_err() {
        Err(HistoricalPriceOnlyArtifactError::CleanupFailed)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn cleanup_and_sync(
    version: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    cleanup_staging_tree(version, staging, written)?;
    rustix::fs::fsync(version).map_err(io_error)
}

#[cfg(unix)]
fn cleanup_staging_tree(
    version: &impl std::os::fd::AsFd,
    staging: &StagingDirectory,
    written: Option<&WrittenStaging>,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{AtFlags, statat, unlinkat};

    revalidate_named_identity(version, &staging.name, &staging.snapshot)?;
    let written = match written {
        Some(written) => written,
        None => {
            // Without a completed write_staging_file snapshot there is no
            // safe basis for unlinking any leaf. An empty staging directory
            // (the only state possible before population starts) may still be
            // removed.
            if !enumerate_directory_entries(&staging.fd)?.is_empty() {
                return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
            }
            revalidate_named_identity(version, &staging.name, &staging.snapshot)?;
            return unlinkat(version, &staging.name, AtFlags::REMOVEDIR)
                .map_err(io_error)
                .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed);
        }
    };

    let mut expected = Vec::with_capacity(2);
    if let Some(snapshot) = written.bars.as_ref() {
        expected.push((b"bars.ndjson".as_slice(), snapshot));
    }
    if let Some(snapshot) = written.manifest.as_ref() {
        expected.push((b"manifest.json".as_slice(), snapshot));
    }
    expected.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let expected_names = expected
        .iter()
        .map(|(name, _)| name.to_vec())
        .collect::<Vec<_>>();

    let entries = enumerate_directory_entries(&staging.fd)
        .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?;
    if entries != expected_names {
        return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
    }

    // Validate both names and both complete snapshots before beginning any
    // unlink.  In particular, a replacement regular file, symlink, missing
    // leaf, or extra entry must never cause cleanup to remove an object that
    // was not produced by this writer.
    for (name, expected) in &expected {
        let current = statat(&staging.fd, *name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?;
        if stat_snapshot(&current) != **expected {
            return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
        }
    }

    // Rewind and enumerate again after the pre-unlink validation.  This
    // closes the shared-directory-offset gap and makes a newly inserted or
    // removed entry fail closed before any expected leaf is unlinked.
    let entries = enumerate_directory_entries(&staging.fd)
        .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?;
    if entries != expected_names {
        return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
    }

    for (name, expected) in &expected {
        let current = statat(&staging.fd, *name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?;
        if stat_snapshot(&current) != **expected {
            return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
        }
    }
    for (name, _) in &expected {
        unlinkat(&staging.fd, *name, AtFlags::empty())
            .map_err(io_error)
            .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?;
    }

    if !enumerate_directory_entries(&staging.fd)
        .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)?
        .is_empty()
    {
        return Err(HistoricalPriceOnlyArtifactError::CleanupFailed);
    }
    revalidate_named_identity(version, &staging.name, &staging.snapshot)?;
    unlinkat(version, &staging.name, AtFlags::REMOVEDIR)
        .map_err(io_error)
        .map_err(|_| HistoricalPriceOnlyArtifactError::CleanupFailed)
}

#[cfg(unix)]
enum PublishOutcome {
    Published,
    DestinationExists,
}

#[cfg(all(
    unix,
    any(
        target_os = "android",
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
        target_os = "redox"
    )
))]
fn publish_noreplace(
    version: &impl std::os::fd::AsFd,
    staging: &[u8],
    destination: &[u8],
) -> Result<PublishOutcome, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{RenameFlags, renameat_with};
    match renameat_with(
        version,
        staging,
        version,
        destination,
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => Ok(PublishOutcome::Published),
        Err(error) if error == rustix::io::Errno::EXIST => Ok(PublishOutcome::DestinationExists),
        Err(error)
            if error == rustix::io::Errno::NOSYS
                || error == rustix::io::Errno::INVAL
                || error == rustix::io::Errno::NOTSUP
                || error == rustix::io::Errno::OPNOTSUPP =>
        {
            Err(HistoricalPriceOnlyArtifactError::UnsupportedAtomicNoReplace)
        }
        Err(error) => Err(io_error(error)),
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "android",
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "visionos",
        target_os = "watchos",
        target_os = "redox"
    ))
))]
fn publish_noreplace(
    _version: &impl std::os::fd::AsFd,
    _staging: &[u8],
    _destination: &[u8],
) -> Result<PublishOutcome, HistoricalPriceOnlyArtifactError> {
    Err(HistoricalPriceOnlyArtifactError::UnsupportedAtomicNoReplace)
}

#[cfg(unix)]
fn lexical_root_components(path: &Path) -> Result<Vec<Vec<u8>>, HistoricalPriceOnlyArtifactError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes.len() < 2 || bytes[0] != b'/' || bytes[1] == b'/' || bytes.ends_with(b"/") {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    let mut components = Vec::new();
    for component in bytes[1..].split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." || component.contains(&0)
        {
            return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
        }
        components.push(component.to_vec());
    }
    if components.is_empty() {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(components)
}

#[cfg(unix)]
fn open_trusted_root(path: &Path) -> Result<TrustedRoot, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{Mode, OFlags, fstat, open, openat};

    let components = lexical_root_components(path)?;
    let open_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let slash = open(Path::new("/"), open_flags, Mode::empty()).map_err(io_error)?;
    let mut directories = vec![slash];
    let mut names = Vec::<Vec<u8>>::with_capacity(components.len());
    let mut snapshots = Vec::with_capacity(components.len());
    for component in components {
        let parent = directories.last().expect("root descriptor is held");
        let fd =
            openat(parent, component.as_slice(), open_flags, Mode::empty()).map_err(io_error)?;
        let stat = fstat(&fd).map_err(io_error)?;
        if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Directory {
            return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
        }
        directories.push(fd);
        names.push(component);
        snapshots.push(stat_snapshot(&stat));
    }
    let root_fd = directories.last().expect("at least one root component");
    let owner = rustix::process::geteuid().as_raw();
    let root_stat = fstat(root_fd).map_err(io_error)?;
    validate_directory_stat(&root_stat, owner, None)?;
    let root_snapshot = stat_snapshot(&root_stat);
    if snapshots.last() != Some(&root_snapshot) {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    let root = TrustedRoot {
        directories,
        names,
        snapshots,
    };
    root.revalidate()?;
    Ok(root)
}

#[cfg(unix)]
fn candidate_directory_name(
    candidate_content_sha256: &ContentHash,
) -> Result<String, HistoricalPriceOnlyArtifactError> {
    let digest = candidate_content_sha256
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(HistoricalPriceOnlyArtifactError::UnsafePath)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(format!("candidate-sha256={digest}"))
}

#[cfg(unix)]
fn io_error(error: rustix::io::Errno) -> HistoricalPriceOnlyArtifactError {
    if error == rustix::io::Errno::LOOP {
        HistoricalPriceOnlyArtifactError::UnsafePath
    } else {
        HistoricalPriceOnlyArtifactError::Io(error.into())
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatSnapshot {
    dev: u64,
    ino: u64,
    file_type: rustix::fs::FileType,
    uid: u64,
    mode: u64,
    nlink: u64,
    size: i128,
}

#[cfg(unix)]
fn stat_snapshot(stat: &rustix::fs::Stat) -> StatSnapshot {
    use rustix::fs::{FileType, Mode};
    StatSnapshot {
        dev: stat.st_dev,
        ino: stat.st_ino,
        file_type: FileType::from_raw_mode(stat.st_mode),
        uid: stat.st_uid as u64,
        mode: Mode::from_raw_mode(stat.st_mode).bits() as u64,
        nlink: stat.st_nlink,
        size: stat.st_size as i128,
    }
}

#[cfg(unix)]
fn validate_directory_stat(
    stat: &rustix::fs::Stat,
    owner: rustix::process::RawUid,
    exact_mode: Option<u32>,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory || stat.st_uid != owner {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    let mode = Mode::from_raw_mode(stat.st_mode);
    if let Some(expected) = exact_mode {
        if mode.bits() != expected {
            return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
        }
    } else if mode.intersects(Mode::WGRP | Mode::WOTH) {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn open_directory_at(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    exact_mode: Option<u32>,
) -> Result<(std::os::fd::OwnedFd, rustix::fs::Stat), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, openat};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = openat(parent, name, flags, Mode::empty()).map_err(io_error)?;
    let stat = fstat(&fd).map_err(io_error)?;
    validate_directory_stat(&stat, owner, exact_mode)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    revalidate_named_identity(parent, name, &stat_snapshot(&stat))?;
    Ok((fd, stat))
}

#[cfg(unix)]
fn open_leaf(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<(std::fs::File, StatSnapshot), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, openat, statat};
    let observed = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    let observed_snapshot = stat_snapshot(&observed);
    validate_leaf_stat(&observed, owner, max_bytes)?;
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let stat = fstat(&fd).map_err(io_error)?;
    validate_leaf_stat(&stat, owner, max_bytes)?;
    if stat_snapshot(&stat) != observed_snapshot
        || FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
    {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok((std::fs::File::from(fd), observed_snapshot))
}

#[cfg(unix)]
fn validate_leaf_stat(
    stat: &rustix::fs::Stat,
    owner: rustix::process::RawUid,
    max_bytes: usize,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{FileType, Mode};
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != owner
        || Mode::from_raw_mode(stat.st_mode).bits() != 0o600
        || stat.st_nlink != 1
        || stat.st_size < 0
        || (stat.st_size as u128) > max_bytes as u128
    {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_named(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{AtFlags, statat};
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    if stat_snapshot(&stat) != *expected {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_named_identity(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{AtFlags, statat};
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
    let actual = stat_snapshot(&stat);
    if !same_stable_identity(&actual, expected) {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    Ok(())
}

#[cfg(unix)]
fn same_stable_identity(actual: &StatSnapshot, expected: &StatSnapshot) -> bool {
    actual.dev == expected.dev
        && actual.ino == expected.ino
        && actual.file_type == expected.file_type
        && actual.uid == expected.uid
        && actual.mode == expected.mode
}

#[cfg(unix)]
fn revalidate_leaf(
    parent: &impl std::os::fd::AsFd,
    name: &[u8],
    file: &std::fs::File,
    expected: &StatSnapshot,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::fstat;
    let stat = fstat(file).map_err(io_error)?;
    if stat_snapshot(&stat) != *expected {
        return Err(HistoricalPriceOnlyArtifactError::UnsafePath);
    }
    revalidate_named(parent, name, expected)
}

#[cfg(unix)]
fn enumerate_candidate_entries(
    candidate: &std::os::fd::OwnedFd,
) -> Result<Vec<&'static [u8]>, HistoricalPriceOnlyArtifactError> {
    let entries = enumerate_directory_entries(candidate)?;
    let mut known = Vec::with_capacity(entries.len());
    for name in entries {
        let name = match name.as_slice() {
            b"bars.ndjson" => b"bars.ndjson".as_slice(),
            b"manifest.json" => b"manifest.json".as_slice(),
            _ => return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact),
        };
        if known.contains(&name) {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        known.push(name);
    }
    known.sort_unstable();
    Ok(known)
}

#[cfg(unix)]
fn enumerate_directory_entries(
    candidate: &std::os::fd::OwnedFd,
) -> Result<Vec<Vec<u8>>, HistoricalPriceOnlyArtifactError> {
    use rustix::fs::RawDir;
    use rustix::io::dup;
    use std::mem::MaybeUninit;

    // dup(2) shares the open file description and therefore its directory
    // offset. Rewind the held descriptor on every enumeration, including
    // cleanup's pre- and post-validation walks.
    rewind_directory(candidate)?;
    let duplicate = dup(candidate).map_err(io_error)?;
    let mut storage = [MaybeUninit::<u8>::uninit(); 4096];
    let mut directory = RawDir::new(&duplicate, &mut storage);
    let mut entries = Vec::new();
    while let Some(entry) = directory.next() {
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let name = name.to_vec();
        if entries.contains(&name) {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        entries.push(name);
    }
    entries.sort_unstable();
    Ok(entries)
}

#[cfg(unix)]
fn rewind_directory(
    candidate: &std::os::fd::OwnedFd,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    use rustix::fs::{SeekFrom, seek};
    seek(candidate, SeekFrom::Start(0))
        .map(|_| ())
        .map_err(io_error)
}

#[cfg(unix)]
fn read_bounded(
    file: &std::fs::File,
    size: i128,
    max_bytes: usize,
) -> Result<Vec<u8>, HistoricalPriceOnlyArtifactError> {
    if size < 0 || size > max_bytes as i128 {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let size =
        usize::try_from(size).map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    let mut bytes = Vec::with_capacity(size);
    let mut remaining = size;
    let mut chunk = [0u8; 8192];
    while remaining != 0 {
        let amount = remaining.min(chunk.len());
        let read = (&*file)
            .read(&mut chunk[..amount])
            .map_err(HistoricalPriceOnlyArtifactError::Io)?;
        if read == 0 {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        bytes.extend_from_slice(&chunk[..read]);
        remaining -= read;
    }
    Ok(bytes)
}

#[cfg(unix)]
fn stream_validate_bars(
    file: &std::fs::File,
    size: i128,
    sessions: &[SessionDto],
    instruments: &[String],
    expected_hash: &ContentHash,
    expected_bytes: Option<&[u8]>,
) -> Result<Vec<HistoricalPriceOnlyBar>, HistoricalPriceOnlyArtifactError> {
    if size < 0 || size > MAX_BARS_BYTES as i128 {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut validator = BarValidator::new(sessions, instruments, Some(expected_hash));
    let mut comparator = expected_bytes.map(ExpectedBytesComparator::new);
    let mut remaining =
        usize::try_from(size).map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    let mut chunk = [0u8; 8192];
    while remaining != 0 {
        let amount = remaining.min(chunk.len());
        let read = (&*file)
            .read(&mut chunk[..amount])
            .map_err(HistoricalPriceOnlyArtifactError::Io)?;
        if read == 0 {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        if let Some(comparator) = comparator.as_mut() {
            comparator.feed(&chunk[..read])?;
        }
        validator.feed(&chunk[..read])?;
        remaining -= read;
    }
    let bars = validator.finish()?;
    if let Some(comparator) = comparator {
        comparator.finish()?;
    }
    Ok(bars)
}

#[cfg(unix)]
struct ExpectedBytesComparator<'a> {
    expected: &'a [u8],
    offset: usize,
}

#[cfg(unix)]
impl<'a> ExpectedBytesComparator<'a> {
    fn new(expected: &'a [u8]) -> Self {
        Self {
            expected,
            offset: 0,
        }
    }

    fn feed(&mut self, actual: &[u8]) -> Result<(), HistoricalPriceOnlyArtifactError> {
        let end = self
            .offset
            .checked_add(actual.len())
            .ok_or(HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
        if end > self.expected.len() || self.expected[self.offset..end] != *actual {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        self.offset = end;
        Ok(())
    }

    fn finish(self) -> Result<(), HistoricalPriceOnlyArtifactError> {
        if self.offset == self.expected.len() {
            Ok(())
        } else {
            Err(HistoricalPriceOnlyArtifactError::InvalidArtifact)
        }
    }
}

#[allow(dead_code)]
fn validate_candidate(
    candidate: &HistoricalPriceOnlyCandidate,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    if candidate.range_start().to_string() != "2020-01-31"
        || candidate.range_end().to_string() != "2026-08-19"
        || candidate.session_count() != 1608
        || candidate.row_count() != 17688
        || candidate.source_file_count() != 187
        || candidate.action_file_count() != 7
        || candidate.ignored_cash_dividends().treatment_id()
            != crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
        || candidate.ignored_cash_dividends().row_count() == 0
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidCandidate);
    }
    let metadata = candidate.metadata();
    if !metadata.vendor_snapshot
        || metadata.strict_pit
        || metadata.materialized
        || !metadata.in_memory
        || metadata.ready
        || metadata.audience != HistoricalPriceOnlyAudience::OwnerOnly
        || metadata.capability != Capability::PriceReturnOnly
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidCandidate);
    }
    Ok(())
}

/// Private deterministic payload consumed by the later descriptor-safe writer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
struct HistoricalPriceOnlyArtifactBytes {
    bars_ndjson: Vec<u8>,
    manifest_json: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BarDto {
    instrument_id: String,
    session_date: TradingDate,
    raw_open: String,
    raw_high: String,
    raw_low: String,
    raw_close: String,
    raw_volume: u64,
    raw_trading_value: Option<String>,
    adjusted_open: String,
    adjusted_high: String,
    adjusted_low: String,
    adjusted_close: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SafeFileDto {
    file_name: String,
    kind: ResponseKind,
    sha256: ContentHash,
    size_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Stage5Dto {
    batch_id: BatchId,
    manifest_sha256: ContentHash,
    file_count: usize,
    files: Vec<SafeFileDto>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct KsdDto {
    batch_id: BatchId,
    manifest_sha256: ContentHash,
    file_count: usize,
    cash_dividend_treatment_id: String,
    ignored_cash_dividend_row_count: usize,
    ignored_cash_dividend_rows_sha256: ContentHash,
    ignored_cash_dividend_source_file_sha256: ContentHash,
    ignored_cash_dividend_acquired_at: UtcTimestamp,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SessionDto {
    session_date: TradingDate,
    normalized_batch_id: BatchId,
    normalized_entry_sha256: ContentHash,
    normalized_bars_sha256: ContentHash,
    acquired_at: UtcTimestamp,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BonusDto {
    instrument_id: String,
    record_date: TradingDate,
    ex_date: TradingDate,
    split_factor: String,
    acquired_at: UtcTimestamp,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BarsDto {
    relative_path: String,
    schema_id: String,
    schema_version: u32,
    sha256: ContentHash,
    size_bytes: u64,
    row_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct UnsignedManifest {
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    candidate_content_sha256: ContentHash,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    price_scale: u8,
    factor_scale: u8,
    stage5: Stage5Dto,
    ksd: KsdDto,
    sessions: Vec<SessionDto>,
    bonus_evidence: Vec<BonusDto>,
    bars: BarsDto,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_id: String,
    schema_version: u32,
    contract: String,
    materializer_version: String,
    candidate_content_sha256: ContentHash,
    audience: String,
    vendor_snapshot: bool,
    strict_pit: bool,
    capability: String,
    materialization_status: String,
    registration_status: String,
    publication_status: String,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: Vec<String>,
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    price_scale: u8,
    factor_scale: u8,
    stage5: Stage5Dto,
    ksd: KsdDto,
    sessions: Vec<SessionDto>,
    bonus_evidence: Vec<BonusDto>,
    bars: BarsDto,
    manifest_sha256: ContentHash,
}

struct ValidatedManifest {
    unsigned: UnsignedManifest,
    approval_summary: HistoricalPriceOnlyArtifactApprovalSummary,
}

impl Manifest {
    fn from_unsigned(unsigned: UnsignedManifest) -> Result<Self, HistoricalPriceOnlyArtifactError> {
        let manifest_sha256 = ContentHash::from_bytes(
            &serde_json::to_vec(&unsigned)
                .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?,
        );
        Ok(Self {
            schema_id: unsigned.schema_id,
            schema_version: unsigned.schema_version,
            contract: unsigned.contract,
            materializer_version: unsigned.materializer_version,
            candidate_content_sha256: unsigned.candidate_content_sha256,
            audience: unsigned.audience,
            vendor_snapshot: unsigned.vendor_snapshot,
            strict_pit: unsigned.strict_pit,
            capability: unsigned.capability,
            materialization_status: unsigned.materialization_status,
            registration_status: unsigned.registration_status,
            publication_status: unsigned.publication_status,
            range_start: unsigned.range_start,
            range_end: unsigned.range_end,
            instruments: unsigned.instruments,
            instrument_count: unsigned.instrument_count,
            session_count: unsigned.session_count,
            row_count: unsigned.row_count,
            price_scale: unsigned.price_scale,
            factor_scale: unsigned.factor_scale,
            stage5: unsigned.stage5,
            ksd: unsigned.ksd,
            sessions: unsigned.sessions,
            bonus_evidence: unsigned.bonus_evidence,
            bars: unsigned.bars,
            manifest_sha256,
        })
    }
    fn unsigned(&self) -> UnsignedManifest {
        UnsignedManifest {
            schema_id: self.schema_id.clone(),
            schema_version: self.schema_version,
            contract: self.contract.clone(),
            materializer_version: self.materializer_version.clone(),
            candidate_content_sha256: self.candidate_content_sha256.clone(),
            audience: self.audience.clone(),
            vendor_snapshot: self.vendor_snapshot,
            strict_pit: self.strict_pit,
            capability: self.capability.clone(),
            materialization_status: self.materialization_status.clone(),
            registration_status: self.registration_status.clone(),
            publication_status: self.publication_status.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            instruments: self.instruments.clone(),
            instrument_count: self.instrument_count,
            session_count: self.session_count,
            row_count: self.row_count,
            price_scale: self.price_scale,
            factor_scale: self.factor_scale,
            stage5: self.stage5.clone(),
            ksd: self.ksd.clone(),
            sessions: self.sessions.clone(),
            bonus_evidence: self.bonus_evidence.clone(),
            bars: self.bars.clone(),
        }
    }

    fn approval_summary(&self) -> HistoricalPriceOnlyArtifactApprovalSummary {
        HistoricalPriceOnlyArtifactApprovalSummary {
            artifact_manifest_sha256: self.manifest_sha256.clone(),
            stage5_manifest_sha256: self.stage5.manifest_sha256.clone(),
            action_manifest_sha256: self.ksd.manifest_sha256.clone(),
            cash_dividend_treatment_id: self.ksd.cash_dividend_treatment_id.clone(),
            ignored_cash_dividend_row_count: self.ksd.ignored_cash_dividend_row_count,
            ignored_cash_dividend_rows_sha256: self.ksd.ignored_cash_dividend_rows_sha256.clone(),
            ignored_cash_dividend_source_file_sha256: self
                .ksd
                .ignored_cash_dividend_source_file_sha256
                .clone(),
            ignored_cash_dividend_acquired_at: self.ksd.ignored_cash_dividend_acquired_at,
            schema_id: self.schema_id.clone(),
            schema_version: self.schema_version,
            audience: self.audience.clone(),
            vendor_snapshot: self.vendor_snapshot,
            strict_pit: self.strict_pit,
            capability: self.capability.clone(),
            materialization_status: self.materialization_status.clone(),
            registration_status: self.registration_status.clone(),
            publication_status: self.publication_status.clone(),
            range_start: self.range_start,
            range_end: self.range_end,
            instruments: self.instruments.clone(),
            instrument_count: self.instrument_count,
            session_count: self.session_count,
            bar_count: self.row_count,
        }
    }
}

/// Produces canonical projection bytes only; it never touches the filesystem.
#[allow(dead_code)]
fn project_historical_price_only_artifact(
    candidate: &HistoricalPriceOnlyCandidate,
) -> Result<HistoricalPriceOnlyArtifactBytes, HistoricalPriceOnlyArtifactError> {
    validate_candidate(candidate)?;
    let mut rows = candidate
        .bars()
        .iter()
        .map(|bar| BarDto {
            instrument_id: bar.instrument_id.to_string(),
            session_date: bar.session_date,
            raw_open: bar.raw_open.to_string(),
            raw_high: bar.raw_high.to_string(),
            raw_low: bar.raw_low.to_string(),
            raw_close: bar.raw_close.to_string(),
            raw_volume: bar.raw_volume,
            raw_trading_value: bar.raw_trading_value.map(|value| value.to_string()),
            adjusted_open: bar.adjusted_open.to_string(),
            adjusted_high: bar.adjusted_high.to_string(),
            adjusted_low: bar.adjusted_low.to_string(),
            adjusted_close: bar.adjusted_close.to_string(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then(left.session_date.cmp(&right.session_date))
    });
    let mut bars_ndjson = Vec::with_capacity(rows.len() * 256);
    for row in rows {
        serde_json::to_writer(&mut bars_ndjson, &row)
            .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
        bars_ndjson.push(b'\n');
    }
    let bars_hash = ContentHash::from_bytes(&bars_ndjson);
    let mut instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    instruments.sort();
    let unsigned = UnsignedManifest {
        schema_id: "kis-historical-price-only-beta".into(),
        schema_version: 2,
        contract: crate::HISTORICAL_PRICE_ONLY_BETA_CONTRACT.into(),
        materializer_version: HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION.into(),
        candidate_content_sha256: candidate.content_hash().clone(),
        audience: "OWNER_ONLY".into(),
        vendor_snapshot: true,
        strict_pit: false,
        capability: "PRICE_RETURN_ONLY".into(),
        materialization_status: "MATERIALIZED".into(),
        registration_status: "UNREGISTERED".into(),
        publication_status: "NOT_PUBLISHED".into(),
        range_start: candidate.range_start(),
        range_end: candidate.range_end(),
        instruments,
        instrument_count: 11,
        session_count: 1608,
        row_count: 17688,
        price_scale: HISTORICAL_PRICE_ONLY_PRICE_SCALE,
        factor_scale: HISTORICAL_PRICE_ONLY_FACTOR_SCALE,
        stage5: Stage5Dto {
            batch_id: candidate.source_batch_id(),
            manifest_sha256: candidate.source_manifest_hash().clone(),
            file_count: 187,
            files: candidate
                .source_files()
                .iter()
                .map(|file| SafeFileDto {
                    file_name: file.file_name.clone(),
                    kind: file.kind,
                    sha256: file.content_hash.clone(),
                    size_bytes: file.size_bytes,
                })
                .collect(),
        },
        ksd: KsdDto {
            batch_id: candidate.action_batch_id(),
            manifest_sha256: candidate.action_manifest_hash().clone(),
            file_count: 7,
            cash_dividend_treatment_id: candidate
                .ignored_cash_dividends()
                .treatment_id()
                .to_owned(),
            ignored_cash_dividend_row_count: candidate.ignored_cash_dividends().row_count(),
            ignored_cash_dividend_rows_sha256: candidate
                .ignored_cash_dividends()
                .rows_sha256()
                .clone(),
            ignored_cash_dividend_source_file_sha256: candidate
                .ignored_cash_dividends()
                .source_file_sha256()
                .clone(),
            ignored_cash_dividend_acquired_at: candidate.ignored_cash_dividends().acquired_at(),
        },
        sessions: candidate
            .session_provenance()
            .iter()
            .map(|s| SessionDto {
                session_date: s.session_date,
                normalized_batch_id: s.normalized_batch_id,
                normalized_entry_sha256: s.normalized_entry_hash.clone(),
                normalized_bars_sha256: s.normalized_bars_hash.clone(),
                acquired_at: s.acquired_at,
            })
            .collect(),
        bonus_evidence: candidate
            .bonus_evidence()
            .iter()
            .map(|b| BonusDto {
                instrument_id: b.instrument_id.to_string(),
                record_date: b.record_date,
                ex_date: b.ex_date,
                split_factor: b.split_factor.to_string(),
                acquired_at: b.acquired_at,
            })
            .collect(),
        bars: BarsDto {
            relative_path: "bars.ndjson".into(),
            schema_id: "historical-price-only-bars".into(),
            schema_version: 1,
            sha256: bars_hash,
            size_bytes: bars_ndjson.len() as u64,
            row_count: 17688,
        },
    };
    let manifest = Manifest::from_unsigned(unsigned)?;
    let mut manifest_json = serde_json::to_vec(&manifest)
        .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    manifest_json.push(b'\n');
    validate_historical_price_only_artifact_bytes(
        candidate.content_hash(),
        &bars_ndjson,
        &manifest_json,
    )?;
    Ok(HistoricalPriceOnlyArtifactBytes {
        bars_ndjson,
        manifest_json,
    })
}

/// Strictly validates projected bytes.  The candidate hash is only a lineage
/// pin and directory input; no candidate preimage is recreated here.
#[allow(dead_code)]
fn validate_historical_price_only_artifact_bytes(
    expected_candidate_hash: &ContentHash,
    bars_ndjson: &[u8],
    manifest_json: &[u8],
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    if manifest_json.len() > MAX_MANIFEST_BYTES || bars_ndjson.len() > MAX_BARS_BYTES {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let validated_manifest = validate_manifest_bytes(expected_candidate_hash, manifest_json)?;
    let u = &validated_manifest.unsigned;
    if u.bars.size_bytes != bars_ndjson.len() as u64
        || u.bars.sha256 != ContentHash::from_bytes(bars_ndjson)
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    validate_bars(bars_ndjson, &u.sessions, &u.instruments)
}

fn validate_manifest_bytes(
    expected_candidate_hash: &ContentHash,
    manifest_json: &[u8],
) -> Result<ValidatedManifest, HistoricalPriceOnlyArtifactError> {
    if manifest_json.len() > MAX_MANIFEST_BYTES || !manifest_json.ends_with(b"\n") {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let manifest: Manifest = serde_json::from_slice(manifest_json)
        .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    let u = manifest.unsigned();
    if &u.candidate_content_sha256 != expected_candidate_hash
        || u.schema_id != "kis-historical-price-only-beta"
        || u.schema_version != 2
        || u.contract != crate::HISTORICAL_PRICE_ONLY_BETA_CONTRACT
        || u.materializer_version != HISTORICAL_PRICE_ONLY_MATERIALIZER_VERSION
        || u.audience != "OWNER_ONLY"
        || !u.vendor_snapshot
        || u.strict_pit
        || u.capability != "PRICE_RETURN_ONLY"
        || u.materialization_status != "MATERIALIZED"
        || u.registration_status != "UNREGISTERED"
        || u.publication_status != "NOT_PUBLISHED"
        || u.range_start.to_string() != "2020-01-31"
        || u.range_end.to_string() != "2026-08-19"
        || u.instrument_count != 11
        || u.session_count != 1608
        || u.row_count != 17688
        || u.price_scale != HISTORICAL_PRICE_ONLY_PRICE_SCALE
        || u.factor_scale != HISTORICAL_PRICE_ONLY_FACTOR_SCALE
        || u.stage5.file_count != 187
        || u.stage5.files.len() != 187
        || u.ksd.file_count != 7
        || u.ksd.cash_dividend_treatment_id != crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
        || u.ksd.ignored_cash_dividend_row_count == 0
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut expected_instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| format!("{symbol}.KRX"))
        .collect::<Vec<_>>();
    expected_instruments.sort();
    if u.instruments != expected_instruments
        || ContentHash::from_bytes(
            &serde_json::to_vec(&u)
                .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?,
        ) != manifest.manifest_sha256
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut canonical = serde_json::to_vec(&manifest)
        .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    canonical.push(b'\n');
    if canonical != manifest_json
        || u.bars.relative_path != "bars.ndjson"
        || u.bars.schema_id != "historical-price-only-bars"
        || u.bars.schema_version != 1
        || u.bars.row_count != 17688
        || u.bars.size_bytes > MAX_BARS_BYTES as u64
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    validate_manifest_semantics(&u)?;
    Ok(ValidatedManifest {
        approval_summary: manifest.approval_summary(),
        unsigned: u,
    })
}

fn validate_manifest_semantics(
    u: &UnsignedManifest,
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    let approved =
        crate::range_normalize::ExpectedRangeSessions::approved_xkrx(u.range_start, u.range_end)
            .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
    if u.sessions.len() != 1608
        || u.sessions.first().map(|s| s.session_date.to_string()) != Some("2020-01-31".into())
        || u.sessions.last().map(|s| s.session_date.to_string()) != Some("2026-08-19".into())
        || u.sessions
            .windows(2)
            .any(|pair| pair[0].session_date >= pair[1].session_date)
        || u.sessions
            .iter()
            .map(|s| s.session_date)
            .collect::<Vec<_>>()
            != approved.sessions
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut names = BTreeSet::new();
    if u.stage5.files.iter().any(|f| {
        f.kind != ResponseKind::Bars
            || f.file_name.is_empty()
            || f.file_name.contains('/')
            || f.file_name.contains("\\")
            || f.size_bytes == 0
            || !names.insert(&f.file_name)
    }) {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let expected_names = KR_ETF_CORE_SYMBOLS
        .iter()
        .flat_map(|symbol| {
            (1..=17).map(move |window| {
                format!("daily-bars-range-window-{window}-{symbol}-page-01.json")
            })
        })
        .collect::<Vec<_>>();
    if u.stage5
        .files
        .iter()
        .map(|file| file.file_name.clone())
        .collect::<Vec<_>>()
        != expected_names
    {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut bonuses = BTreeSet::new();
    let mut previous = None;
    for b in &u.bonus_evidence {
        let factor = parse_price(&b.split_factor)?;
        if !u.instruments.contains(&b.instrument_id)
            || b.record_date < u.range_start
            || b.record_date > u.range_end
            || b.ex_date < u.range_start
            || b.ex_date > u.range_end
            || factor.scale() != HISTORICAL_PRICE_ONLY_FACTOR_SCALE
            || factor <= FixedPoint::parse("1").expect("one")
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        let key = (&b.instrument_id, b.ex_date);
        if previous.as_ref().is_some_and(|old| old > &key) || !bonuses.insert(key) {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        previous = Some((&b.instrument_id, b.ex_date));
    }
    Ok(())
}

fn validate_bars(
    bytes: &[u8],
    sessions: &[SessionDto],
    instruments: &[String],
) -> Result<(), HistoricalPriceOnlyArtifactError> {
    if bytes.len() > MAX_BARS_BYTES {
        return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
    }
    let mut validator = BarValidator::new(sessions, instruments, None);
    validator.feed(bytes)?;
    validator.finish().map(|_| ())
}

struct BarValidator<'a> {
    sessions: &'a [SessionDto],
    instruments: &'a [String],
    expected_hash: Option<&'a ContentHash>,
    hasher: Sha256,
    line: Vec<u8>,
    seen: BTreeSet<(String, TradingDate)>,
    previous: Option<(String, TradingDate)>,
    count: usize,
    total_bytes: usize,
    saw_terminal_lf: bool,
    approved_bars: Vec<HistoricalPriceOnlyBar>,
}

impl<'a> BarValidator<'a> {
    fn new(
        sessions: &'a [SessionDto],
        instruments: &'a [String],
        expected_hash: Option<&'a ContentHash>,
    ) -> Self {
        Self {
            sessions,
            instruments,
            expected_hash,
            hasher: Sha256::new(),
            line: Vec::with_capacity(MAX_BAR_LINE_BYTES),
            seen: BTreeSet::new(),
            previous: None,
            count: 0,
            total_bytes: 0,
            saw_terminal_lf: false,
            approved_bars: Vec::with_capacity(17688),
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<(), HistoricalPriceOnlyArtifactError> {
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len())
            .ok_or(HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
        if self.total_bytes > MAX_BARS_BYTES {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        self.hasher.update(bytes);
        for byte in bytes {
            if *byte == b'\n' {
                self.validate_line()?;
                self.line.clear();
                self.saw_terminal_lf = true;
            } else {
                self.saw_terminal_lf = false;
                if self.line.len() >= MAX_BAR_LINE_BYTES {
                    return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
                }
                self.line.push(*byte);
            }
        }
        Ok(())
    }

    fn validate_line(&mut self) -> Result<(), HistoricalPriceOnlyArtifactError> {
        if self.line.is_empty() {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        let row: BarDto = serde_json::from_slice(&self.line)
            .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?;
        if serde_json::to_vec(&row)
            .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?
            != self.line
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        let key = (row.instrument_id.clone(), row.session_date);
        if self.previous.as_ref().is_some_and(|old| old >= &key) || !self.seen.insert(key.clone()) {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        self.previous = Some(key);
        let open = parse_price(&row.raw_open)?;
        let high = parse_price(&row.raw_high)?;
        let low = parse_price(&row.raw_low)?;
        let close = parse_price(&row.raw_close)?;
        if !self.instruments.contains(&row.instrument_id)
            || !low.is_positive()
            || high < low
            || open < low
            || open > high
            || close < low
            || close > high
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        let adjusted_open = parse_price(&row.adjusted_open)?;
        let adjusted_high = parse_price(&row.adjusted_high)?;
        let adjusted_low = parse_price(&row.adjusted_low)?;
        let adjusted_close = parse_price(&row.adjusted_close)?;
        if [adjusted_open, adjusted_high, adjusted_low, adjusted_close]
            .iter()
            .any(|value| !value.is_positive() || value.scale() != HISTORICAL_PRICE_ONLY_PRICE_SCALE)
            || adjusted_high < adjusted_low
            || adjusted_open < adjusted_low
            || adjusted_open > adjusted_high
            || adjusted_close < adjusted_low
            || adjusted_close > adjusted_high
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        if let Some(value) = &row.raw_trading_value
            && !parse_price(value)?.is_positive()
            && parse_price(value)? != FixedPoint::ZERO
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        self.count += 1;
        if self.count > 17688 {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        let raw_trading_value = row
            .raw_trading_value
            .as_deref()
            .map(parse_price)
            .transpose()?;
        self.approved_bars.push(HistoricalPriceOnlyBar {
            instrument_id: InstrumentId::parse(&row.instrument_id)
                .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?,
            session_date: row.session_date,
            raw_open: open,
            raw_high: high,
            raw_low: low,
            raw_close: close,
            raw_volume: row.raw_volume,
            raw_trading_value,
            adjusted_open,
            adjusted_high,
            adjusted_low,
            adjusted_close,
        });
        Ok(())
    }

    fn finish(self) -> Result<Vec<HistoricalPriceOnlyBar>, HistoricalPriceOnlyArtifactError> {
        let session_dates = self
            .sessions
            .iter()
            .map(|s| s.session_date)
            .collect::<BTreeSet<_>>();
        if !self.saw_terminal_lf
            || !self.line.is_empty()
            || self.count != 17688
            || self.seen.len() != 17688
            || self.instruments.iter().any(|instrument| {
                self.seen
                    .iter()
                    .filter(|(i, _)| i == instrument)
                    .map(|(_, date)| *date)
                    .collect::<BTreeSet<_>>()
                    != session_dates
            })
        {
            return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
        }
        if let Some(expected) = self.expected_hash {
            let digest = self.hasher.finalize();
            let hex = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if ContentHash::parse(&format!("sha256:{hex}"))
                .map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)?
                != *expected
            {
                return Err(HistoricalPriceOnlyArtifactError::InvalidArtifact);
            }
        }
        Ok(self.approved_bars)
    }
}

fn parse_price(value: &str) -> Result<FixedPoint, HistoricalPriceOnlyArtifactError> {
    FixedPoint::parse(value).map_err(|_| HistoricalPriceOnlyArtifactError::InvalidArtifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reseal(
        candidate: &HistoricalPriceOnlyCandidate,
        bars: Vec<u8>,
        manifest_bytes: &[u8],
    ) -> Vec<u8> {
        let manifest: Manifest = serde_json::from_slice(manifest_bytes).unwrap();
        let mut unsigned = manifest.unsigned();
        unsigned.bars.sha256 = ContentHash::from_bytes(&bars);
        unsigned.bars.size_bytes = bars.len() as u64;
        let manifest = Manifest::from_unsigned(unsigned).unwrap();
        let mut out = serde_json::to_vec(&manifest).unwrap();
        out.push(b'\n');
        assert_ne!(
            candidate.content_hash(),
            &ContentHash::from_bytes(bars.as_slice())
        );
        out
    }

    fn artifact() -> (
        HistoricalPriceOnlyCandidate,
        HistoricalPriceOnlyArtifactBytes,
    ) {
        let candidate = crate::historical_price_only::artifact_test_candidate();
        let artifact = project_historical_price_only_artifact(&candidate).unwrap();
        (candidate, artifact)
    }

    #[cfg(unix)]
    fn materialize_fixture(
        root: &Path,
        candidate: &HistoricalPriceOnlyCandidate,
        artifact: &HistoricalPriceOnlyArtifactBytes,
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let operator_root = root.join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let candidate_dir = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2")
            .join(format!(
                "candidate-sha256={}",
                candidate
                    .content_hash()
                    .as_str()
                    .strip_prefix("sha256:")
                    .unwrap()
            ));
        std::fs::create_dir_all(&candidate_dir).unwrap();
        for directory in [
            operator_root.join("kis-historical-price-only-beta"),
            operator_root
                .join("kis-historical-price-only-beta")
                .join("v2"),
            candidate_dir.clone(),
        ] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(candidate_dir.join("bars.ndjson"), &artifact.bars_ndjson).unwrap();
        std::fs::write(candidate_dir.join("manifest.json"), &artifact.manifest_json).unwrap();
        for file in [
            candidate_dir.join("bars.ndjson"),
            candidate_dir.join("manifest.json"),
        ] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        operator_root
    }

    #[cfg(unix)]
    fn fixture_candidate_dir(
        operator_root: &Path,
        candidate: &HistoricalPriceOnlyCandidate,
    ) -> PathBuf {
        operator_root
            .join("kis-historical-price-only-beta")
            .join("v2")
            .join(format!(
                "candidate-sha256={}",
                candidate
                    .content_hash()
                    .as_str()
                    .strip_prefix("sha256:")
                    .unwrap()
            ))
    }

    #[cfg(unix)]
    fn write_fixture() -> (tempfile::TempDir, HistoricalPriceOnlyCandidate, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let (candidate, artifact) = artifact();
        let operator_root = materialize_fixture(root.path(), &candidate, &artifact);
        (root, candidate, operator_root)
    }

    fn reseal_manifest(unsigned: UnsignedManifest) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&Manifest::from_unsigned(unsigned).unwrap()).unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn mutate_first_bar(
        artifact: &HistoricalPriceOnlyArtifactBytes,
        mutate: impl FnOnce(&mut BarDto),
    ) -> Vec<u8> {
        let newline = artifact
            .bars_ndjson
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap();
        let mut row: BarDto = serde_json::from_slice(&artifact.bars_ndjson[..newline]).unwrap();
        mutate(&mut row);
        let mut result = serde_json::to_vec(&row).unwrap();
        result.push(b'\n');
        result.extend_from_slice(&artifact.bars_ndjson[newline + 1..]);
        result
    }

    #[test]
    fn full_candidate_projection_round_trips_deterministically_without_raw_metadata() {
        let candidate = crate::historical_price_only::artifact_test_candidate();
        let first = project_historical_price_only_artifact(&candidate)
            .unwrap_or_else(|error| panic!("{error:?}"));
        let second = project_historical_price_only_artifact(&candidate).unwrap();
        assert_eq!(first, second);
        validate_historical_price_only_artifact_bytes(
            candidate.content_hash(),
            &first.bars_ndjson,
            &first.manifest_json,
        )
        .unwrap();
        let text = String::from_utf8(first.manifest_json).unwrap();
        for forbidden in [
            "request",
            "sentinel-request-secret",
            "sentinel-query",
            "sentinel-header",
            "sentinel-secret",
        ] {
            assert!(!text.contains(forbidden));
        }
    }

    #[test]
    fn typed_manifest_rejects_unknown_fields_and_hash_or_pin_tamper() {
        let candidate = crate::historical_price_only::artifact_test_candidate();
        let artifact = project_historical_price_only_artifact(&candidate).unwrap();
        let unknown = String::from_utf8(artifact.manifest_json.clone())
            .unwrap()
            .replacen("{", "{\"unexpected\":true,", 1)
            .into_bytes();
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &artifact.bars_ndjson,
                &unknown
            )
            .is_err()
        );
        let tampered = String::from_utf8(artifact.manifest_json.clone())
            .unwrap()
            .replace("\"OWNER_ONLY\"", "\"MEMBER\"")
            .into_bytes();
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &artifact.bars_ndjson,
                &tampered
            )
            .is_err()
        );
        assert!(
            validate_historical_price_only_artifact_bytes(
                &ContentHash::from_bytes(b"other"),
                &artifact.bars_ndjson,
                &artifact.manifest_json
            )
            .is_err()
        );
    }

    #[test]
    fn cash_dividend_treatment_fields_are_manifest_bound() {
        let candidate = crate::historical_price_only::artifact_test_candidate();
        let artifact = project_historical_price_only_artifact(&candidate).unwrap();
        let manifest: Manifest = serde_json::from_slice(&artifact.manifest_json).unwrap();
        for index in 0..5 {
            let mut unsigned = manifest.unsigned();
            match index {
                0 => unsigned.ksd.cash_dividend_treatment_id = "OTHER".into(),
                1 => unsigned.ksd.ignored_cash_dividend_row_count = 0,
                2 => {
                    unsigned.ksd.ignored_cash_dividend_rows_sha256 =
                        ContentHash::from_bytes(b"other-rows")
                }
                3 => {
                    unsigned.ksd.ignored_cash_dividend_source_file_sha256 =
                        ContentHash::from_bytes(b"other-source")
                }
                _ => {
                    unsigned.ksd.ignored_cash_dividend_acquired_at =
                        UtcTimestamp::parse_rfc3339("2026-08-20T00:00:00Z").unwrap()
                }
            }
            let resealed = reseal_manifest(unsigned);
            assert_ne!(resealed, artifact.manifest_json);
            if index < 2 {
                assert!(
                    validate_historical_price_only_artifact_bytes(
                        candidate.content_hash(),
                        &artifact.bars_ndjson,
                        &resealed,
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn terminal_newline_is_semantic_even_when_every_digest_is_resealed() {
        let candidate = crate::historical_price_only::artifact_test_candidate();
        let artifact = project_historical_price_only_artifact(&candidate).unwrap();
        let mut bars = artifact.bars_ndjson.clone();
        assert_eq!(bars.pop(), Some(b'\n'));
        let manifest = reseal(&candidate, bars.clone(), &artifact.manifest_json);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &bars,
                &manifest
            )
            .is_err()
        );
    }

    #[test]
    fn unsafe_writer_and_reader_do_not_write() {
        let root = tempfile::tempdir().unwrap();
        let candidate = crate::historical_price_only::artifact_test_candidate();
        assert!(matches!(
            write_historical_price_only_artifact(&root.path().join("missing"), &candidate),
            Err(HistoricalPriceOnlyArtifactError::Io(_))
        ));
        #[cfg(unix)]
        assert!(matches!(
            read_historical_price_only_artifact(root.path(), candidate.content_hash()),
            Err(HistoricalPriceOnlyArtifactError::UnsafePath)
        ));
        #[cfg(not(unix))]
        assert!(matches!(
            read_historical_price_only_artifact(root.path(), candidate.content_hash()),
            Err(HistoricalPriceOnlyArtifactError::UnsupportedPlatform)
        ));
        assert!(std::fs::read_dir(root.path()).unwrap().next().is_none());
    }

    #[test]
    fn verified_debug_redacts_operator_root() {
        let verified = VerifiedHistoricalPriceOnlyArtifact {
            path: PathBuf::from("/operator-root-sentinel-must-not-leak"),
            candidate_content_sha256: ContentHash::from_bytes(b"candidate"),
            approval_summary: HistoricalPriceOnlyArtifactApprovalSummary {
                artifact_manifest_sha256: ContentHash::from_bytes(b"manifest"),
                stage5_manifest_sha256: ContentHash::from_bytes(b"stage5"),
                action_manifest_sha256: ContentHash::from_bytes(b"action"),
                cash_dividend_treatment_id: crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
                    .into(),
                ignored_cash_dividend_row_count: 1,
                ignored_cash_dividend_rows_sha256: ContentHash::from_bytes(b"dividend-rows"),
                ignored_cash_dividend_source_file_sha256: ContentHash::from_bytes(b"dividend-file"),
                ignored_cash_dividend_acquired_at: UtcTimestamp::parse_rfc3339(
                    "2026-08-19T00:00:00Z",
                )
                .unwrap(),
                schema_id: "kis-historical-price-only-beta".into(),
                schema_version: 2,
                audience: "OWNER_ONLY".into(),
                vendor_snapshot: true,
                strict_pit: false,
                capability: "PRICE_RETURN_ONLY".into(),
                materialization_status: "MATERIALIZED".into(),
                registration_status: "UNREGISTERED".into(),
                publication_status: "NOT_PUBLISHED".into(),
                range_start: TradingDate::parse("2020-01-31").unwrap(),
                range_end: TradingDate::parse("2026-08-19").unwrap(),
                instruments: vec!["069500.KRX".into()],
                instrument_count: 1,
                session_count: 1,
                bar_count: 1,
            },
            approved_bars: Vec::new(),
        };
        let debug = format!("{verified:?}");
        assert!(debug.contains("VerifiedHistoricalPriceOnlyArtifact"));
        assert!(debug.contains(verified.candidate_content_sha256().as_str()));
        assert!(!debug.contains("operator-root-sentinel-must-not-leak"));
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_safe_reader_accepts_fixture_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let (candidate, artifact) = artifact();
        let operator_root = materialize_fixture(root.path(), &candidate, &artifact);
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        let before_bars = std::fs::read(candidate_dir.join("bars.ndjson")).unwrap();
        let before_manifest = std::fs::read(candidate_dir.join("manifest.json")).unwrap();
        let before_entries = std::fs::read_dir(&candidate_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>();
        let verified =
            read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                .expect("valid descriptor-safe fixture");
        assert_eq!(
            verified.path(),
            &operator_root
                .join("kis-historical-price-only-beta")
                .join("v2")
                .join(format!(
                    "candidate-sha256={}",
                    candidate
                        .content_hash()
                        .as_str()
                        .strip_prefix("sha256:")
                        .unwrap()
                ))
        );
        assert_eq!(
            verified.candidate_content_sha256(),
            candidate.content_hash()
        );
        let summary = verified.approval_summary();
        assert_eq!(
            summary.stage5_manifest_sha256(),
            candidate.source_manifest_hash()
        );
        assert_eq!(
            summary.action_manifest_sha256(),
            candidate.action_manifest_hash()
        );
        assert_eq!(summary.schema_id(), "kis-historical-price-only-beta");
        assert_eq!(summary.schema_version(), 2);
        assert_eq!(
            summary.cash_dividend_treatment_id(),
            crate::HISTORICAL_PRICE_ONLY_CASH_DIVIDEND_TREATMENT
        );
        assert_eq!(summary.ignored_cash_dividend_row_count(), 1);
        assert_eq!(
            summary.ignored_cash_dividend_rows_sha256(),
            candidate.ignored_cash_dividends().rows_sha256()
        );
        assert_eq!(summary.audience(), "OWNER_ONLY");
        assert!(summary.vendor_snapshot());
        assert!(!summary.strict_pit());
        assert_eq!(summary.capability(), "PRICE_RETURN_ONLY");
        assert_eq!(summary.materialization_status(), "MATERIALIZED");
        assert_eq!(summary.registration_status(), "UNREGISTERED");
        assert_eq!(summary.publication_status(), "NOT_PUBLISHED");
        assert_eq!(summary.range_start().to_string(), "2020-01-31");
        assert_eq!(summary.range_end().to_string(), "2026-08-19");
        assert_eq!(summary.instruments().len(), 11);
        assert_eq!(summary.instrument_count(), 11);
        assert_eq!(summary.session_count(), 1608);
        assert_eq!(summary.bar_count(), 17688);
        assert_eq!(verified.approved_bars().len(), 17_688);
        assert_eq!(
            std::fs::read(verified.path().join("bars.ndjson")).unwrap(),
            before_bars
        );
        assert_eq!(
            std::fs::read(verified.path().join("manifest.json")).unwrap(),
            before_manifest
        );
        assert_eq!(
            std::fs::read_dir(verified.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<BTreeSet<_>>(),
            before_entries
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_safe_writer_publishes_golden_candidate() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        let verified = write_historical_price_only_artifact(&operator_root, &candidate)
            .expect("writer should publish a valid candidate");
        let second = write_historical_price_only_artifact(&operator_root, &candidate)
            .expect("identical writer should be idempotent");
        assert_eq!(second, verified);
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        assert_eq!(verified.path(), candidate_dir.as_path());
        assert_eq!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).unwrap(),
            verified
        );
        assert_eq!(
            std::fs::read(candidate_dir.join("bars.ndjson")).unwrap(),
            expected.bars_ndjson
        );
        assert_eq!(
            std::fs::read(candidate_dir.join("manifest.json")).unwrap(),
            expected.manifest_json
        );
        for bytes in [
            std::fs::read(candidate_dir.join("bars.ndjson")).unwrap(),
            std::fs::read(candidate_dir.join("manifest.json")).unwrap(),
        ] {
            let text = String::from_utf8(bytes).unwrap();
            for forbidden in [
                "sentinel-request-secret",
                "sentinel-query",
                "sentinel-header",
                "sentinel-secret",
                "Raw",
                "Curated",
                "READY",
            ] {
                assert!(!text.contains(forbidden));
            }
        }
        let candidate_metadata = std::fs::metadata(&candidate_dir).unwrap();
        assert_eq!(candidate_metadata.mode() & 0o777, 0o700);
        for name in ["bars.ndjson", "manifest.json"] {
            let metadata = std::fs::metadata(candidate_dir.join(name)).unwrap();
            assert_eq!(metadata.mode() & 0o777, 0o600);
            assert_eq!(metadata.nlink(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn writer_preserves_differing_existing_destination() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, _) = artifact();
        let first = write_historical_price_only_artifact(&operator_root, &candidate).unwrap();
        let bars = first.path().join("bars.ndjson");
        let mut tampered = std::fs::read(&bars).unwrap();
        tampered[0] ^= 1;
        std::fs::write(&bars, &tampered).unwrap();
        assert!(matches!(
            write_historical_price_only_artifact(&operator_root, &candidate),
            Err(HistoricalPriceOnlyArtifactError::Conflict { .. })
        ));
        assert_eq!(std::fs::read(bars).unwrap(), tampered);
    }

    #[cfg(unix)]
    struct FailWriterAt(ArtifactWriteStage);

    #[cfg(unix)]
    impl ArtifactWriteOps for FailWriterAt {
        fn checkpoint(
            &self,
            stage: ArtifactWriteStage,
        ) -> Result<(), HistoricalPriceOnlyArtifactError> {
            if stage == self.0 {
                Err(HistoricalPriceOnlyArtifactError::InvalidArtifact)
            } else {
                Ok(())
            }
        }
    }

    #[cfg(unix)]
    struct FailPublishedVersionSync;

    #[cfg(unix)]
    impl ArtifactWriteOps for FailPublishedVersionSync {
        fn sync_published_version(
            &self,
            _version: &std::os::fd::OwnedFd,
        ) -> Result<(), HistoricalPriceOnlyArtifactError> {
            Err(HistoricalPriceOnlyArtifactError::Io(std::io::Error::other(
                "injected",
            )))
        }
    }

    #[cfg(unix)]
    struct UnsupportedPublish;

    #[cfg(unix)]
    impl ArtifactWriteOps for UnsupportedPublish {
        fn publish(
            &self,
            _version: &std::os::fd::OwnedFd,
            _staging: &[u8],
            _destination: &[u8],
        ) -> Result<PublishOutcome, HistoricalPriceOnlyArtifactError> {
            Err(HistoricalPriceOnlyArtifactError::UnsupportedAtomicNoReplace)
        }
    }

    #[cfg(unix)]
    struct WriterAction {
        stage: ArtifactWriteStage,
        action: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    #[cfg(unix)]
    impl ArtifactWriteOps for WriterAction {
        fn checkpoint(
            &self,
            stage: ArtifactWriteStage,
        ) -> Result<(), HistoricalPriceOnlyArtifactError> {
            if stage == self.stage
                && let Some(action) = self.action.lock().unwrap().take()
            {
                action();
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    #[test]
    fn injected_before_rename_failure_cleans_staging_without_final() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        let result = write_historical_price_only_artifact_with_ops(
            &operator_root,
            &candidate,
            &expected,
            &FailWriterAt(ArtifactWriteStage::BeforeRename),
        );
        assert!(matches!(
            result,
            Err(HistoricalPriceOnlyArtifactError::InvalidArtifact)
        ));
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        assert_eq!(std::fs::read_dir(&version).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn partial_manifest_population_cleanup_uses_written_snapshot() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &FailWriterAt(ArtifactWriteStage::Manifest),
            ),
            Err(HistoricalPriceOnlyArtifactError::InvalidArtifact)
        ));
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        assert_eq!(std::fs::read_dir(version).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn extra_staging_entry_before_publish_fails_without_final_or_extra_cleanup() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        let action_root = operator_root.clone();
        let action = WriterAction {
            stage: ArtifactWriteStage::BeforeRename,
            action: std::sync::Mutex::new(Some(Box::new(move || {
                let version = action_root
                    .join("kis-historical-price-only-beta")
                    .join("v2");
                let stage = std::fs::read_dir(&version)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".stage-pid-"))
                    })
                    .unwrap();
                std::fs::write(stage.join("unexpected"), b"keep-extra").unwrap();
            }))),
        };
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &action,
            ),
            Err(HistoricalPriceOnlyArtifactError::CleanupFailed)
        ));
        assert!(!fixture_candidate_dir(&operator_root, &candidate).exists());
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        let stage = std::fs::read_dir(version)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".stage-pid-"))
            })
            .expect("cleanup must preserve the unverified extra entry");
        assert_eq!(
            std::fs::read(stage.join("unexpected")).unwrap(),
            b"keep-extra"
        );
    }

    #[cfg(unix)]
    #[test]
    fn staging_leaf_substitutions_are_never_unlinked_by_cleanup() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        for symlink_substitution in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let operator_root = root.path().join("operator");
            std::fs::create_dir(&operator_root).unwrap();
            std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            let (candidate, expected) = artifact();
            let action_root = operator_root.clone();
            let saved = root.path().join("saved-manifest.json");
            let saved_for_action = saved.clone();
            let action = WriterAction {
                stage: ArtifactWriteStage::BeforeRename,
                action: std::sync::Mutex::new(Some(Box::new(move || {
                    let version = action_root
                        .join("kis-historical-price-only-beta")
                        .join("v2");
                    let stage = std::fs::read_dir(&version)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .find(|path| {
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.starts_with(".stage-pid-"))
                        })
                        .unwrap();
                    let manifest = stage.join("manifest.json");
                    std::fs::rename(&manifest, &saved_for_action).unwrap();
                    if symlink_substitution {
                        symlink(&saved_for_action, &manifest).unwrap();
                    } else {
                        std::fs::write(&manifest, b"replacement").unwrap();
                        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600))
                            .unwrap();
                    }
                }))),
            };
            assert!(matches!(
                write_historical_price_only_artifact_with_ops(
                    &operator_root,
                    &candidate,
                    &expected,
                    &action,
                ),
                Err(HistoricalPriceOnlyArtifactError::CleanupFailed)
            ));
            assert!(!fixture_candidate_dir(&operator_root, &candidate).exists());
            assert!(saved.exists());
            let version = operator_root
                .join("kis-historical-price-only-beta")
                .join("v2");
            let stage = std::fs::read_dir(version)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(".stage-pid-"))
                })
                .expect("cleanup must preserve the substituted leaf");
            let manifest = stage.join("manifest.json");
            if symlink_substitution {
                assert_eq!(std::fs::read_link(manifest).unwrap(), saved);
            } else {
                assert_eq!(std::fs::read(manifest).unwrap(), b"replacement");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn bars_create_failure_does_not_unlink_preexisting_object() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        let action_root = operator_root.clone();
        let action = WriterAction {
            stage: ArtifactWriteStage::Manifest,
            action: std::sync::Mutex::new(Some(Box::new(move || {
                let version = action_root
                    .join("kis-historical-price-only-beta")
                    .join("v2");
                let stage = std::fs::read_dir(&version)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".stage-pid-"))
                    })
                    .unwrap();
                std::fs::write(stage.join("bars.ndjson"), b"preexisting-bars").unwrap();
                std::fs::set_permissions(
                    stage.join("bars.ndjson"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }))),
        };
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &action,
            ),
            Err(HistoricalPriceOnlyArtifactError::CleanupFailed)
        ));
        assert!(!fixture_candidate_dir(&operator_root, &candidate).exists());
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        let stage = std::fs::read_dir(version)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".stage-pid-"))
            })
            .expect("unverified bars object must not be removed");
        assert_eq!(
            std::fs::read(stage.join("bars.ndjson")).unwrap(),
            b"preexisting-bars"
        );
    }

    #[cfg(unix)]
    #[test]
    fn injected_after_rename_is_indeterminate_and_retry_is_idempotent() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &FailWriterAt(ArtifactWriteStage::AfterRename),
            ),
            Err(HistoricalPriceOnlyArtifactError::IndeterminateCommit)
        ));
        let verified = write_historical_price_only_artifact(&operator_root, &candidate).unwrap();
        assert_eq!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).unwrap(),
            verified
        );
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        assert_eq!(std::fs::read_dir(version).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn injected_version_fsync_failure_is_indeterminate_and_preserves_final() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &FailPublishedVersionSync,
            ),
            Err(HistoricalPriceOnlyArtifactError::IndeterminateCommit)
        ));
        let verified = write_historical_price_only_artifact(&operator_root, &candidate).unwrap();
        assert_eq!(
            verified.path(),
            fixture_candidate_dir(&operator_root, &candidate)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_atomic_publish_is_typed_and_cleans_staging() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, expected) = artifact();
        assert!(matches!(
            write_historical_price_only_artifact_with_ops(
                &operator_root,
                &candidate,
                &expected,
                &UnsupportedPublish,
            ),
            Err(HistoricalPriceOnlyArtifactError::UnsupportedAtomicNoReplace)
        ));
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        assert_eq!(std::fs::read_dir(version).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_same_hash_writers_publish_one_verified_final() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{Arc, Barrier};

        let root = tempfile::tempdir().unwrap();
        let operator_root = root.path().join("operator");
        std::fs::create_dir(&operator_root).unwrap();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        let (candidate, _) = artifact();
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let first_root = operator_root.clone();
            let first_candidate = candidate.clone();
            let first_barrier = Arc::clone(&barrier);
            let first = scope.spawn(move || {
                first_barrier.wait();
                write_historical_price_only_artifact(&first_root, &first_candidate)
            });
            let second_root = operator_root.clone();
            let second_candidate = candidate.clone();
            let second_barrier = Arc::clone(&barrier);
            let second = scope.spawn(move || {
                second_barrier.wait();
                write_historical_price_only_artifact(&second_root, &second_candidate)
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        let first = results.0.expect("first writer");
        let second = results.1.expect("second writer");
        assert_eq!(first, second);
        assert_eq!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).unwrap(),
            first
        );
        let version = operator_root
            .join("kis-historical-price-only-beta")
            .join("v2");
        assert_eq!(std::fs::read_dir(version).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn writer_rejects_control_and_version_substitutions() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        for substitute_version in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let operator_root = root.path().join("operator");
            std::fs::create_dir(&operator_root).unwrap();
            std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            let target = root.path().join("outside");
            std::fs::create_dir(&target).unwrap();
            let control = operator_root.join("kis-historical-price-only-beta");
            if substitute_version {
                std::fs::create_dir(&control).unwrap();
                std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, control.join("v2")).unwrap();
            } else {
                symlink(&target, &control).unwrap();
            }
            let (candidate, _) = artifact();
            assert!(write_historical_price_only_artifact(&operator_root, &candidate).is_err());
            assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn writer_rejects_destination_substitutions_without_overwrite() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        for destination_kind in [0u8, 1, 2] {
            let root = tempfile::tempdir().unwrap();
            let operator_root = root.path().join("operator");
            std::fs::create_dir(&operator_root).unwrap();
            std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            let (candidate, _) = artifact();
            let control = operator_root.join("kis-historical-price-only-beta");
            let version = control.join("v2");
            std::fs::create_dir_all(&version).unwrap();
            std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::set_permissions(&version, std::fs::Permissions::from_mode(0o700)).unwrap();
            let destination = fixture_candidate_dir(&operator_root, &candidate);
            match destination_kind {
                0 => {
                    let target = root.path().join("target");
                    std::fs::create_dir(&target).unwrap();
                    symlink(&target, &destination).unwrap();
                }
                1 => std::fs::write(&destination, b"keep").unwrap(),
                _ => {
                    std::fs::create_dir(&destination).unwrap();
                    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o750))
                        .unwrap();
                }
            }
            assert!(matches!(
                write_historical_price_only_artifact(&operator_root, &candidate),
                Err(HistoricalPriceOnlyArtifactError::Conflict { .. })
            ));
            if destination_kind == 1 {
                assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
            }
            assert_eq!(std::fs::read_dir(&version).unwrap().count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn writer_rejects_staging_and_held_path_replacements_before_publish() {
        use std::os::unix::fs::PermissionsExt;

        for replacement in 0u8..3 {
            let root = tempfile::tempdir().unwrap();
            let operator_root = root.path().join("operator");
            std::fs::create_dir(&operator_root).unwrap();
            std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o750))
                .unwrap();
            let (candidate, expected) = artifact();
            let action_root = operator_root.clone();
            let action = WriterAction {
                stage: ArtifactWriteStage::BeforeRename,
                action: std::sync::Mutex::new(Some(Box::new(move || {
                    let version = action_root
                        .join("kis-historical-price-only-beta")
                        .join("v2");
                    let stage = std::fs::read_dir(&version)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .find(|path| {
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.starts_with(".stage-pid-"))
                        })
                        .unwrap();
                    match replacement {
                        0 => {
                            let moved = stage.with_extension("moved");
                            std::fs::rename(&stage, &moved).unwrap();
                            std::fs::create_dir(&stage).unwrap();
                            std::fs::set_permissions(
                                &stage,
                                std::fs::Permissions::from_mode(0o700),
                            )
                            .unwrap();
                        }
                        1 => {
                            let leaf = stage.join("manifest.json");
                            let moved = stage.join("manifest.moved");
                            std::fs::rename(&leaf, &moved).unwrap();
                            std::fs::write(&leaf, b"replacement").unwrap();
                            std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o600))
                                .unwrap();
                        }
                        _ => {
                            let moved = action_root.with_extension("moved");
                            std::fs::rename(&action_root, &moved).unwrap();
                            std::fs::create_dir(&action_root).unwrap();
                            std::fs::set_permissions(
                                &action_root,
                                std::fs::Permissions::from_mode(0o750),
                            )
                            .unwrap();
                        }
                    }
                }))),
            };
            assert!(
                write_historical_price_only_artifact_with_ops(
                    &operator_root,
                    &candidate,
                    &expected,
                    &action,
                )
                .is_err()
            );
            assert!(!fixture_candidate_dir(&operator_root, &candidate).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_operator_roots_fail_closed_before_opening() {
        let (_root, candidate, operator_root) = write_fixture();
        for unsafe_root in [
            PathBuf::from("relative"),
            PathBuf::from("/"),
            PathBuf::from("."),
            PathBuf::from(".."),
            operator_root.parent().unwrap().join("operator//nested"),
            PathBuf::from(format!("{}/", operator_root.display())),
        ] {
            assert!(matches!(
                read_historical_price_only_artifact(&unsafe_root, candidate.content_hash()),
                Err(HistoricalPriceOnlyArtifactError::UnsafePath)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_ancestors_and_layout_entries_fail_closed() {
        use std::os::unix::fs::symlink;

        let (root, candidate, operator_root) = write_fixture();
        let actual_parent = root.path().join("actual-parent");
        std::fs::create_dir(&actual_parent).unwrap();
        let actual = actual_parent.join("operator");
        std::fs::rename(&operator_root, &actual).unwrap();
        symlink(&actual_parent, root.path().join("link")).unwrap();
        assert!(
            read_historical_price_only_artifact(
                &root.path().join("link").join("operator"),
                candidate.content_hash()
            )
            .is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let control = operator_root.join("kis-historical-price-only-beta");
        let control_saved = operator_root.join("control-saved");
        std::fs::rename(&control, &control_saved).unwrap();
        symlink(&control_saved, &control).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        let candidate_saved = candidate_dir.with_file_name("candidate-saved");
        std::fs::rename(&candidate_dir, &candidate_saved).unwrap();
        symlink(&candidate_saved, &candidate_dir).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        for leaf in ["bars.ndjson", "manifest.json"] {
            let (_root, candidate, operator_root) = write_fixture();
            let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
            let target = candidate_dir.join(leaf);
            let saved = target.with_extension("saved");
            std::fs::rename(&target, &saved).unwrap();
            symlink(&saved, &target).unwrap();
            assert!(
                read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_leaves_and_unsafe_modes_fail_closed() {
        use std::os::unix::fs::PermissionsExt;

        for leaf in ["bars.ndjson", "manifest.json"] {
            let (_root, candidate, operator_root) = write_fixture();
            let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
            std::fs::hard_link(
                candidate_dir.join(leaf),
                operator_root.parent().unwrap().join(format!("{leaf}.link")),
            )
            .unwrap();
            assert!(
                read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                    .is_err()
            );
        }

        let (_root, candidate, operator_root) = write_fixture();
        std::fs::set_permissions(&operator_root, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        for relative in [
            PathBuf::from("kis-historical-price-only-beta"),
            PathBuf::from("kis-historical-price-only-beta/v2"),
        ] {
            let (_root, candidate, operator_root) = write_fixture();
            let path = operator_root.join(relative);
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750)).unwrap();
            assert!(
                read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                    .is_err()
            );
        }

        let (_root, candidate, operator_root) = write_fixture();
        std::fs::set_permissions(
            fixture_candidate_dir(&operator_root, &candidate),
            std::fs::Permissions::from_mode(0o750),
        )
        .unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        for leaf in ["bars.ndjson", "manifest.json"] {
            let (_root, candidate, operator_root) = write_fixture();
            let path = fixture_candidate_dir(&operator_root, &candidate).join(leaf);
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(
                read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn candidate_extras_missing_entries_and_non_directories_fail_closed() {
        use std::os::unix::fs::PermissionsExt;

        for extra in ["extra", "nested"] {
            let (_root, candidate, operator_root) = write_fixture();
            let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
            if extra == "nested" {
                std::fs::create_dir(candidate_dir.join(extra)).unwrap();
            } else {
                std::fs::write(candidate_dir.join(extra), b"extra").unwrap();
            }
            assert!(
                read_historical_price_only_artifact(&operator_root, candidate.content_hash())
                    .is_err()
            );
        }

        let (_root, candidate, operator_root) = write_fixture();
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        let directory = rustix::fs::open(
            &candidate_dir,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        rustix::fs::mkfifoat(
            &directory,
            &b"fifo"[..],
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        std::fs::remove_file(candidate_dir.join("manifest.json")).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let control = operator_root.join("kis-historical-price-only-beta");
        std::fs::remove_dir_all(&control).unwrap();
        std::fs::write(&control, b"not a directory").unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        std::fs::remove_file(candidate_dir.join("bars.ndjson")).unwrap();
        std::fs::create_dir(candidate_dir.join("bars.ndjson")).unwrap();
        std::fs::set_permissions(
            candidate_dir.join("bars.ndjson"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn metadata_size_caps_reject_before_reading() {
        use std::os::unix::fs::PermissionsExt;

        let (_root, candidate, operator_root) = write_fixture();
        let manifest = fixture_candidate_dir(&operator_root, &candidate).join("manifest.json");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&manifest)
            .unwrap();
        file.set_len((MAX_MANIFEST_BYTES + 1) as u64).unwrap();
        drop(file);
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let bars = fixture_candidate_dir(&operator_root, &candidate).join("bars.ndjson");
        let file = std::fs::OpenOptions::new().write(true).open(&bars).unwrap();
        file.set_len((MAX_BARS_BYTES + 1) as u64).unwrap();
        drop(file);
        std::fs::set_permissions(&bars, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn reader_rejects_bytes_hash_and_directory_pin_tamper() {
        let (_root, candidate, operator_root) = write_fixture();
        let candidate_dir = fixture_candidate_dir(&operator_root, &candidate);
        let bars = candidate_dir.join("bars.ndjson");
        let mut bytes = std::fs::read(&bars).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&bars, bytes).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, candidate, operator_root) = write_fixture();
        let manifest = fixture_candidate_dir(&operator_root, &candidate).join("manifest.json");
        let mut bytes = std::fs::read(&manifest).unwrap();
        bytes[0] = b' ';
        std::fs::write(&manifest, bytes).unwrap();
        assert!(
            read_historical_price_only_artifact(&operator_root, candidate.content_hash()).is_err()
        );

        let (_root, _candidate, operator_root) = write_fixture();
        let other = ContentHash::from_bytes(b"other-candidate");
        assert!(read_historical_price_only_artifact(&operator_root, &other).is_err());
    }

    #[cfg(unix)]
    struct ReplaceAfterOpen {
        stage: ArtifactReadStage,
        action: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    #[cfg(unix)]
    impl ArtifactReadOps for ReplaceAfterOpen {
        fn after_open(&self, stage: ArtifactReadStage) {
            if stage == self.stage
                && let Some(action) = self.action.lock().unwrap().take()
            {
                action();
            }
        }
    }

    #[cfg(unix)]
    fn replace_directory_after_open(path: PathBuf) -> ReplaceAfterOpen {
        ReplaceAfterOpen {
            stage: ArtifactReadStage::Candidate,
            action: std::sync::Mutex::new(Some(Box::new(move || {
                use std::os::unix::fs::PermissionsExt;
                let moved = path.with_extension("moved");
                std::fs::rename(&path, &moved).unwrap();
                std::fs::create_dir(&path).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }))),
        }
    }

    #[cfg(unix)]
    #[test]
    fn deterministic_replacement_after_open_is_rejected_by_final_name_checks() {
        use std::os::unix::fs::PermissionsExt;

        {
            let (_root, candidate, operator_root) = write_fixture();
            let path = operator_root.clone();
            let hook = ReplaceAfterOpen {
                stage: ArtifactReadStage::Root,
                action: std::sync::Mutex::new(Some(Box::new(move || {
                    let moved = path.with_extension("moved");
                    std::fs::rename(&path, &moved).unwrap();
                    std::fs::create_dir(&path).unwrap();
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
                }))),
            };
            assert!(
                read_historical_price_only_artifact_with_ops(
                    &operator_root,
                    candidate.content_hash(),
                    &hook,
                )
                .is_err()
            );
        }

        {
            let (_root, candidate, operator_root) = write_fixture();
            let path = fixture_candidate_dir(&operator_root, &candidate);
            let hook = replace_directory_after_open(path);
            assert!(
                read_historical_price_only_artifact_with_ops(
                    &operator_root,
                    candidate.content_hash(),
                    &hook,
                )
                .is_err()
            );
        }

        for stage_name in [ArtifactReadStage::Manifest, ArtifactReadStage::Bars] {
            let (_root, candidate, operator_root) = write_fixture();
            let leaf = if stage_name == ArtifactReadStage::Manifest {
                "manifest.json"
            } else {
                "bars.ndjson"
            };
            let path = fixture_candidate_dir(&operator_root, &candidate).join(leaf);
            let hook = ReplaceAfterOpen {
                stage: stage_name,
                action: std::sync::Mutex::new(Some(Box::new(move || {
                    let moved = path.with_extension("moved");
                    std::fs::rename(&path, &moved).unwrap();
                    std::fs::write(&path, b"replacement").unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                }))),
            };
            assert!(
                read_historical_price_only_artifact_with_ops(
                    &operator_root,
                    candidate.content_hash(),
                    &hook,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn every_nested_manifest_dto_rejects_unknown_fields() {
        let (_, artifact) = artifact();
        let text = String::from_utf8(artifact.manifest_json).unwrap();
        for needle in [
            "\"stage5\":{",
            "\"files\":[{",
            "\"ksd\":{",
            "\"sessions\":[{",
            "\"bars\":{",
        ] {
            let mutated = text.replacen(needle, &format!("{needle}\"unknown\":true,"), 1);
            assert!(
                serde_json::from_str::<Manifest>(&mutated).is_err(),
                "{needle}"
            );
        }
        let manifest: Manifest = serde_json::from_slice(text.as_bytes()).unwrap();
        let mut unsigned = manifest.unsigned();
        unsigned.bonus_evidence.push(BonusDto {
            instrument_id: unsigned.instruments[0].clone(),
            record_date: TradingDate::parse("2020-01-31").unwrap(),
            ex_date: TradingDate::parse("2020-01-31").unwrap(),
            split_factor: "2.00000000".into(),
            acquired_at: UtcTimestamp::parse_rfc3339("2026-08-19T00:00:00Z").unwrap(),
        });
        let bonus = String::from_utf8(reseal_manifest(unsigned)).unwrap();
        let mutated = bonus.replacen(
            "\"bonus_evidence\":[{",
            "\"bonus_evidence\":[{\"unknown\":true,",
            1,
        );
        assert!(serde_json::from_str::<Manifest>(&mutated).is_err(), "bonus");
        let bar = String::from_utf8(artifact.bars_ndjson).unwrap();
        let mutated = bar.replacen("{", "{\"unknown\":true,", 1);
        assert!(serde_json::from_str::<BarDto>(mutated.lines().next().unwrap()).is_err());
    }

    #[test]
    fn bars_parser_rejects_utf8_blank_and_noncanonical_forms_after_reseal() {
        let (candidate, artifact) = artifact();
        let mut invalid_utf8 = artifact.bars_ndjson.clone();
        invalid_utf8[0] = 0xff;
        let manifest = reseal(&candidate, invalid_utf8.clone(), &artifact.manifest_json);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &invalid_utf8,
                &manifest
            )
            .is_err()
        );
        let mut blank = artifact.bars_ndjson.clone();
        let position = blank.iter().position(|b| *b == b'\n').unwrap() + 1;
        blank.insert(position, b'\n');
        let manifest = reseal(&candidate, blank.clone(), &artifact.manifest_json);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &blank,
                &manifest
            )
            .is_err()
        );
        let mut whitespace = artifact.bars_ndjson.clone();
        whitespace.insert(0, b' ');
        let manifest = reseal(&candidate, whitespace.clone(), &artifact.manifest_json);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &whitespace,
                &manifest
            )
            .is_err()
        );
    }

    #[test]
    fn resealed_valid_context_reaches_raw_adjusted_and_ordering_checks() {
        let (candidate, artifact) = artifact();
        for mutate in [
            |row: &mut BarDto| row.raw_low = "12.00".into(),
            |row: &mut BarDto| row.adjusted_low = "12.0000".into(),
            |row: &mut BarDto| row.adjusted_close = "0.0000".into(),
            |row: &mut BarDto| row.adjusted_open = "10.00".into(),
        ] {
            let bars = mutate_first_bar(&artifact, mutate);
            let manifest = reseal(&candidate, bars.clone(), &artifact.manifest_json);
            assert!(
                validate_historical_price_only_artifact_bytes(
                    candidate.content_hash(),
                    &bars,
                    &manifest
                )
                .is_err()
            );
        }
        let first_end = artifact
            .bars_ndjson
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap()
            + 1;
        let second_end = artifact.bars_ndjson[first_end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap()
            + first_end
            + 1;
        let mut reordered = artifact.bars_ndjson.clone();
        let first = reordered[..first_end].to_vec();
        let second = reordered[first_end..second_end].to_vec();
        reordered[..second_end].copy_from_slice(&[second, first].concat());
        let manifest = reseal(&candidate, reordered.clone(), &artifact.manifest_json);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &reordered,
                &manifest
            )
            .is_err()
        );
    }

    #[test]
    fn resealed_bonus_factor_one_is_rejected() {
        let (candidate, artifact) = artifact();
        let manifest: Manifest = serde_json::from_slice(&artifact.manifest_json).unwrap();
        let mut unsigned = manifest.unsigned();
        unsigned.bonus_evidence.push(BonusDto {
            instrument_id: unsigned.instruments[0].clone(),
            record_date: TradingDate::parse("2020-01-31").unwrap(),
            ex_date: TradingDate::parse("2020-01-31").unwrap(),
            split_factor: "1.00000000".into(),
            acquired_at: UtcTimestamp::parse_rfc3339("2026-08-19T00:00:00Z").unwrap(),
        });
        let manifest = reseal_manifest(unsigned);
        assert!(
            validate_historical_price_only_artifact_bytes(
                candidate.content_hash(),
                &artifact.bars_ndjson,
                &manifest
            )
            .is_err()
        );
    }

    #[test]
    fn per_line_and_row_caps_reject_without_large_allocations() {
        let (_, artifact) = artifact();
        let sessions = serde_json::from_slice::<Manifest>(&artifact.manifest_json)
            .unwrap()
            .sessions;
        let instruments = serde_json::from_slice::<Manifest>(&artifact.manifest_json)
            .unwrap()
            .instruments;
        let mut line = vec![b'x'; MAX_BAR_LINE_BYTES + 1];
        line.push(b'\n');
        assert!(validate_bars(&line, &sessions, &instruments).is_err());

        let mut rows = Vec::new();
        let mut date = TradingDate::parse("1970-01-01").unwrap();
        for index in 0..=17688 {
            serde_json::to_writer(&mut rows, &row("A.KRX", &date.to_string())).unwrap();
            rows.push(b'\n');
            if index != 17688 {
                date = date.next_day();
            }
        }
        assert!(validate_bars(&rows, &[], &["A.KRX".to_owned()]).is_err());
    }

    fn row(instrument_id: &str, date: &str) -> BarDto {
        BarDto {
            instrument_id: instrument_id.to_owned(),
            session_date: TradingDate::parse(date).unwrap(),
            raw_open: "10.00".to_owned(),
            raw_high: "11.00".to_owned(),
            raw_low: "9.00".to_owned(),
            raw_close: "10.50".to_owned(),
            raw_volume: 1,
            raw_trading_value: None,
            adjusted_open: "10.0000".to_owned(),
            adjusted_high: "11.0000".to_owned(),
            adjusted_low: "9.0000".to_owned(),
            adjusted_close: "10.5000".to_owned(),
        }
    }

    #[test]
    fn bars_reject_noncanonical_order_and_ohlc() {
        let mut bytes = Vec::new();
        for value in [row("A.KRX", "2020-01-02"), row("A.KRX", "2020-01-01")] {
            serde_json::to_writer(&mut bytes, &value).unwrap();
            bytes.push(b'\n');
        }
        assert!(validate_bars(&bytes, &[], &[]).is_err());
        let mut broken = row("A.KRX", "2020-01-01");
        broken.raw_low = "12.00".to_owned();
        let mut one = serde_json::to_vec(&broken).unwrap();
        one.push(b'\n');
        assert!(validate_bars(&one, &[], &[]).is_err());
    }

    #[test]
    fn bars_reject_oversized_line() {
        let mut bytes = vec![b'x'; MAX_BAR_LINE_BYTES + 1];
        bytes.push(b'\n');
        assert!(validate_bars(&bytes, &[], &[]).is_err());
    }
}

/// Fail-closed artifact errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoricalPriceOnlyArtifactError {
    #[error("historical price-only artifact requires Unix support")]
    UnsupportedPlatform,
    #[error("historical price-only artifact path is unsafe")]
    UnsafePath,
    #[error("historical price-only candidate does not match the fixed contract")]
    InvalidCandidate,
    #[error("historical price-only artifact bytes violate the fixed contract")]
    InvalidArtifact,
    #[error("historical price-only artifact filesystem error")]
    Io(#[source] std::io::Error),
    #[error("historical price-only artifact destination already exists")]
    Conflict {
        candidate_content_sha256: ContentHash,
    },
    #[error("historical price-only artifact atomic no-replace publication is unavailable")]
    UnsupportedAtomicNoReplace,
    #[error("historical price-only artifact commit state is indeterminate")]
    IndeterminateCommit,
    #[error("historical price-only artifact staging cleanup failed")]
    CleanupFailed,
    #[error("historical price-only artifact staging name allocation exhausted")]
    StagingNameExhausted,
}
