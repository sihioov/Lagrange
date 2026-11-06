//! Daily producer/admitter orchestration. One pinned local scope pair per cycle;
//! no provider transport, arbitrary registry or generic dataset readiness path.
use super::{
    daily_admission::{
        AdmissionPollFailure, CalendarProjectionOutcome, DailyAdmissionCycle,
        DailyAdmissionOutcome, DailyAdmissionStart, RecordedAdmissionOutcome,
        read_committed_daily_input, read_daily_admission_receipt,
    },
    daily_catalog::Catalog,
    daily_local_state::{DailyState, Pending, Stage},
    daily_runtime::{DailyProcessMode, DailyRuntimeError},
};
use chrono::{DateTime, Utc};
use domain::{CodeCommit, ContentHash, TradingDate, UtcTimestamp};
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, ResponseKind, SourceFileReference,
    approve_historical_price_only_artifact,
    daily_price_only::{
        DailyPricePinsV1Wire, DailySessionTypeV1,
        admission::{
            PinnedDailyPolicy, verify_historical_daily_candidate,
            write_verified_historical_admission,
        },
        artifact::{
            DailyAdmissionRoot, DailyArtifactError, DailyArtifactNameLimits, DailyCandidateRoot,
            DailySourceFrontierRoot, read_daily_candidate, write_daily_candidate,
            write_daily_continuation_candidate,
        },
        calendar::{
            VerifiedCalendarProjection, expected_calendar_session, project_calendar_candidates,
        },
        candidate::build_historical_daily_candidate,
        continuation::{
            SealedDailyParent, build_daily_continuation_candidate,
            verify_daily_continuation_candidate, write_verified_daily_continuation,
        },
        discovery::{SessionSourceRequest, discover_session_sources},
        frontier::{
            SourceFrontierRefreshError, prepare_continued_source_frontier,
            prepare_first_source_frontier, refresh_source_frontier, write_source_frontier,
        },
        history::{ProtectedHistoricalPrefix, reconstruct_historical_extension},
        observation::DailyBlockCode,
        replay::{
            DailyObservationPins, DailyReplayLimits, ReopenedDailyObservation,
            read_sealed_daily_chain, read_sealed_daily_observation,
        },
        source::{
            ApprovedRawScope, CommittedRawReader, CommittedRawSnapshot, ScanLimitValues, ScanLimits,
        },
        verify::{RawEvidenceSnapshots, resolve_original},
    },
};
use serde::Deserialize;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DAILY_PRODUCER_UID: u32 = 10002;
pub const DAILY_ADMITTER_UID: u32 = 10003;
pub const DAILY_READER_GROUP: u32 = 10001;

#[cfg(test)]
fn ingest_diagnostic(stage: &'static str, code: &'static str) {
    eprintln!(
        "R5_PAPER_INGEST_DIAGNOSTIC {}",
        serde_json::json!({"stage": stage, "code": code})
    );
}

#[cfg(test)]
fn admission_diagnostic_code(
    error: market_data::daily_price_only::admission::DailyAdmissionVerificationError,
) -> &'static str {
    use market_data::daily_price_only::admission::DailyAdmissionVerificationError as Error;
    match error {
        Error::Evidence(error) => error.code(),
        Error::Artifact(error) => match error {
            DailyArtifactError::UnsupportedPlatform => "DAILY_ARTIFACT_UNSUPPORTED_PLATFORM",
            DailyArtifactError::InvalidRoot => "DAILY_ARTIFACT_INVALID_ROOT",
            DailyArtifactError::UnsafePath => "DAILY_ARTIFACT_UNSAFE_PATH",
            DailyArtifactError::UnexpectedEntries => "DAILY_ARTIFACT_UNEXPECTED_ENTRIES",
            DailyArtifactError::SourceChanged => "DAILY_ARTIFACT_SOURCE_CHANGED",
            DailyArtifactError::InvalidArtifact => "DAILY_ARTIFACT_INVALID",
            DailyArtifactError::LimitExceeded => "DAILY_ARTIFACT_LIMIT",
            DailyArtifactError::Conflict => "DAILY_ARTIFACT_CONFLICT",
            DailyArtifactError::Io => "DAILY_ARTIFACT_IO",
            DailyArtifactError::CleanupFailed => "DAILY_ARTIFACT_CLEANUP_FAILED",
            DailyArtifactError::DurabilityUnknown => "DAILY_ARTIFACT_DURABILITY_UNKNOWN",
        },
        Error::InvalidPolicy => "DAILY_ADMISSION_INVALID_POLICY",
        Error::InvalidCandidate => "DAILY_ADMISSION_INVALID_CANDIDATE",
        Error::LimitExceeded => "DAILY_ADMISSION_LIMIT",
    }
}

#[cfg(test)]
macro_rules! diagnostic_input_blocked {
    ($stage:literal, $code:path) => {
        |error| {
            ingest_diagnostic($stage, $code(error));
            IngestError::InputBlocked
        }
    };
}

