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
        calendar::{VerifiedCalendarProjection, expected_calendar_session, project_calendar},
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
        let bytes = pinned_bytes(path, expected, 16 * 1024)?;
        let settings: Settings =
            serde_json::from_slice(&bytes).map_err(|_| IngestError::Configuration)?;
        let (names, replay) = validate_settings(&settings)?;
        let bytes = pinned_bytes(&settings.policy_path, &settings.policy_sha256, 256 * 1024)?;
        let policy = PinnedDailyPolicy::from_pinned_bytes(&bytes, &settings.policy_sha256)
            .map_err(|_| IngestError::Configuration)?;
        Ok(Self {
            settings,
            policy,
            names,
            replay,
        })
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
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| IngestError::Configuration)?;
        let before = file.metadata().map_err(|_| IngestError::Configuration)?;
        if !canonical_absolute(path)
            || !before.is_file()
            || before.len() > limit
            || before.nlink() != 1
            || before.mode() & 0o022 != 0
        {
            return Err(IngestError::Configuration);
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| IngestError::Configuration)?;
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
            || ContentHash::from_bytes(&bytes) != *expected
        {
            return Err(IngestError::Configuration);
        }
        Ok(bytes)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, expected, limit);
        Err(IngestError::Configuration)
    }
}

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
    let reader = CommittedRawReader::new(&config.settings.raw_root);
    let capture = |provider| {
        clock.check()?;
        let scope = ApprovedRawScope::new(provider, "kr", config.settings.raw_owner_uid)
            .map_err(|_| IngestError::Configuration)?;
        let limits = ScanLimits::new(config.settings.scan.values())
            .map_err(|_| IngestError::Configuration)?;
        reader
            .snapshot(scope, limits)
            .map_err(|_| IngestError::InputBlocked)
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
                let original = resolve_original(snapshots, &reference)
                    .map_err(|_| IngestError::InputBlocked)?;
                bytes = bytes
                    .checked_add(original.original_bytes().len() as u64)
                    .ok_or(IngestError::InputBlocked)?;
                if bytes > config.settings.scan.max_retained_bytes as u64 {
                    return Err(IngestError::InputBlocked);
                }
                values.push(
                    project_calendar(&original, config.settings.owner, config.policy.sha256())
                        .map_err(|_| IngestError::InputBlocked)?,
                );
            }
        }
    }
    clock.check()?;
    Ok(values)
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
    .map_err(|_| IngestError::InputBlocked)?;
    let parent = current.as_ref().map(ReopenedDailyObservation::as_parent);
    let _refreshed = match (&current, &parent) {
        (Some(current), Some(parent)) => Some(
            refresh_source_frontier(parent, current.frontier(), &snapshots).map_err(|error| {
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
    .map_err(|_| IngestError::InputBlocked)?;
    clock.check()?;
    match current.as_ref() {
        None => {
            let extension = reconstruct_historical_extension(&prefix, &sources, &snapshots)
                .map_err(|_| IngestError::InputBlocked)?;
            let candidate = build_historical_daily_candidate(&extension, commit.clone())
                .map_err(|_| IngestError::InputBlocked)?;
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
            .map_err(|_| IngestError::InputBlocked)?;
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