// The normal build expands to the original error mapping with no diagnostic.
#[cfg(not(test))]
macro_rules! diagnostic_input_blocked {
    ($stage:literal, $code:path) => {
        |_| IngestError::InputBlocked
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IngestError {
    #[error("DAILY_INGEST_CONFIGURATION")]
    Configuration,
    #[error("DAILY_INGEST_STATE")]
    State,
    #[error("DAILY_INGEST_INTERRUPTED")]
    Interrupted,
    #[error("DAILY_INGEST_INPUT_PENDING")]
    InputPending,
    #[error("DAILY_INGEST_INPUT_BLOCKED")]
    InputBlocked,
    #[error("DAILY_INGEST_ARTIFACT_INVALID")]
    InvalidArtifact,
    #[error("DAILY_INGEST_CONFLICT")]
    Conflict,
    #[error("DAILY_INGEST_DATABASE")]
    Database,
    #[error("DAILY_INGEST_COMMIT_UNKNOWN")]
    CommitUnknown,
}
impl IngestError {
    pub(super) fn artifact(_: DailyArtifactError) -> Self {
        Self::InvalidArtifact
    }
    fn database(error: DailyRuntimeError) -> Self {
        if error == DailyRuntimeError::CommitUnknown {
            Self::CommitUnknown
        } else {
            Self::Database
        }
    }
    fn poll_failure(self) -> AdmissionPollFailure {
        match self {
            Self::InputPending | Self::Interrupted => AdmissionPollFailure::InputPending,
            Self::InputBlocked | Self::InvalidArtifact | Self::Conflict => {
                AdmissionPollFailure::InputBlocked
            }
            _ => AdmissionPollFailure::IoFailed,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    Disabled,
    Busy,
    NotDue,
    Pending,
    CandidateReady,
    Admitted,
    Replay,
    Blocked(DailyBlockCode),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanBudget {
    max_manifest_bytes: usize,
    max_manifest_records: usize,
    max_batches: usize,
    max_files_per_batch: usize,
    max_files: usize,
    max_batch_metadata_bytes: usize,
    max_source_file_bytes: usize,
    max_retained_bytes: usize,
    max_open_descriptors: usize,
    max_data_root_bytes: usize,
    max_data_root_components: usize,
    max_metadata_json_depth: usize,
}
impl ScanBudget {
    fn values(&self) -> ScanLimitValues {
        ScanLimitValues {
            max_manifest_bytes: self.max_manifest_bytes,
            max_manifest_records: self.max_manifest_records,
            max_batches: self.max_batches,
            max_files_per_batch: self.max_files_per_batch,
            max_files: self.max_files,
            max_batch_metadata_bytes: self.max_batch_metadata_bytes,
            max_source_file_bytes: self.max_source_file_bytes,
            max_retained_bytes: self.max_retained_bytes,
            max_open_descriptors: self.max_open_descriptors,
            max_data_root_bytes: self.max_data_root_bytes,
            max_data_root_components: self.max_data_root_components,
            max_metadata_json_depth: self.max_metadata_json_depth,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    schema: String,
    owner: Uuid,
    policy_sha256: ContentHash,
    policy_path: PathBuf,
    historical_root: PathBuf,
    raw_root: PathBuf,
    candidate_root: PathBuf,
    admitted_root: PathBuf,
    frontier_root: PathBuf,
    producer_state_root: PathBuf,
    admitter_state_root: PathBuf,
    raw_owner_uid: u32,
    scan: ScanBudget,
    max_artifact_entries: u64,
    max_replay_admissions: u64,
    max_cycle_seconds: u64,
    code_commit: CodeCommit,
    entitlement_reference: String,
    entitlement_sha256: ContentHash,
    first_auto_session: TradingDate,
}
pub struct DailyIngestConfig {
    settings: Settings,
    policy: PinnedDailyPolicy,
    names: DailyArtifactNameLimits,
    replay: DailyReplayLimits,
}
impl DailyIngestConfig {
    pub fn from_pinned_file(path: &Path, expected: &ContentHash) -> Result<Self, IngestError> {
        let mut session = ConfigSession::legacy();
        let mut retained = TypedConfigRetention::default();
        Self::from_pinned_file_with(&mut session, &mut retained, path, expected)
            .map_err(ConfigFailure::legacy_source)
    }
    pub fn policy(&self) -> &PinnedDailyPolicy {
        &self.policy
    }
    pub fn admission_path(&self) -> &Path {
        &self.settings.admitted_root
    }
    pub fn historical_path(&self) -> &Path {
        &self.settings.historical_root
    }
    pub fn replay_limits(&self) -> DailyReplayLimits {
        self.replay
    }
}

fn canonical_absolute(path: &Path) -> bool {
    use std::path::Component;
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && path != Path::new("/")
        && !text.ends_with('/')
        && !text.contains("//")
        && !text.split('/').any(|part| part == "." || part == "..")
        && path
            .components()
            .skip(1)
            .all(|part| matches!(part, Component::Normal(_)))
}

/// The installed setting names the mounted Raw leaf. The evidence reader takes
/// its data-directory parent and opens the fixed `raw` child itself. Never pass
/// the mounted leaf as a data directory or silently accept a differently named
/// leaf: either would select a different source tree.
fn raw_data_root(raw_root: &Path) -> Result<&Path, IngestError> {
    if !canonical_absolute(raw_root) || raw_root.file_name().is_none_or(|name| name != "raw") {
        return Err(IngestError::Configuration);
    }
    raw_root.parent().ok_or(IngestError::Configuration)
}

fn validate_settings(
    settings: &Settings,
) -> Result<(DailyArtifactNameLimits, DailyReplayLimits), IngestError> {
    if settings.schema != "owner-beta-daily-process-v1"
        || settings.owner.is_nil()
        || settings.raw_owner_uid == u32::MAX
        || settings.code_commit.as_str().len() != 40
        || settings.entitlement_reference.trim().is_empty()
        || settings.entitlement_reference.trim() != settings.entitlement_reference
        || settings.entitlement_reference.len() > 4096
        || !(1..=3600).contains(&settings.max_cycle_seconds)
        || settings.first_auto_session
            <= TradingDate::parse("2026-08-28").map_err(|_| IngestError::Configuration)?
        || [
            &settings.policy_path,
            &settings.historical_root,
            &settings.raw_root,
            &settings.candidate_root,
            &settings.admitted_root,
            &settings.frontier_root,
            &settings.producer_state_root,
            &settings.admitter_state_root,
        ]
        .iter()
        .any(|p| !canonical_absolute(p))
    {
        return Err(IngestError::Configuration);
    }
    raw_data_root(&settings.raw_root)?;
    ScanLimits::new(settings.scan.values()).map_err(|_| IngestError::Configuration)?;
    let names = DailyArtifactNameLimits::new(settings.max_artifact_entries)
        .map_err(|_| IngestError::Configuration)?;
    let replay = DailyReplayLimits::new(settings.max_replay_admissions)
        .map_err(|_| IngestError::Configuration)?;

    let roots = [
        &settings.historical_root,
        &settings.raw_root,
        &settings.candidate_root,
        &settings.admitted_root,
        &settings.frontier_root,
        &settings.producer_state_root,
        &settings.admitter_state_root,
    ];
    for (index, left) in roots.iter().enumerate() {
        if roots
            .iter()
            .skip(index + 1)
            .any(|right| left.starts_with(right) || right.starts_with(left))
        {
            return Err(IngestError::Configuration);
        }
    }
    Ok((names, replay))
}

fn pinned_bytes(path: &Path, expected: &ContentHash, limit: u64) -> Result<Vec<u8>, IngestError> {
    let mut session = ConfigSession::legacy();
    let index = pinned_bytes_with(
        &mut session,
        path,
        expected,
        limit,
        ConfigReadStage::TypedConfig,
    )
    .map_err(ConfigFailure::legacy_source)?;
    Ok(session.take_legacy_bytes(index))
}
fn pinned_bytes_with(
    session: &mut ConfigSession<'_, '_>,
    path: &Path,
    expected: &ContentHash,
    limit: u64,
    stage: ConfigReadStage,
) -> Result<usize, ConfigFailure<IngestError>> {
    session.preflight()?;
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        session.with_file(
            stage,
            path,
            |_| IngestError::Configuration,
            |file, bytes, check| {
                check()?;
                let before = file.metadata().map_err(|_| IngestError::Configuration)?;
                if !canonical_absolute(path)
                    || !before.is_file()
                    || lower_config_len_invalid(before.len(), limit)
                    || before.nlink() != 1
                    || before.mode() & 0o022 != 0
                {
                    return Err(IngestError::Configuration.into());
                }
                check()?;
                Read::by_ref(file)
                    .take(limit + 1)
                    .read_to_end(bytes)
                    .map_err(|_| IngestError::Configuration)?;
                check()?;
                let after = file.metadata().map_err(|_| IngestError::Configuration)?;
                if bytes.len() as u64 != before.len()
                    || before.len() != after.len()
                    || before.mode() != after.mode()
                    || before.uid() != after.uid()
                    || before.gid() != after.gid()
                    || before.nlink() != after.nlink()
                    || before.ctime() != after.ctime()
                    || before.ctime_nsec() != after.ctime_nsec()
                    || before.mtime() != after.mtime()
                    || before.mtime_nsec() != after.mtime_nsec()
                    || ContentHash::from_bytes(bytes) != *expected
                {
                    return Err(IngestError::Configuration.into());
                }
                Ok(())
            },
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, expected, limit, stage);
        Err(session.non_linux_error(IngestError::Configuration))
    }
}

#[cfg(any(target_os = "linux", test))]
fn lower_config_len_invalid(len: u64, limit: u64) -> bool {
    len > limit
}

// BEGIN C9 config custody
use market_data::{
    checked_close::{
        Attempt as ConfigAttempt, Close as ConfigClose, Failure as ConfigCoreFailure,
        Gap as ConfigGap, Owner as ConfigOwner, Report as ConfigReport, Role as ConfigRole,
    },
    raw_custody::RawInvocation as ConfigInvocation,
};
type ConfigUnit = Result<(), ConfigCoreFailure<std::convert::Infallible>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigReadStage {
    AliasConfig,
    TypedConfig,
    Policy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFirstKind {
    Policy,
    Scope,
    Late,
}
enum ConfigFirst<E> {
    Policy(E),
    Scope(usize),
    Late,
}
pub(crate) struct ConfigFailure<E> {
    first: ConfigFirst<E>,
}
impl<E> From<E> for ConfigFailure<E> {
    fn from(source: E) -> Self {
        Self {
            first: ConfigFirst::Policy(source),
        }
    }
}
impl<E> ConfigFailure<E> {
    pub(crate) fn legacy_source(self) -> E {
        match self.first {
            ConfigFirst::Policy(source) => source,
            _ => unreachable!("private Legacy route has no checked scope"),
        }
    }
    pub(crate) fn policy(&self) -> Option<&E> {
        match &self.first {
            ConfigFirst::Policy(source) => Some(source),
            _ => None,
        }
    }
    pub(crate) fn kind(&self) -> ConfigFirstKind {
        match self.first {
            ConfigFirst::Policy(_) => ConfigFirstKind::Policy,
            ConfigFirst::Scope(_) => ConfigFirstKind::Scope,
            ConfigFirst::Late => ConfigFirstKind::Late,
        }
    }
    pub(crate) fn report<'a>(&self, completion: &'a ConfigCompletion) -> Option<&'a ConfigReport> {
        match self.first {
            ConfigFirst::Scope(index) => completion.checks[index].report(),
            _ => None,
        }
    }
}
impl<E> std::fmt::Debug for ConfigFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigFailure")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}
struct ConfigCheck {
    expected: ConfigUnit,
    captured: ConfigUnit,
    owner: Option<ConfigUnit>,
}
impl ConfigCheck {
    fn is_clean(&self) -> bool {
        self.expected.is_ok()
            && self.captured.is_ok()
            && self.owner.as_ref().is_none_or(Result::is_ok)
    }
    fn report(&self) -> Option<&ConfigReport> {
        self.expected
            .as_ref()
            .err()
            .or_else(|| self.captured.as_ref().err())
            .or_else(|| self.owner.as_ref().and_then(|r| r.as_ref().err()))
            .map(ConfigCoreFailure::report)
    }
}
#[derive(Clone, Copy)]
struct ConfigScope<'s, 'a> {
    invocation: &'s ConfigInvocation<'a>,
    expected: &'s ConfigAttempt,
    captured: &'s ConfigAttempt,
}
impl ConfigScope<'_, '_> {
    fn compare(&self, owner: Option<&ConfigAttempt>) -> ConfigCheck {
        let expected = self
            .invocation
            .settle_local_in(self.expected, Ok::<(), std::convert::Infallible>(()));
        let captured = self
            .invocation
            .settle_local_in(self.captured, Ok::<(), std::convert::Infallible>(()));
        let owner = owner.map(|owner| {
            self.invocation
                .settle_local_in(owner, Ok::<(), std::convert::Infallible>(()))
        });
        ConfigCheck {
            expected,
            captured,
            owner,
        }
    }
}
#[derive(Clone, Copy)]
enum ConfigMode<'s, 'a> {
    Legacy,
    Checked(ConfigScope<'s, 'a>),
}
struct ConfigReadRecord {
    stage: ConfigReadStage,
    bytes: Vec<u8>,
    original: ConfigFirstKind,
    original_ok: bool,
    native_birth: bool,
    local: Option<ConfigUnit>,
}
/// Actual local config evidence. No terminal result or resource constructor.
pub struct ConfigCompletion {
    checks: Vec<ConfigCheck>,
    reads: Vec<ConfigReadRecord>,
    local: Option<ConfigUnit>,
    failed: bool,
}
pub struct ConfigReadView<'a> {
    record: &'a ConfigReadRecord,
}
impl<'a> ConfigReadView<'a> {
    pub fn stage(&self) -> ConfigReadStage {
        self.record.stage
    }
    pub fn bytes_len(&self) -> usize {
        self.record.bytes.len()
    }
    pub fn original_ok(&self) -> bool {
        self.record.original_ok
    }
    pub fn original_kind(&self) -> Option<ConfigFirstKind> {
        (!self.record.original_ok).then_some(self.record.original)
    }
    pub fn native_birth(&self) -> bool {
        self.record.native_birth
    }
    pub fn local(&self) -> Option<&'a ConfigUnit> {
        self.record.local.as_ref()
    }
}
impl ConfigCompletion {
    pub fn read_count(&self) -> usize {
        self.reads.len()
    }
    pub fn read(&self, index: usize) -> Option<ConfigReadView<'_>> {
        self.reads
            .get(index)
            .map(|record| ConfigReadView { record })
    }
    pub fn local(&self) -> Option<&ConfigUnit> {
        self.local.as_ref()
    }
    pub fn expected_scope(&self, index: usize) -> Option<&ConfigUnit> {
        self.checks.get(index).map(|check| &check.expected)
    }
    pub fn captured_scope(&self, index: usize) -> Option<&ConfigUnit> {
        self.checks.get(index).map(|check| &check.captured)
    }
    pub fn owner_scope(&self, index: usize) -> Option<&ConfigUnit> {
        self.checks
            .get(index)
            .and_then(|check| check.owner.as_ref())
    }
    pub(crate) fn is_clean(&self) -> bool {
        !self.failed
            && self.checks.iter().all(ConfigCheck::is_clean)
            && self.local.as_ref().is_none_or(Result::is_ok)
            && self
                .reads
                .iter()
                .all(|read| read.local.as_ref().is_none_or(Result::is_ok))
    }
}
impl std::fmt::Debug for ConfigCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigCompletion")
            .field("reads", &self.reads.len())
            .field("checks", &self.checks.len())
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}
pub(crate) struct ConfigSession<'s, 'a> {
    mode: ConfigMode<'s, 'a>,
    checks: std::cell::RefCell<Vec<ConfigCheck>>,
    reads: Vec<ConfigReadRecord>,
    failed: bool,
    armed: bool,
}
fn retain_config_result<T, E, C>(
    result: Result<T, E>,
    retire: impl FnOnce() -> C,
) -> (Result<T, E>, C) {
    let completion = retire();
    (result, completion)
}
impl<'s, 'a> ConfigSession<'s, 'a> {
    pub(crate) fn legacy() -> Self {
        Self {
            mode: ConfigMode::Legacy,
            checks: Default::default(),
            reads: Vec::new(),
            failed: false,
            armed: false,
        }
    }
    pub(crate) fn borrowed_in(
        invocation: &'s ConfigInvocation<'a>,
        expected: &'s ConfigAttempt,
        captured: &'s ConfigAttempt,
    ) -> Self {
        Self {
            mode: ConfigMode::Checked(ConfigScope {
                invocation,
                expected,
                captured,
            }),
            checks: Default::default(),
            reads: Vec::new(),
            failed: false,
            armed: true,
        }
    }
    pub(crate) fn is_legacy(&self) -> bool {
        matches!(self.mode, ConfigMode::Legacy)
    }
    fn compare<E>(&self, owner: Option<&ConfigAttempt>) -> Result<(), ConfigFailure<E>> {
        if let ConfigMode::Checked(scope) = self.mode {
            let check = scope.compare(owner);
            let clean = check.is_clean();
            let mut checks = self.checks.borrow_mut();
            let index = checks.len();
            checks.push(check);
            if !clean {
                return Err(ConfigFailure {
                    first: ConfigFirst::Scope(index),
                });
            }
        }
        Ok(())
    }
    pub(crate) fn preflight<E>(&self) -> Result<(), ConfigFailure<E>> {
        self.compare(None)?;
        if self.failed {
            if let ConfigMode::Checked(scope) = self.mode {
                scope.captured.gap(ConfigGap::Reuse);
            }
            return Err(ConfigFailure {
                first: ConfigFirst::Late,
            });
        }
        Ok(())
    }
    pub(crate) fn policy_error<E>(&mut self, source: E) -> ConfigFailure<E> {
        let failure = ConfigFailure::from(source);
        self.mark_failed();
        failure
    }
    fn mark_failed(&mut self) {
        self.failed = true;
        if let ConfigMode::Checked(scope) = self.mode
            && scope.captured.check()
        {
            scope.captured.gap(ConfigGap::Abandoned);
        }
    }
    pub(crate) fn bytes(&self, index: usize) -> &[u8] {
        &self.reads[index].bytes
    }
    pub(crate) fn legacy_bytes(&mut self, stage: ConfigReadStage, bytes: Vec<u8>) -> usize {
        assert!(self.is_legacy(), "private untracked byte transport");
        self.store(stage, bytes, true, ConfigFirstKind::Policy, false, None)
    }
    pub(crate) fn take_legacy_bytes(&mut self, index: usize) -> Vec<u8> {
        assert!(self.is_legacy(), "private Legacy projection");
        std::mem::take(&mut self.reads[index].bytes)
    }
    fn store(
        &mut self,
        stage: ConfigReadStage,
        bytes: Vec<u8>,
        original_ok: bool,
        original: ConfigFirstKind,
        native_birth: bool,
        local: Option<ConfigUnit>,
    ) -> usize {
        let index = self.reads.len();
        self.reads.push(ConfigReadRecord {
            stage,
            bytes,
            original,
            original_ok,
            native_birth,
            local,
        });
        index
    }
    pub(crate) fn retire<R: ConfigClose, E>(
        &mut self,
        owner: ConfigOwner<R>,
        outcome: Result<(), ConfigFailure<E>>,
    ) -> Result<(), ConfigFailure<E>> {
        assert!(!self.is_legacy(), "private checked owner retirement");
        let _before = self.compare::<E>(Some(owner.attempt()));
        let (outcome, ()) = retain_config_result(outcome, || {
            let _ = owner.finish();
        });
        let after = self.compare::<E>(None);
        match outcome {
            Err(source) => {
                self.mark_failed();
                Err(source)
            }
            Ok(value) if after.is_ok() => Ok(value),
            Ok(_) => {
                self.failed = true;
                Err(ConfigFailure {
                    first: ConfigFirst::Late,
                })
            }
        }
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn with_file<E>(
        &mut self,
        stage: ConfigReadStage,
        path: &Path,
        open_error: impl FnOnce(std::io::Error) -> E,
        body: impl FnOnce(
            &mut std::fs::File,
            &mut Vec<u8>,
            &mut dyn FnMut() -> Result<(), ConfigFailure<E>>,
        ) -> Result<(), ConfigFailure<E>>,
    ) -> Result<usize, ConfigFailure<E>> {
        use std::os::unix::fs::OpenOptionsExt;
        self.preflight()?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path);
        let file = match file {
            Ok(file) => file,
            Err(error) => {
                let failure = self.policy_error(open_error(error));
                let local =
                    match self.mode {
                        ConfigMode::Legacy => None,
                        ConfigMode::Checked(scope) => Some(scope.invocation.settle_local_in(
                            scope.captured,
                            Ok::<(), std::convert::Infallible>(()),
                        )),
                    };
                self.store(stage, Vec::new(), false, failure.kind(), false, local);
                return Err(failure);
            }
        };
        let mut bytes = Vec::new();
        let (result, original_ok, original) = match self.mode {
            ConfigMode::Legacy => {
                let mut file = file;
                let outcome = body(&mut file, &mut bytes, &mut || Ok(()));
                let kind = outcome
                    .as_ref()
                    .err()
                    .map(ConfigFailure::kind)
                    .unwrap_or(ConfigFirstKind::Policy);
                let ok = outcome.is_ok();
                let (outcome, ()) = retain_config_result(outcome, || drop(file));
                (outcome, ok, kind)
            }
            ConfigMode::Checked(scope) => {
                let mut owner = scope.captured.adopt(file, ConfigRole::Source);
                let actual_owner = owner.attempt().clone();
                let outcome = body(owner.get_mut(), &mut bytes, &mut || {
                    self.compare(Some(&actual_owner))
                });
                let kind = outcome
                    .as_ref()
                    .err()
                    .map(ConfigFailure::kind)
                    .unwrap_or(ConfigFirstKind::Policy);
                let ok = outcome.is_ok();
                (self.retire(owner, outcome), ok, kind)
            }
        };
        if result.is_err() {
            self.mark_failed();
        }
        let local = match self.mode {
            ConfigMode::Legacy => None,
            ConfigMode::Checked(scope) => Some(
                scope
                    .invocation
                    .settle_local_in(scope.captured, Ok::<(), std::convert::Infallible>(())),
            ),
        };
        self.complete_read(stage, bytes, original_ok, original, result, local)
    }
    #[cfg(target_os = "linux")]
    fn complete_read<E>(
        &mut self,
        stage: ConfigReadStage,
        bytes: Vec<u8>,
        original_ok: bool,
        original: ConfigFirstKind,
        result: Result<(), ConfigFailure<E>>,
        local: Option<ConfigUnit>,
    ) -> Result<usize, ConfigFailure<E>> {
        let local_clean = local.as_ref().is_none_or(Result::is_ok);
        let index = self.store(stage, bytes, original_ok, original, true, local);
        match result {
            Err(source) => Err(source),
            Ok(()) if local_clean => Ok(index),
            Ok(()) => {
                self.mark_failed();
                Err(ConfigFailure {
                    first: ConfigFirst::Late,
                })
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn non_linux_error<E>(&mut self, source: E) -> ConfigFailure<E> {
        if let ConfigMode::Checked(scope) = self.mode {
            scope.captured.gap(ConfigGap::NonLinux);
        }
        self.policy_error(source)
    }
    pub(crate) fn finish_local(mut self) -> ConfigCompletion {
        let local = match self.mode {
            ConfigMode::Legacy => None,
            ConfigMode::Checked(scope) => Some(
                scope
                    .invocation
                    .settle_local_in(scope.captured, Ok::<(), std::convert::Infallible>(())),
            ),
        };
        self.armed = false;
        ConfigCompletion {
            checks: self.checks.take(),
            reads: std::mem::take(&mut self.reads),
            local,
            failed: self.failed,
        }
    }
}
impl Drop for ConfigSession<'_, '_> {
    fn drop(&mut self) {
        if self.armed
            && let ConfigMode::Checked(scope) = self.mode
        {
            scope.captured.gap(ConfigGap::Abandoned);
        }
    }
}
#[derive(Default)]
pub(crate) struct TypedConfigRetention {
    settings: Option<Settings>,
    policy: Option<PinnedDailyPolicy>,
}
impl TypedConfigRetention {
    pub(crate) fn partial_count(&self) -> usize {
        usize::from(self.settings.is_some()) + usize::from(self.policy.is_some())
    }
}
fn typed_config_flow<S, N, T, E>(
    state: &mut S,
    read_settings: impl FnOnce(&mut S) -> Result<(), E>,
    validate: impl FnOnce(&mut S) -> Result<N, E>,
    read_policy: impl FnOnce(&mut S) -> Result<(), E>,
    build: impl FnOnce(&mut S, N) -> T,
) -> Result<T, E> {
    read_settings(state)?;
    let validated = validate(state)?;
    read_policy(state)?;
    Ok(build(state, validated))
}
impl DailyIngestConfig {
    pub(crate) fn from_pinned_file_with(
        session: &mut ConfigSession<'_, '_>,
        retained: &mut TypedConfigRetention,
        path: &Path,
        expected: &ContentHash,
    ) -> Result<Self, ConfigFailure<IngestError>> {
        let mut state = (session, retained);
        typed_config_flow(
            &mut state,
            |(session, retained)| {
                let index = lower_config_bytes(
                    session,
                    path,
                    expected,
                    16 * 1024,
                    ConfigReadStage::TypedConfig,
                )?;
                retained.settings = Some(
                    serde_json::from_slice(session.bytes(index))
                        .map_err(|_| session.policy_error(IngestError::Configuration))?,
                );
                Ok(())
            },
            |(session, retained)| {
                validate_settings(retained.settings.as_ref().expect("real parsed settings"))
                    .map_err(|source| session.policy_error(source))
            },
            |(session, retained)| {
                let settings = retained.settings.as_ref().expect("real validated settings");
                let index = lower_config_bytes(
                    session,
                    &settings.policy_path,
                    &settings.policy_sha256,
                    256 * 1024,
                    ConfigReadStage::Policy,
                )?;
                retained.policy = Some(
                    PinnedDailyPolicy::from_pinned_bytes(
                        session.bytes(index),
                        &settings.policy_sha256,
                    )
                    .map_err(|_| session.policy_error(IngestError::Configuration))?,
                );
                Ok(())
            },
            |(_, retained), (names, replay)| Self {
                settings: retained.settings.take().expect("real settings"),
                policy: retained.policy.take().expect("real policy"),
                names,
                replay,
            },
        )
    }
}
fn lower_config_bytes(
    session: &mut ConfigSession<'_, '_>,
    path: &Path,
    expected: &ContentHash,
    limit: u64,
    stage: ConfigReadStage,
) -> Result<usize, ConfigFailure<IngestError>> {
    if session.is_legacy() {
        let bytes = pinned_bytes(path, expected, limit)?;
        Ok(session.legacy_bytes(stage, bytes))
    } else {
        pinned_bytes_with(session, path, expected, limit, stage)
    }
}
// END C9 config custody

struct CycleClock<'a> {
    started: Instant,
    budget: Duration,
    stop: &'a AtomicBool,
}
impl<'a> CycleClock<'a> {
    fn new(config: &DailyIngestConfig, stop: &'a AtomicBool) -> Self {
        Self {
            started: Instant::now(),
            budget: Duration::from_secs(config.settings.max_cycle_seconds),
            stop,
        }
    }
    fn keep_reading(&self) -> bool {
        !self.stop.load(Ordering::Acquire) && self.started.elapsed() <= self.budget
    }
    fn check(&self) -> Result<(), IngestError> {
        if self.keep_reading() {
            Ok(())
        } else {
            Err(IngestError::Interrupted)
        }
    }
}
struct Roots {
    candidates: DailyCandidateRoot,
    admissions: DailyAdmissionRoot,
    frontiers: DailySourceFrontierRoot,
}
impl Roots {
    fn open(config: &DailyIngestConfig) -> Result<Self, IngestError> {
        let s = &config.settings;
        Ok(Self {
            candidates: DailyCandidateRoot::open(
                &s.candidate_root,
                DAILY_PRODUCER_UID,
                DAILY_READER_GROUP,
            )
            .map_err(IngestError::artifact)?,
            admissions: DailyAdmissionRoot::open(
                &s.admitted_root,
                DAILY_ADMITTER_UID,
                DAILY_READER_GROUP,
            )
            .map_err(IngestError::artifact)?,
            frontiers: DailySourceFrontierRoot::open(
                &s.frontier_root,
                DAILY_ADMITTER_UID,
                DAILY_READER_GROUP,
            )
            .map_err(IngestError::artifact)?,
        })
    }
}
fn capture_raw(
    config: &DailyIngestConfig,
    clock: &CycleClock<'_>,
) -> Result<[CommittedRawSnapshot; 2], IngestError> {
    let reader = CommittedRawReader::new(raw_data_root(&config.settings.raw_root)?);
    let capture = |provider| {
        clock.check()?;
        let scope = ApprovedRawScope::new(provider, "kr", config.settings.raw_owner_uid)
            .map_err(|_| IngestError::Configuration)?;
        let limits = ScanLimits::new(config.settings.scan.values())
            .map_err(|_| IngestError::Configuration)?;
        reader
            .snapshot(scope, limits)
            .map_err(diagnostic_input_blocked!(
                "CAPTURE_RAW",
                market_data::daily_price_only::source::DailyEvidenceError::code
            ))
    };
    let kis = capture("kis")?;
    let calendar = capture("kis-calendar")?;
    clock.check()?;
    Ok([kis, calendar])
}
fn calendars<'a>(
    captured: &'a [CommittedRawSnapshot; 2],
    snapshots: &RawEvidenceSnapshots<'a>,
    config: &DailyIngestConfig,
    clock: &CycleClock<'_>,
) -> Result<Vec<VerifiedCalendarProjection<'a>>, IngestError> {
    let mut values = Vec::new();
    let mut bytes = 0u64;
    let mut count = 0usize;
    for scope in captured {
        for batch in scope.batches() {
            if !snapshots.matches_entitlement(batch) {
                continue;
            }
            for file in batch
                .files()
                .filter(|file| file.kind() == ResponseKind::Calendar)
            {
                clock.check()?;
                count = count.checked_add(1).ok_or(IngestError::InputBlocked)?;
                if count > config.settings.scan.max_files {
                    return Err(IngestError::InputBlocked);
                }
                let reference = SourceFileReference {
                    provider: scope.scope().provider().into(),
                    market: scope.scope().market().into(),
                    batch_id: batch.batch_id(),
                    file_name: file.file_name().into(),
                    content_hash: file.content_hash().clone(),
                    retrieved_at: batch.retrieved_at(),
                };
                let original =
                    resolve_original(snapshots, &reference).map_err(diagnostic_input_blocked!(
                        "CALENDAR_RESOLVE",
                        market_data::daily_price_only::verify::DailyVerificationError::code
                    ))?;
                bytes = bytes
                    .checked_add(original.original_bytes().len() as u64)
                    .ok_or(IngestError::InputBlocked)?;
                if bytes > config.settings.scan.max_retained_bytes as u64 {
                    return Err(IngestError::InputBlocked);
                }
                values.push(original);
            }
        }
    }
    clock.check()?;
    let projections =
        project_calendar_candidates(&values, config.settings.owner, config.policy.sha256())
            .map_err(diagnostic_input_blocked!(
                "CALENDAR_PROJECT",
                market_data::daily_price_only::verify::DailyVerificationError::code
            ))?;
    clock.check()?;
    Ok(projections)
}
fn reopen<'a>(
    roots: &Roots,
    config: &'a DailyIngestConfig,
    base: &'a ApprovedHistoricalPriceOnlyArtifact,
    pins: &DailyPricePinsV1Wire,
    frontier: &ContentHash,
    clock: &CycleClock<'_>,
) -> Result<ReopenedDailyObservation<'a>, IngestError> {
    read_sealed_daily_observation(
        &roots.admissions,
        &roots.frontiers,
        DailyObservationPins {
            admission: pins,
            source_frontier: frontier,
            owner: config.settings.owner,
        },
        base,
        &config.policy,
        config.replay,
        || clock.keep_reading(),
    )
    .map_err(|_| IngestError::InvalidArtifact)
}

pub fn produce_daily_once(
    config: &DailyIngestConfig,
    mode: DailyProcessMode,
    now: DateTime<Utc>,
    stop: &AtomicBool,
) -> Result<IngestOutcome, IngestError> {
    if mode == DailyProcessMode::Disabled {
        return Ok(IngestOutcome::Disabled);
    }
    let clock = CycleClock::new(config, stop);
    clock.check()?;
    let Some(mut state) = DailyState::acquire(
        &config.settings.producer_state_root,
        DAILY_PRODUCER_UID,
        DAILY_PRODUCER_UID,
        Stage::Producer,
        config.settings.owner,
        config.policy.sha256(),
    )
    .map_err(|_| IngestError::State)?
    else {
        return Ok(IngestOutcome::Busy);
    };
    if !state.begin(now).map_err(|_| IngestError::State)? {
        return Ok(IngestOutcome::NotDue);
    }
    let roots = Roots::open(config)?;
    let base = approve_historical_price_only_artifact(&config.settings.historical_root)
        .map_err(|_| IngestError::InvalidArtifact)?;
    let prefix = ProtectedHistoricalPrefix::from_approved_v3(&base, config.policy.wire())
        .map_err(|_| IngestError::InvalidArtifact)?;
    let catalog = Catalog::read(
        &roots.admissions,
        &roots.frontiers,
        &roots.candidates,
        config.names,
        config.settings.owner,
        config.policy.sha256(),
        || clock.keep_reading(),
    )?;
    if catalog.incomplete().is_some() {
        // The admitter owns recovery of an admission whose frontier publication
        // was interrupted. A producer must not invent that missing seal.
        return Ok(IngestOutcome::Pending);
    }
    let current = catalog
        .tip()
        .map(|(pins, frontier)| reopen(&roots, config, &base, pins, frontier, &clock))
        .transpose()?;
    let captured = capture_raw(config, &clock)?;
    let snapshots = RawEvidenceSnapshots::new(
        &[&captured[0], &captured[1]],
        &config.settings.entitlement_reference,
        UtcTimestamp::from_datetime(now),
    )
    .map_err(diagnostic_input_blocked!(
        "SNAPSHOTS",
        market_data::daily_price_only::verify::DailyVerificationError::code
    ))?;
    let parent = current.as_ref().map(ReopenedDailyObservation::as_parent);
    let _refreshed = match (&current, &parent) {
        (Some(current), Some(parent)) => Some(
            refresh_source_frontier(parent, current.frontier(), &snapshots).map_err(|error| {
                #[cfg(test)]
                match &error {
                    SourceFrontierRefreshError::Input(error) => {
                        ingest_diagnostic("REFRESH", admission_diagnostic_code(*error));
                    }
                    SourceFrontierRefreshError::Blocked(event) => {
                        ingest_diagnostic("REFRESH", event.code().code());
                    }
                }
                match error {
                    SourceFrontierRefreshError::Input(_) => IngestError::InputBlocked,
                    SourceFrontierRefreshError::Blocked(_) => IngestError::Conflict,
                }
            })?,
        ),
        (None, None) => None,
        _ => return Err(IngestError::InvalidArtifact),
    };
    if let Some(Pending::Candidate {
        content_sha256,
        manifest_sha256,
        ..
    }) = state.pending()
        && catalog.has_candidate(content_sha256, manifest_sha256)
    {
        // The complete physical ancestry has been reopened and freshly checked.
        // This clears a restart hint only, never confers SQL admission authority.
        state.clear_pending().map_err(|_| IngestError::State)?;
    }
    let projections = calendars(&captured, &snapshots, config, &clock)?;
    let references: Vec<_> = projections.iter().collect();
    let expected = expected_calendar_session(
        &references,
        prefix
            .frontier()
            .checked_add_days(1)
            .map_err(|_| IngestError::InputBlocked)?,
        UtcTimestamp::from_datetime(now),
    )
    .map_err(|_| IngestError::InputPending)?;
    let previous = parent
        .as_ref()
        .map_or(prefix.frontier(), |parent| parent.manifest().range_end);
    let next = expected
        .proof()
        .days()
        .iter()
        .find(|day| day.date() > previous && day.session_type() == DailySessionTypeV1::Trading)
        .map(|day| day.date());
    let Some(next) = next else {
        if state.pending().is_some() || catalog.unadmitted_candidate().is_some() {
            return Err(IngestError::Conflict);
        }
        return Ok(IngestOutcome::Replay);
    };
    let parent_hash = parent
        .as_ref()
        .map(|parent| parent.pins().admission_sha256.clone());
    let existing = catalog
        .unadmitted_candidate()
        .map(|hash| read_daily_candidate(&roots.candidates, hash).map_err(IngestError::artifact))
        .transpose()?;
    let commit = match state.pending() {
        Some(Pending::Candidate {
            content_sha256,
            parent_admission_sha256,
            session,
            producer_commit,
            ..
        }) => {
            if parent_admission_sha256 != &parent_hash
                || *session != next
                || catalog
                    .unadmitted_candidate()
                    .is_some_and(|hash| hash != content_sha256)
            {
                return Err(IngestError::Conflict);
            }
            producer_commit.clone()
        }
        Some(Pending::Receipt { .. }) => return Err(IngestError::State),
        None => match &existing {
            Some(value) => {
                if value.manifest().parent_admission_sha256 != parent_hash
                    || value.manifest().range_end != next
                    || value.manifest().policy_sha256 != *config.policy.sha256()
                {
                    return Err(IngestError::Conflict);
                }
                value.manifest().code_commit.clone()
            }
            None => config.settings.code_commit.clone(),
        },
    };
    let sources = discover_session_sources(
        &snapshots,
        &SessionSourceRequest {
            owner: config.settings.owner,
            policy_sha256: config.policy.sha256().clone(),
            previous_frontier: previous,
            session_date: next,
        },
    )
    .map_err(diagnostic_input_blocked!(
        "DISCOVER",
        market_data::daily_price_only::verify::DailyVerificationError::code
    ))?;
    clock.check()?;
    match current.as_ref() {
        None => {
            let extension = reconstruct_historical_extension(&prefix, &sources, &snapshots)
                .map_err(diagnostic_input_blocked!(
                    "HISTORICAL_EXTENSION",
                    market_data::daily_price_only::verify::DailyVerificationError::code
                ))?;
            let candidate = build_historical_daily_candidate(&extension, commit.clone()).map_err(
                diagnostic_input_blocked!(
                    "HISTORICAL_BUILD",
                    market_data::daily_price_only::verify::DailyVerificationError::code
                ),
            )?;
            if let Some(existing) = &existing {
                existing
                    .matches_reconstruction(&candidate)
                    .map_err(IngestError::artifact)?;
            }
            let planned = Pending::Candidate {
                content_sha256: candidate.content_sha256().clone(),
                manifest_sha256: candidate.manifest_sha256().clone(),
                parent_admission_sha256: parent_hash,
                session: next,
                producer_commit: commit,
            };
            catalog.revalidate()?;
            clock.check()?;
            state.set_pending(planned).map_err(|_| IngestError::State)?;
            clock.check()?;
            write_daily_candidate(&roots.candidates, &candidate)
                .map_err(IngestError::artifact)?
                .revalidate()
                .map_err(IngestError::artifact)?;
        }
        Some(current) => {
            let candidate = build_daily_continuation_candidate(
                current.as_parent(),
                &sources,
                &snapshots,
                commit.clone(),
            )
            .map_err(diagnostic_input_blocked!(
                "CONTINUATION",
                admission_diagnostic_code
            ))?;
            if let Some(existing) = &existing {
                existing
                    .matches_continuation_reconstruction(&candidate)
                    .map_err(IngestError::artifact)?;
            }
            let planned = Pending::Candidate {
                content_sha256: candidate.content_sha256().clone(),
                manifest_sha256: candidate.manifest_sha256().clone(),
                parent_admission_sha256: parent_hash,
                session: next,
                producer_commit: commit,
            };
            catalog.revalidate()?;
            clock.check()?;
            state.set_pending(planned).map_err(|_| IngestError::State)?;
            clock.check()?;
            write_daily_continuation_candidate(&roots.candidates, &candidate)
                .map_err(IngestError::artifact)?
                .revalidate()
                .map_err(IngestError::artifact)?;
        }
    }
    clock.check()?;
    Ok(IngestOutcome::CandidateReady)
}

pub async fn admit_daily_once(
    pool: &PgPool,
    config: &DailyIngestConfig,
    mode: DailyProcessMode,
    now: DateTime<Utc>,
    stop: &AtomicBool,
) -> Result<IngestOutcome, IngestError> {
    if mode == DailyProcessMode::Disabled {
        return Ok(IngestOutcome::Disabled);
    }
    let clock = CycleClock::new(config, stop);
    clock.check()?;
    let Some(mut state) = DailyState::acquire(
        &config.settings.admitter_state_root,
        DAILY_ADMITTER_UID,
        DAILY_ADMITTER_UID,
        Stage::Admitter,
        config.settings.owner,
        config.policy.sha256(),
    )
    .map_err(|_| IngestError::State)?
    else {
        return Ok(IngestOutcome::Busy);
    };
    if !state.begin(now).map_err(|_| IngestError::State)? {
        return Ok(IngestOutcome::NotDue);
    }
    let started = DailyAdmissionCycle::start(pool, mode)
        .await
        .map_err(IngestError::database)?;
    let mut cycle = Some(match started {
        DailyAdmissionStart::Disabled => return Ok(IngestOutcome::Disabled),
        DailyAdmissionStart::Busy => return Ok(IngestOutcome::Busy),
        DailyAdmissionStart::NotDue => return Ok(IngestOutcome::NotDue),
        DailyAdmissionStart::Started(cycle) => cycle,
    });
    let mut tracks_current_receipt = false;
    let result = async {
        let context = cycle
            .as_mut()
            .ok_or(IngestError::Database)?
            .context()
            .await
            .map_err(IngestError::database)?;
        if context.owner() != config.settings.owner
            || context.policy_sha256() != config.policy.sha256()
            || context.entitlement_reference() != config.settings.entitlement_reference
            || context.entitlement_sha256() != &config.settings.entitlement_sha256
            || context.first_auto_session() != config.settings.first_auto_session.as_naive_date()
        {
            return Err(IngestError::Configuration);
        }
        if let Some(pending) = state.pending() {
            let Pending::Receipt { receipt_id } = pending else {
                return Err(IngestError::State);
            };
            let old = read_daily_admission_receipt(pool, *receipt_id)
                .await
                .map_err(IngestError::database)?
                .ok_or(IngestError::State)?;
            if old.owner() != context.owner()
                || old.policy_sha256() != context.policy_sha256()
                || old.matches_current_cycle()
            {
                return Err(IngestError::State);
            }
            // A STARTED receipt remains incomplete in SQL. Holding the new
            // exclusive cycle/context proves it is superseded, not successful.
            // Completed receipts are history, never a shortcut around the tip.
            match old.outcome() {
                RecordedAdmissionOutcome::Started
                | RecordedAdmissionOutcome::Verified { .. }
                | RecordedAdmissionOutcome::Failed { .. } => (),
            }
        }
        let receipt = cycle.as_ref().ok_or(IngestError::Database)?.receipt_id();
        state
            .replace_receipt(receipt)
            .map_err(|_| IngestError::State)?;
        tracks_current_receipt = true;
        if context.source_blocked() {
            return Err(IngestError::InputBlocked);
        }
        let committed = match context.latest_admission_sha256() {
            Some(hash) => Some(
                read_committed_daily_input(pool, hash)
                    .await
                    .map_err(IngestError::database)?
                    .ok_or(IngestError::InvalidArtifact)?,
            ),
            None => None,
        };
        if committed
            .as_ref()
            .is_some_and(|input| !input.unblocked_at_read())
        {
            return Err(IngestError::InputBlocked);
        }
        clock.check()?;
        let roots = Roots::open(config)?;
        let base = approve_historical_price_only_artifact(&config.settings.historical_root)
            .map_err(|_| IngestError::InvalidArtifact)?;
        ProtectedHistoricalPrefix::from_approved_v3(&base, config.policy.wire())
            .map_err(|_| IngestError::InvalidArtifact)?;
        let catalog = Catalog::read(
            &roots.admissions,
            &roots.frontiers,
            &roots.candidates,
            config.names,
            context.owner(),
            config.policy.sha256(),
            || clock.keep_reading(),
        )?;
        let current = match &committed {
            None => None,
            Some(input) => {
                let (pins, frontier) = catalog.current_pair(&input.pins().admission_sha256)?;
                if pins != input.pins() || frontier != input.source_frontier_sha256() {
                    return Err(IngestError::InvalidArtifact);
                }
                let opened = reopen(&roots, config, &base, pins, frontier, &clock)?;
                if opened.as_parent().manifest().range_end.as_naive_date() != input.range_end()
                    || opened.as_parent().manifest().range_start.as_naive_date()
                        != input.range_start()
                    || opened.as_parent().manifest().sessions.len()
                        != input.session_count() as usize
                {
                    return Err(IngestError::InvalidArtifact);
                }
                Some(opened)
            }
        };
        let captured = capture_raw(config, &clock)?;
        let snapshots = RawEvidenceSnapshots::new(
            &[&captured[0], &captured[1]],
            context.entitlement_reference(),
            UtcTimestamp::from_datetime(context.observed_at()),
        )
        .map_err(|_| IngestError::InputBlocked)?;
        let parent = current.as_ref().map(ReopenedDailyObservation::as_parent);
        let refreshed = match (&parent, &current) {
            (Some(parent), Some(current)) => {
                match refresh_source_frontier(parent, current.frontier(), &snapshots) {
                    Ok(value) => Some(value),
                    Err(SourceFrontierRefreshError::Blocked(event)) => {
                        clock.check()?;
                        let code = cycle
                            .take()
                            .ok_or(IngestError::Database)?
                            .finish_source_event(&event)
                            .await
                            .map_err(IngestError::database)?;
                        return Ok(IngestOutcome::Blocked(code));
                    }
                    Err(SourceFrontierRefreshError::Input(_)) => {
                        return Err(IngestError::InputBlocked);
                    }
                }
            }
            (None, None) => None,
            _ => return Err(IngestError::InvalidArtifact),
        };
        let projections = calendars(&captured, &snapshots, config, &clock)?;
        for projection in &projections {
            clock.check()?;
            if let CalendarProjectionOutcome::Blocked(code) = cycle
                .as_mut()
                .ok_or(IngestError::Database)?
                .project_calendar(projection)
                .await
                .map_err(IngestError::database)?
            {
                cycle
                    .take()
                    .ok_or(IngestError::Database)?
                    .finish_blocked()
                    .await
                    .map_err(IngestError::database)?;
                return Ok(IngestOutcome::Blocked(code));
            }
        }
        let orphan = catalog.next_after(context.latest_admission_sha256())?;
        let (content, verifier, expected_orphan) = if let Some((pins, frontier)) = orphan {
            let opened = read_sealed_daily_chain(
                &roots.admissions,
                pins,
                &base,
                &config.policy,
                config.replay,
                || clock.keep_reading(),
            )
            .map_err(|_| IngestError::InvalidArtifact)?;
            let orphan_parent = opened.as_parent();
            if orphan_parent.manifest().parent_admission_sha256.as_ref()
                != context.latest_admission_sha256()
            {
                return Err(IngestError::Conflict);
            }
            if let Some(frontier) = frontier {
                let paired = reopen(&roots, config, &base, pins, frontier, &clock)?;
                match refresh_source_frontier(&paired.as_parent(), paired.frontier(), &snapshots) {
                    Ok(_) => (),
                    Err(SourceFrontierRefreshError::Blocked(event)) => {
                        clock.check()?;
                        let code = cycle
                            .take()
                            .ok_or(IngestError::Database)?
                            .finish_source_event(&event)
                            .await
                            .map_err(IngestError::database)?;
                        return Ok(IngestOutcome::Blocked(code));
                    }
                    Err(SourceFrontierRefreshError::Input(_)) => {
                        return Err(IngestError::InputBlocked);
                    }
                }
            }
            let verifier = match orphan_parent {
                SealedDailyParent::First(value) => value.admission().verifier_code_commit.clone(),
                SealedDailyParent::Continued(value) => {
                    value.admission().verifier_code_commit.clone()
                }
            };
            (
                pins.content_sha256.clone(),
                verifier,
                Some((pins.clone(), frontier.cloned())),
            )
        } else if let Some(content) = catalog.unadmitted_candidate() {
            (content.clone(), config.settings.code_commit.clone(), None)
        } else if let Some(current) = &current {
            clock.check()?;
            catalog.revalidate()?;
            let outcome = cycle
                .take()
                .ok_or(IngestError::Database)?
                .admit(&current.as_parent(), current.frontier())
                .await
                .map_err(IngestError::database)?;
            return Ok(admission_outcome(outcome));
        } else {
            return Err(IngestError::InputPending);
        };
        clock.check()?;
        let candidate =
            read_daily_candidate(&roots.candidates, &content).map_err(IngestError::artifact)?;
        if candidate.manifest().parent_admission_sha256.as_ref()
            != context.latest_admission_sha256()
        {
            return Err(IngestError::Conflict);
        }
        let outcome = match &refreshed {
            None => {
                let verified = verify_historical_daily_candidate(
                    &candidate,
                    &base,
                    &snapshots,
                    &config.policy,
                    context.owner(),
                    verifier,
                )
                .map_err(|_| IngestError::InputBlocked)?;
                let prepared = prepare_first_source_frontier(&verified)
                    .map_err(|_| IngestError::InputBlocked)?;
                check_orphan(expected_orphan.as_ref(), verified.pins(), prepared.sha256())?;
                catalog.revalidate()?;
                clock.check()?;
                let sealed = write_verified_historical_admission(&roots.admissions, &verified)
                    .map_err(|_| IngestError::InvalidArtifact)?;
                let parent = SealedDailyParent::First(&sealed);
                let frontier = write_source_frontier(&roots.frontiers, &prepared, &parent, None)
                    .map_err(|_| IngestError::InvalidArtifact)?;
                clock.check()?;
                cycle
                    .take()
                    .ok_or(IngestError::Database)?
                    .admit(&parent, &frontier)
                    .await
                    .map_err(IngestError::database)?
            }
            Some(refreshed) => {
                let verified = verify_daily_continuation_candidate(
                    &candidate,
                    refreshed,
                    &config.policy,
                    verifier,
                )
                .map_err(|_| IngestError::InputBlocked)?;
                let prepared = prepare_continued_source_frontier(&verified)
                    .map_err(|_| IngestError::InputBlocked)?;
                check_orphan(expected_orphan.as_ref(), verified.pins(), prepared.sha256())?;
                catalog.revalidate()?;
                clock.check()?;
                let sealed = write_verified_daily_continuation(
                    &roots.admissions,
                    &verified,
                    &base,
                    &config.policy,
                )
                .map_err(|_| IngestError::InvalidArtifact)?;
                let parent = SealedDailyParent::Continued(&sealed);
                let frontier = write_source_frontier(
                    &roots.frontiers,
                    &prepared,
                    &parent,
                    Some(refreshed.sealed()),
                )
                .map_err(|_| IngestError::InvalidArtifact)?;
                clock.check()?;
                cycle
                    .take()
                    .ok_or(IngestError::Database)?
                    .admit(&parent, &frontier)
                    .await
                    .map_err(IngestError::database)?
            }
        };
        Ok(admission_outcome(outcome))
    }
    .await;
    match result {
        Ok(outcome) => {
            state.clear_pending().map_err(|_| IngestError::State)?;
            Ok(outcome)
        }
        Err(error) => {
            if let Some(cycle) = cycle {
                // Preserve the hint if completion or release is uncertain.
                cycle
                    .finish_failure(error.poll_failure())
                    .await
                    .map_err(IngestError::database)?;
                if tracks_current_receipt {
                    state.clear_pending().map_err(|_| IngestError::State)?;
                }
            }
            Err(error)
        }
    }
}
fn check_orphan(
    expected: Option<&(DailyPricePinsV1Wire, Option<ContentHash>)>,
    actual: &DailyPricePinsV1Wire,
    frontier: &ContentHash,
) -> Result<(), IngestError> {
    if expected.is_some_and(|(pins, prior_frontier)| {
        pins != actual || prior_frontier.as_ref().is_some_and(|hash| hash != frontier)
    }) {
        Err(IngestError::Conflict)
    } else {
        Ok(())
    }
}
fn admission_outcome(value: DailyAdmissionOutcome) -> IngestOutcome {
    match value {
        DailyAdmissionOutcome::Admitted => IngestOutcome::Admitted,
        DailyAdmissionOutcome::Replay => IngestOutcome::Replay,
        DailyAdmissionOutcome::Blocked(code) => IngestOutcome::Blocked(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn hash(text: &str) -> ContentHash {
        ContentHash::from_bytes(text.as_bytes())
    }
    fn settings_value() -> serde_json::Value {
        json!({
            "schema":"owner-beta-daily-process-v1","owner":Uuid::new_v4(),"policy_sha256":hash("policy"),
            "policy_path":"/installed/policy.json","historical_root":"/artifacts/historical",
            "raw_root":"/raw","candidate_root":"/artifacts/candidates","admitted_root":"/artifacts/admitted",
            "frontier_root":"/artifacts/frontiers","producer_state_root":"/state/producer","admitter_state_root":"/state/admitter",
            "raw_owner_uid":10001,"scan":{"max_manifest_bytes":1024,"max_manifest_records":8,"max_batches":8,
                "max_files_per_batch":8,"max_files":16,"max_batch_metadata_bytes":4096,
                "max_source_file_bytes":4096,"max_retained_bytes":65536,"max_open_descriptors":64,
                "max_data_root_bytes":4096,"max_data_root_components":64,"max_metadata_json_depth":32},
            "max_artifact_entries":100,"max_replay_admissions":100,"max_cycle_seconds":60,
            "code_commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","entitlement_reference":"fixture-only",
            "entitlement_sha256":hash("entitlement"),"first_auto_session":"2026-08-31"
        })
    }
    fn policy() -> PinnedDailyPolicy {
        let value = json!({
            "schema":"owner-beta-daily-etf11-policy-v1","capability":"PRICE_RETURN_ONLY",
            "audience":"OWNER_ONLY","vendor_snapshot":true,"strict_pit":false,
            "base":{"kind":"historical_price_only_v3","pins":{
                "candidate_content_sha256":hash("content"),"artifact_manifest_sha256":hash("manifest"),
                "stage5_manifest_sha256":hash("price"),"action_manifest_sha256":hash("actions"),
                "approval_registry_sha256":hash("registry")}},
            "instruments":["069500.KRX","102110.KRX","114260.KRX","132030.KRX","133690.KRX","143850.KRX","148070.KRX","153130.KRX","192090.KRX","195930.KRX","229200.KRX"],
            "price_semantics":"KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC","cash_action_policy":"CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1",
            "normalizer_version":"daily-raw-reconstruction-v1","materializer_version":"daily-price-only-v1",
            "source_kinds":[{"provider":"kis","market":"kr","kind":"bars"},{"provider":"kis","market":"kr","kind":"calendar"},
                {"provider":"kis","market":"kr","kind":"corporate_actions"},{"provider":"kis","market":"kr","kind":"reference"},{"provider":"kis-calendar","market":"kr","kind":"calendar"}],
            "limits":{"max_policy_bytes":262144,"max_manifest_bytes":16777216,"max_admission_bytes":1048576,
                "max_normalization_bytes":16777216,"max_bars_bytes":134217728,"max_sessions":20000,
                "max_civil_dates_per_addition":370,"max_action_chains_per_session":32768,"max_file_evidence_per_session":262144,
                "max_source_file_bytes":67108864,"max_normalized_document_bytes":67108864,"max_copy_depth":16,
                "max_pages_per_action_chain":10,"max_cash_rows_per_chain":100000,"max_json_depth":32}
        });
        let wire = serde_json::from_value(value).unwrap();
        let bytes = market_data::daily_price_only::encode_daily_price_policy_v1(&wire).unwrap();
        PinnedDailyPolicy::from_pinned_bytes(&bytes, &ContentHash::from_bytes(&bytes)).unwrap()
    }
    fn config() -> DailyIngestConfig {
        let mut settings: Settings = serde_json::from_value(settings_value()).unwrap();
        let policy = policy();
        settings.policy_sha256 = policy.sha256().clone();
        let (names, replay) = validate_settings(&settings).unwrap();
        DailyIngestConfig {
            settings,
            policy,
            names,
            replay,
        }
    }
    #[test]
    fn daily_ingest_configuration_requires_explicit_finite_disjoint_roots_and_budgets() {
        let valid = settings_value();
        assert!(validate_settings(&serde_json::from_value(valid.clone()).unwrap()).is_ok());
        for (key, replacement) in [
            ("raw_root", json!("relative")),
            ("raw_root", json!("/")),
            ("raw_root", json!("/raw/../other")),
            ("raw_root", json!("/raw//data")),
            ("raw_root", json!("/data/not-raw")),
            ("candidate_root", json!("/artifacts/admitted/child")),
            ("frontier_root", json!("/artifacts/admitted")),
            ("code_commit", json!("abcdef0")),
            ("max_cycle_seconds", json!(0)),
            ("max_cycle_seconds", json!(3601)),
            ("max_artifact_entries", json!(0)),
            ("max_replay_admissions", json!(0)),
            ("first_auto_session", json!("2026-08-28")),
            ("entitlement_reference", json!(" fixture-only ")),
            ("owner", json!(Uuid::nil())),
        ] {
            let mut value = valid.clone();
            value[key] = replacement;
            let settings: Settings = serde_json::from_value(value).unwrap();
            assert!(validate_settings(&settings).is_err(), "{key}");
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("scan");
        assert!(serde_json::from_value::<Settings>(missing).is_err());
        let mut extra = valid.clone();
        extra["implicit_defaults"] = json!(true);
        assert!(serde_json::from_value::<Settings>(extra).is_err());
        let mut zero = valid;
        zero["scan"]["max_manifest_records"] = json!(0);
        assert!(validate_settings(&serde_json::from_value(zero).unwrap()).is_err());
    }

    #[cfg(target_os = "linux")]
    fn raw_controls(raw: &Path, manifest: &[u8]) {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(raw).unwrap();
        fs::set_permissions(raw, fs::Permissions::from_mode(0o750)).unwrap();
        for provider in ["kis", "kis-calendar"] {
            let mut path = raw.to_path_buf();
            for part in [
                "manifests".to_owned(),
                format!("provider={provider}"),
                "market=kr".to_owned(),
            ] {
                path.push(part);
                fs::create_dir_all(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
            }
            for (name, bytes) in [("commit.lock", &b""[..]), ("manifest.jsonl", manifest)] {
                let leaf = path.join(name);
                fs::write(&leaf, bytes).unwrap();
                fs::set_permissions(leaf, fs::Permissions::from_mode(0o640)).unwrap();
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn raw_capture_fixture() -> (tempfile::TempDir, DailyIngestConfig) {
        use std::os::unix::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        let mut value = config();
        value.settings.raw_root = temp.path().join("data/raw");
        value.settings.raw_owner_uid = temp.path().metadata().unwrap().uid();
        // This fixture exercises only the actual Raw reader boundary. It does
        // not approve a historical artifact or claim an admitted daily input.
        (temp, value)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daily_ingest_reads_mounted_raw_leaf_once_and_never_nested_decoy() {
        let (_temp, config) = raw_capture_fixture();
        raw_controls(&config.settings.raw_root, b"");
        raw_controls(
            &config.settings.raw_root.join("raw"),
            b"invalid-nested-manifest\n",
        );
        let stop = AtomicBool::new(false);
        let clock = CycleClock::new(&config, &stop);
        let snapshots = capture_raw(&config, &clock).unwrap();
        for (snapshot, provider) in snapshots.iter().zip(["kis", "kis-calendar"]) {
            assert_eq!(snapshot.scope().provider(), provider);
            assert!(snapshot.manifest_bytes().is_empty());
            assert!(snapshot.batches().is_empty());
        }
        assert_eq!(
            raw_data_root(Path::new("/data/raw")).unwrap(),
            Path::new("/data")
        );
        assert_eq!(raw_data_root(Path::new("/raw")).unwrap(), Path::new("/"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daily_ingest_missing_raw_controls_cannot_fall_back_to_nested_tree() {
        let (_temp, config) = raw_capture_fixture();
        raw_controls(&config.settings.raw_root.join("raw"), b"");
        let stop = AtomicBool::new(false);
        assert!(matches!(
            capture_raw(&config, &CycleClock::new(&config, &stop)),
            Err(IngestError::InputBlocked)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daily_ingest_raw_leaf_symlink_keeps_reader_no_follow_boundary() {
        use std::{fs, os::unix::fs::symlink};
        let (_temp, config) = raw_capture_fixture();
        let parent = config.settings.raw_root.parent().unwrap();
        let target = parent.join("different-source");
        raw_controls(&target, b"");
        symlink(&target, &config.settings.raw_root).unwrap();
        let stop = AtomicBool::new(false);
        assert!(matches!(
            capture_raw(&config, &CycleClock::new(&config, &stop)),
            Err(IngestError::InputBlocked)
        ));
        assert!(
            fs::symlink_metadata(&config.settings.raw_root)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(target.join("manifests/provider=kis/market=kr/manifest.jsonl")).unwrap(),
            b""
        );
    }

    #[test]
    fn daily_ingest_raw_leaf_name_is_explicit_without_filesystem_resolution() {
        for invalid in [
            "/data",
            "/data/renamed",
            "/data/Raw",
            "/data/raw/",
            "relative/raw",
            "/data/../raw",
        ] {
            assert!(matches!(
                raw_data_root(Path::new(invalid)),
                Err(IngestError::Configuration)
            ));
        }
    }

    #[tokio::test]
    async fn daily_ingest_disabled_and_interrupted_cycles_never_open_protected_inputs_or_database()
    {
        let config = config();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let stop = AtomicBool::new(false);
        let now = Utc::now();
        assert_eq!(
            produce_daily_once(&config, DailyProcessMode::Disabled, now, &stop),
            Ok(IngestOutcome::Disabled)
        );
        assert_eq!(
            admit_daily_once(&pool, &config, DailyProcessMode::Disabled, now, &stop).await,
            Ok(IngestOutcome::Disabled)
        );
        stop.store(true, Ordering::Release);
        assert_eq!(
            produce_daily_once(&config, DailyProcessMode::OwnerOnly, now, &stop),
            Err(IngestError::Interrupted)
        );
        assert_eq!(
            admit_daily_once(&pool, &config, DailyProcessMode::OwnerOnly, now, &stop).await,
            Err(IngestError::Interrupted)
        );
        assert_eq!(pool.size(), 0);
        pool.close().await;
    }
    #[test]
    fn daily_ingest_orphan_recovery_requires_all_admission_and_optional_frontier_pins() {
        let pins = DailyPricePinsV1Wire {
            content_sha256: hash("content"),
            manifest_sha256: hash("manifest"),
            admission_sha256: hash("admission"),
            policy_sha256: hash("policy"),
        };
        let frontier = hash("frontier");
        assert!(check_orphan(Some(&(pins.clone(), None)), &pins, &frontier).is_ok());
        assert!(
            check_orphan(
                Some(&(pins.clone(), Some(frontier.clone()))),
                &pins,
                &frontier
            )
            .is_ok()
        );
        assert!(
            check_orphan(
                Some(&(pins.clone(), Some(hash("changed")))),
                &pins,
                &frontier
            )
            .is_err()
        );
        for changed in [0, 1, 2, 3] {
            let mut prior = pins.clone();
            match changed {
                0 => prior.content_sha256 = hash("changed"),
                1 => prior.manifest_sha256 = hash("changed"),
                2 => prior.admission_sha256 = hash("changed"),
                _ => prior.policy_sha256 = hash("changed"),
            }
            assert!(check_orphan(Some(&(prior, None)), &pins, &frontier).is_err());
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn daily_ingest_pinned_files_reject_wrong_hash_limit_symlink_and_writable_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("configuration.json");
        std::fs::write(&path, b"bounded settings").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
        let hash = ContentHash::from_bytes(b"bounded settings");
        assert_eq!(pinned_bytes(&path, &hash, 64).unwrap(), b"bounded settings");
        assert!(pinned_bytes(&path, &hash, 1).is_err());
        assert!(pinned_bytes(&path, &ContentHash::from_bytes(b"different"), 64).is_err());
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(pinned_bytes(&link, &hash, 64).is_err());
        std::fs::hard_link(&path, directory.path().join("hard")).unwrap();
        assert!(pinned_bytes(&path, &hash, 64).is_err());
        std::fs::remove_file(directory.path().join("hard")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660)).unwrap();
        assert!(pinned_bytes(&path, &hash, 64).is_err());
    }

    #[test]
    fn i6_installer_shared_config_and_reader_match_runtime_contracts() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/owner-beta-daily-install/config.json"
        ));
        let settings: Settings = serde_json::from_slice(bytes).unwrap();
        assert!(validate_settings(&settings).is_ok());
        let policy = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/evidence/owner-beta-daily-price-policy-v1.json"
        ));
        assert_eq!(settings.policy_sha256, ContentHash::from_bytes(policy));
        assert!(PinnedDailyPolicy::from_pinned_bytes(policy, &settings.policy_sha256).is_ok());
        let reader: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../configs/evidence/owner-beta-daily-reader-v1.json"
        )))
        .unwrap();
        assert_eq!(
            reader,
            json!({
                "schema": "owner-beta-daily-reader-v1",
                "input_kind": factor_engine::daily_price_only::DAILY_PRICE_ONLY_INPUT_KIND,
                "required_migration_versions": [60, 61, 62],
                "historical_v2_v3": true,
                "preserve_daily_history": true
            })
        );
    }
}

#[cfg(test)]
mod config_custody_tests {
    use super::*;
    use market_data::checked_close::{Errno, Issue};
    use std::{
        cell::{Cell, RefCell},
        convert::Infallible,
        rc::Rc,
    };
    struct Counted {
        log: Rc<RefCell<Vec<&'static str>>>,
        error: Option<Errno>,
    }
    impl ConfigClose for Counted {
        fn close(self) -> Result<(), Errno> {
            self.log.borrow_mut().push("close");
            self.error.map_or(Ok(()), Err)
        }
    }
    struct Body(Box<str>);
    struct Row(Box<str>);

    #[test]
    fn config_policy_error_survives_unknown_close_and_partial_bytes() {
        let attempt = ConfigAttempt::default();
        let invocation = ConfigInvocation::new(&attempt);
        let mut session = ConfigSession::borrowed_in(&invocation, &attempt, &attempt);
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = attempt.adopt(
            Counted {
                log: log.clone(),
                error: Some(Errno::Interrupted),
            },
            ConfigRole::Source,
        );
        let bytes = b"partial ordinary bytes".to_vec();
        let body = Body("original error".into());
        let address = body.0.as_ptr();
        let result = session.retire(owner, Err::<(), _>(ConfigFailure::from(body)));
        let source = result.err().unwrap();
        assert_eq!(source.policy().unwrap().0.as_ptr(), address);
        let completion = session.finish_local();
        assert!(source.report(&completion).is_none());
        assert_eq!(bytes, b"partial ordinary bytes");
        assert_eq!(&*log.borrow(), &["close"]);
        assert_eq!(
            (attempt.report().attempted(), attempt.report().unknown()),
            (1, 1)
        );
        assert!(completion.local().unwrap().is_err());
        assert!(matches!(
            attempt.report().first(),
            Some(Issue::Close { .. })
        ));
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    fn lower_byte_length_preserves_empty_and_exact_limit() {
        assert!(!lower_config_len_invalid(0, 16 * 1024));
        assert!(!lower_config_len_invalid(16 * 1024, 16 * 1024));
        assert!(lower_config_len_invalid(16 * 1024 + 1, 16 * 1024));
        assert!(!lower_config_len_invalid(256 * 1024, 256 * 1024));
        assert!(lower_config_len_invalid(256 * 1024 + 1, 256 * 1024));
        assert!(serde_json::from_slice::<serde_json::Value>(b"").is_err());
    }

    #[test]
    fn typed_config_order_has_distinct_reads_and_stops_at_failure() {
        for fail in 0..4 {
            let mut log = Vec::new();
            let result = typed_config_flow(
                &mut log,
                |log| {
                    log.push("config read/parse");
                    if fail == 0 {
                        Err(Body("read".into()))
                    } else {
                        Ok(())
                    }
                },
                |log| {
                    log.push("validate");
                    if fail == 1 {
                        Err(Body("validate".into()))
                    } else {
                        Ok(())
                    }
                },
                |log| {
                    log.push("policy read/parse");
                    if fail == 2 {
                        Err(Body("policy".into()))
                    } else {
                        Ok(())
                    }
                },
                |log, ()| {
                    log.push("construct");
                    Row("configured".into())
                },
            );
            let expected = [
                "config read/parse",
                "validate",
                "policy read/parse",
                "construct",
            ];
            assert_eq!(log.as_slice(), &expected[..fail + 1]);
            assert_eq!(result.is_ok(), fail == 3);
        }
        let mut reads = Vec::new();
        let result = typed_config_flow(
            &mut reads,
            |reads| {
                reads.push(b"typed config SECOND read".to_vec());
                Ok::<(), Body>(())
            },
            |_| Ok(()),
            |reads| {
                reads.push(b"policy THIRD read".to_vec());
                Ok(())
            },
            |_, ()| (),
        );
        assert!(result.is_ok());
        assert_ne!(reads[0], reads[1]);
    }

    #[test]
    fn typed_config_retains_partial_nonclone_settings_and_moves_success_once() {
        struct State {
            settings: Option<Row>,
            policy: Option<Row>,
        }
        let mut state = State {
            settings: None,
            policy: None,
        };
        let result = typed_config_flow(
            &mut state,
            |s| {
                s.settings = Some(Row("parsed settings".into()));
                Ok::<(), Body>(())
            },
            |_| Err::<(), _>(Body("validation".into())),
            |_| panic!("policy must not be read"),
            |_, ()| (),
        );
        assert!(result.is_err());
        assert_eq!(&*state.settings.as_ref().unwrap().0, "parsed settings");
        let address = state.settings.as_ref().unwrap().0.as_ptr();
        let result = typed_config_flow(
            &mut state,
            |_| Ok::<(), Body>(()),
            |_| Ok(()),
            |s| {
                s.policy = Some(Row("parsed policy".into()));
                Ok(())
            },
            |s, ()| (s.settings.take().unwrap(), s.policy.take().unwrap()),
        );
        let (settings, policy) = match result {
            Ok(pair) => pair,
            Err(_) => panic!("ordinary success"),
        };
        assert_eq!(settings.0.as_ptr(), address);
        assert_eq!(&*policy.0, "parsed policy");
        assert!(state.settings.is_none() && state.policy.is_none());
    }

    #[test]
    fn config_foreign_ledgers_are_all_poisoned_when_dirty() {
        let actual = ConfigAttempt::default();
        let expected = ConfigAttempt::default();
        let captured = ConfigAttempt::default();
        let foreign = ConfigAttempt::default();
        actual.gap(ConfigGap::Unbound);
        let invocation = ConfigInvocation::new(&actual);
        let session = ConfigSession::borrowed_in(&invocation, &expected, &captured);
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = foreign.adopt(
            Counted {
                log: log.clone(),
                error: None,
            },
            ConfigRole::Source,
        );
        assert!(session.compare::<Body>(Some(owner.attempt())).is_err());
        for scope in [&actual, &expected, &captured, &foreign] {
            assert!(!scope.check());
        }
        assert_eq!(actual.report().attempted(), 0);
        assert_eq!(foreign.report().live(), 1);
        assert!(!owner.finish());
        let completion = session.finish_local();
        assert!(completion.owner_scope(0).unwrap().is_err());
        assert_eq!(foreign.report().closed(), 1);
        assert_eq!(actual.report().closed(), 0);
    }

    #[test]
    fn read_ok_late_close_keeps_value_and_blocks_next_parser() {
        let attempt = ConfigAttempt::default();
        let invocation = ConfigInvocation::new(&attempt);
        let mut session = ConfigSession::borrowed_in(&invocation, &attempt, &attempt);
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = attempt.adopt(
            Counted {
                log: log.clone(),
                error: Some(Errno::Io),
            },
            ConfigRole::Source,
        );
        let value = Row("actual ordinary readOk".into());
        let (original, later) = retain_config_result(Ok::<_, Body>(value), || {
            session.retire(owner, Ok::<(), ConfigFailure<Body>>(()))
        });
        assert_eq!(
            &*original.as_ref().ok().unwrap().0,
            "actual ordinary readOk"
        );
        assert_eq!(later.as_ref().err().unwrap().kind(), ConfigFirstKind::Late);
        let parser = Cell::new(0);
        if later.is_ok() {
            parser.set(parser.get() + 1);
        }
        assert_eq!(parser.get(), 0);
        let completion = session.finish_local();
        assert!(!completion.is_clean());
        assert!(later.err().unwrap().report(&completion).is_none());
        assert_eq!(&*log.borrow(), &["close"]);
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn post_retirement_local_refusal_retains_read_and_stops_parsing() {
        for policy_first in [false, true] {
            let attempt = ConfigAttempt::default();
            let invocation = ConfigInvocation::new(&attempt);
            let mut session = ConfigSession::borrowed_in(&invocation, &attempt, &attempt);
            let log = Rc::new(RefCell::new(Vec::new()));
            let outer = attempt.adopt(
                Counted {
                    log: log.clone(),
                    error: None,
                },
                ConfigRole::Ancestor,
            );
            let owner = attempt.adopt(
                Counted {
                    log: log.clone(),
                    error: None,
                },
                ConfigRole::Source,
            );
            let bytes = b"actual returned ordinary read bytes".to_vec();
            let bytes_address = bytes.as_ptr();
            let body = Body("first nonclone policy error".into());
            let body_address = body.0.as_ptr();
            let original = if policy_first {
                Err(ConfigFailure::from(body))
            } else {
                Ok(())
            };
            let result = session.retire(owner, original);
            assert_eq!(result.is_ok(), !policy_first);
            assert_eq!(attempt.report().closed(), 1);
            assert_eq!(attempt.report().live(), 1);
            if !policy_first {
                // This genuine shared-ledger change occurs after retirement.
                assert!(attempt.check());
                attempt.clone().gap(ConfigGap::Abandoned);
            }
            let local = invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()));
            assert!(local.is_err());
            let parser_calls = Cell::new(0);
            let next_reads = Cell::new(0);
            let result = typed_config_flow(
                &mut session,
                |session| {
                    session
                        .complete_read(
                            ConfigReadStage::TypedConfig,
                            bytes,
                            !policy_first,
                            ConfigFirstKind::Policy,
                            result,
                            Some(local),
                        )
                        .map(|_| ())
                },
                |_| {
                    parser_calls.set(parser_calls.get() + 1);
                    Ok(())
                },
                |_| {
                    next_reads.set(next_reads.get() + 1);
                    Ok(())
                },
                |_, ()| (),
            );
            let error = result.err().unwrap();
            assert_eq!(parser_calls.get(), 0);
            assert_eq!(next_reads.get(), 0);
            if policy_first {
                assert_eq!(error.policy().unwrap().0.as_ptr(), body_address);
            } else {
                assert_eq!(error.kind(), ConfigFirstKind::Late);
                assert!(session.failed);
            }
            let completion = session.finish_local();
            assert_eq!(completion.read_count(), 1);
            assert_eq!(completion.reads[0].bytes.as_ptr(), bytes_address);
            assert_eq!(
                completion.reads[0].bytes,
                b"actual returned ordinary read bytes"
            );
            let read = completion.read(0).unwrap();
            assert_eq!(read.original_ok(), !policy_first);
            assert_eq!(read.original_kind().is_none(), !policy_first);
            assert!(read.local().unwrap().is_err());
            assert!(error.report(&completion).is_none());
            assert!(!completion.is_clean());
            assert_eq!(attempt.report().live(), 1);
            assert_eq!(&*log.borrow(), &["close"]);
            assert!(!outer.finish());
            assert_eq!(&*log.borrow(), &["close", "close"]);
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
    }

    #[test]
    fn config_unwind_is_gap_and_once_cleanup_without_terminal() {
        let attempt = ConfigAttempt::default();
        let invocation = ConfigInvocation::new(&attempt);
        let log = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(
            Counted {
                log: log.clone(),
                error: None,
            },
            ConfigRole::Source,
        );
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _session = ConfigSession::borrowed_in(&invocation, &attempt, &attempt);
            let _inner = attempt.adopt(
                Counted {
                    log: log.clone(),
                    error: None,
                },
                ConfigRole::Source,
            );
            panic!("ordinary read unwind");
        }));
        assert!(unwind.is_err());
        assert_eq!(attempt.report().attempted(), 1);
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(
            attempt.report().first(),
            Some(Issue::Gap(ConfigGap::Abandoned))
        );
        assert!(!outer.finish());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    fn actual_config_preflight_refuses_before_native_birth() {
        let actual = ConfigAttempt::default();
        let other = ConfigAttempt::default();
        let invocation = ConfigInvocation::new(&actual);
        let mut session = ConfigSession::borrowed_in(&invocation, &other, &actual);
        let error = pinned_bytes_with(
            &mut session,
            Path::new("not-opened"),
            &ContentHash::from_bytes(b"test"),
            1,
            ConfigReadStage::TypedConfig,
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), ConfigFirstKind::Scope);
        let completion = session.finish_local();
        assert_eq!(completion.read_count(), 0);
        assert_eq!(actual.report().attempted(), 0);
        assert!(!other.check() && !actual.check());
        assert!(error.report(&completion).is_some());
        let debug = format!("{error:?} {completion:?}");
        assert!(!debug.contains("not-opened") && !debug.contains("Report"));
    }
}
