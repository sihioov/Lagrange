//! Observed Raw admission evidence for the existing role-separated helper.
//!
//! This is a two-real-read correspondence bridge, not an atomic snapshot,
//! protected/config/SQLx cleanup proof, Root permit, or activation capability.
use super::DailyContext;
use crate::owner_beta::paper::contract::PaperEligibilityReasonCode;
use crate::owner_beta::paper::source_evidence::{
    self as wire, CheckedCurrentCompletion, CheckedCurrentFailure, CheckedProtectedSourceChain,
    CheckedSourceFailure, CurrentSourceKind, EvidenceError, ExecutionGateFailure,
    ProtectedCurrentBinding, ProtectedExecutionContext, ProtectedSourceChain,
    SourceChainSourceKind,
};
use chrono::DateTime;
use domain::ContentHash;
use market_data::{
    CheckedHistoricalApproval, CheckedHistoricalApprovalCompletion,
    CheckedHistoricalApprovalFailure, HistoricalApprovalFirstKind, HistoricalUseFailure,
    HistoricalV3FirstKind, SourceFileReference, approve_historical_price_only_artifact_checked,
    checked_close::{Attempt, Failure, Gap, Report},
    daily_price_only::{
        DailyFileEvidenceV1Wire, DailyPricePinsV1Wire,
        admission::{
            CheckedDailyPolicyCompletion, CheckedDailyPolicyFailure, DailyPolicyFirstKind,
            DailyPolicySource,
        },
        artifact::{
            CheckedDailyAdmissionRoot, CheckedDailyArtifactFailure, CheckedDailyCompletion,
            DailyArtifactFirstKind,
        },
        replay::{DailyReplayError, read_sealed_daily_chain_checked},
        source::{
            ApprovedRawScope, CheckedRawSnapshotCompletion, CheckedRawSnapshotFailure,
            CheckedRawSnapshotView, CommittedRawReader, ScanLimits, SnapshotFirstKind,
        },
        verify::{
            CheckedLineageData, CheckedLineageFailure, CheckedLineageFirstKind,
            CheckedRawEvidenceSnapshots, resolve_original_checked,
        },
    },
    raw_custody::RawInvocation,
    storage::{
        CheckedCommittedEvidenceObservation, CheckedCommittedEvidenceObservationFailure,
        CommittedManifestReadLimits, RawStore,
    },
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    fmt,
};

type PolicyResult<T> = Result<T, EvidenceError>;
type UnitSettlement = Result<(), Failure<Infallible>>;

/// Fixed channels; neither a source selector nor caller-supplied authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionChannel {
    Kis,
    KisCalendar,
}
impl AdmissionChannel {
    fn index(self) -> usize {
        match self {
            Self::Kis => 0,
            Self::KisCalendar => 1,
        }
    }
    fn provider(self) -> &'static str {
        match self {
            Self::Kis => "kis",
            Self::KisCalendar => "kis-calendar",
        }
    }
}

/// Bounded classification of the original result, separate from later failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionFirstKind {
    Scope,
    Policy,
    Context,
    Execution,
    Observation(AdmissionChannel),
}

struct Observations<R> {
    channels: [Option<R>; 2],
    current: Option<CheckedCurrentCompletion>,
    config: Option<ConfigObservations>,
    raw: Option<Box<NativeRawCompletion>>,
    historical: Option<HistoricalCompletion>,
    daily: Option<NativeDailyRetention>,
}
impl<R> Observations<R> {
    fn new() -> Self {
        Self {
            channels: [None, None],
            current: None,
            config: None,
            raw: None,
            historical: None,
            daily: None,
        }
    }
    fn get(&self, channel: AdmissionChannel) -> Option<&R> {
        self.channels[channel.index()].as_ref()
    }
}

enum AdmissionSource {
    Daily(Box<DailySource>),
    Historical(Box<CheckedHistoricalApprovalFailure>),
    HistoricalUse {
        failure: Box<HistoricalUseFailure>,
        // Actual replay return projected after ownership; not a custody settlement.
        replay: Option<Result<(), DailyReplayError>>,
    },
    Raw(Box<NativeRawCompletion>),
    Config(Box<super::ContextConfigFailure>),
    Source(Box<CheckedSourceFailure>),
    Current(Box<CheckedCurrentFailure>),
    Policy(EvidenceError),
    Context(PaperEligibilityReasonCode),
    Observation {
        channel: AdmissionChannel,
        // New container only: the actual original native failure moves whole.
        failure: Box<CheckedCommittedEvidenceObservationFailure>,
    },
    Execution {
        failure: Box<ExecutionGateFailure>,
    },
}
impl From<EvidenceError> for AdmissionSource {
    fn from(source: EvidenceError) -> Self {
        Self::Policy(source)
    }
}

impl From<CheckedCurrentFailure> for AdmissionSource {
    fn from(source: CheckedCurrentFailure) -> Self {
        Self::Current(Box::new(source))
    }
}

impl From<ExecutionGateFailure> for AdmissionSource {
    fn from(source: ExecutionGateFailure) -> Self {
        Self::Execution {
            failure: Box::new(source),
        }
    }
}

// The original Result is already owned, and the mutable reader borrow has ended.
// Ordinary model C exercises this same transport without a native completion factory.
fn retain_current_result<T, E, C>(
    result: Result<T, E>,
    completion: C,
    destination: &mut Option<C>,
) -> Result<T, E> {
    *destination = Some(completion);
    result
}

struct Completion<R, E> {
    observations: Observations<R>,
    // None means both real scope comparisons were saved and refused operation.
    operation: Option<Result<(), E>>,
    expected_scope: UnitSettlement,
    captured_scope: UnitSettlement,
    local: UnitSettlement,
    terminal: UnitSettlement,
}
impl<R, E> Completion<R, E> {
    fn is_clean(&self) -> bool {
        self.operation.as_ref().is_some_and(Result::is_ok)
            && self.expected_scope.is_ok()
            && self.captured_scope.is_ok()
            && self.local.is_ok()
            && self.terminal.is_ok()
            && self
                .observations
                .current
                .as_ref()
                .is_none_or(CheckedCurrentCompletion::is_clean)
            && self
                .observations
                .config
                .as_ref()
                .is_none_or(ConfigObservations::is_clean)
            && self
                .observations
                .raw
                .as_ref()
                .is_none_or(|raw| raw.is_clean())
            && self
                .observations
                .historical
                .as_ref()
                .is_none_or(Result::is_ok)
            && self
                .observations
                .daily
                .as_ref()
                .is_none_or(NativeDailyRetention::is_clean)
    }
}

// A genuine cloned scope, gap-only on unwind. It owns or closes no resource.
struct AdmissionUnwind {
    attempt: Attempt,
    armed: bool,
}
impl Drop for AdmissionUnwind {
    fn drop(&mut self) {
        if self.armed {
            self.attempt.gap(Gap::Abandoned);
        }
    }
}

// Shared production dispatch for actual observed rows and ordinary-data models.
// Result and row bags never enter any unit settlement/terminal T.
struct LocalCompletion<R, E> {
    observations: Observations<R>,
    operation: Option<Result<(), E>>,
    expected_scope: UnitSettlement,
    captured_scope: UnitSettlement,
    local: UnitSettlement,
}
impl<R, E> LocalCompletion<R, E> {
    fn is_clean(&self) -> bool {
        self.operation.as_ref().is_some_and(Result::is_ok)
            && self.expected_scope.is_ok()
            && self.captured_scope.is_ok()
            && self.local.is_ok()
            && self
                .observations
                .current
                .as_ref()
                .is_none_or(CheckedCurrentCompletion::is_clean)
            && self
                .observations
                .config
                .as_ref()
                .is_none_or(ConfigObservations::is_clean)
            && self
                .observations
                .raw
                .as_ref()
                .is_none_or(|raw| raw.is_clean())
            && self
                .observations
                .historical
                .as_ref()
                .is_none_or(Result::is_ok)
            && self
                .observations
                .daily
                .as_ref()
                .is_none_or(NativeDailyRetention::is_clean)
    }
}
// The preflight is an already-owned genuine result; never a caller authority bool.
fn complete_admission_local<R, E>(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
    preflight: Result<(), E>,
    operation: impl FnOnce(&RawInvocation<'_>, &mut Observations<R>) -> Result<(), E>,
) -> LocalCompletion<R, E> {
    let expected_scope = invocation.settle_local_in(expected, Ok::<(), Infallible>(()));
    let captured_scope = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
    let mut unwind = AdmissionUnwind {
        attempt: captured.clone(),
        armed: true,
    };
    let mut observations = Observations::new();
    let operation = match preflight {
        Err(source) => Some(Err(source)),
        Ok(()) if expected_scope.is_ok() && captured_scope.is_ok() => {
            Some(operation(invocation, &mut observations))
        }
        Ok(()) => None,
    };
    if operation.as_ref().is_some_and(Result::is_err) && captured.check() {
        captured.gap(Gap::Abandoned);
    }
    let local = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
    unwind.armed = false;
    LocalCompletion {
        observations,
        operation,
        expected_scope,
        captured_scope,
        local,
    }
}

fn complete_admission<R, E>(
    attempt: Attempt,
    expected: &Attempt,
    captured: &Attempt,
    operation: impl FnOnce(&RawInvocation<'_>, &mut Observations<R>) -> Result<(), E>,
) -> Completion<R, E> {
    let local = {
        let invocation = RawInvocation::new(&attempt);
        complete_admission_local(&invocation, expected, captured, Ok(()), operation)
    };
    // Only this owned-scope wrapper performs a terminal finish.
    let terminal = attempt.finish(Ok::<(), Infallible>(()));
    Completion {
        observations: local.observations,
        operation: local.operation,
        expected_scope: local.expected_scope,
        captured_scope: local.captured_scope,
        local: local.local,
        terminal,
    }
}

// Genuine first-report selection. No fallback to local or terminal diagnostics.
fn original_report<'a, R, E>(
    completion: &'a Completion<R, E>,
    report: impl FnOnce(&'a E) -> Option<&'a Report>,
) -> Option<&'a Report> {
    match &completion.operation {
        Some(Err(source)) => report(source),
        Some(Ok(())) => None,
        None => completion
            .expected_scope
            .as_ref()
            .err()
            .or_else(|| completion.captured_scope.as_ref().err())
            .map(Failure::report),
    }
}

// The same helper retains success before checkpoint/second/downstream effects.
fn read_channels<R, E, S>(
    state: &mut S,
    observations: &mut Observations<R>,
    mut read: impl FnMut(&mut S, AdmissionChannel) -> Result<R, E>,
    mut checkpoint: impl FnMut(&mut S) -> Result<(), E>,
    downstream: impl FnOnce(&mut S, &Observations<R>) -> Result<(), E>,
) -> Result<(), E> {
    for channel in [AdmissionChannel::Kis, AdmissionChannel::KisCalendar] {
        let result = read(state, channel);
        observations.channels[channel.index()] = Some(result?);
        checkpoint(state)?;
    }
    downstream(state, observations)
}

type NativeCompletion = Completion<CheckedCommittedEvidenceObservation, AdmissionSource>;

/// Owns both actual successful observations and real local/terminal outcomes.
/// Its private constructor is not a permit or proof of protected-resource close.
pub struct ObservedAdmission {
    completion: Box<NativeCompletion>,
}
/// Owns the original whole result, prior successful channels and later results.
pub struct ObservedAdmissionFailure {
    completion: Box<NativeCompletion>,
}
/// Borrowed diagnostics; cannot reconstruct an observation or an authority.
pub struct ObservedAdmissionView<'a> {
    completion: &'a NativeCompletion,
}
impl ObservedAdmission {
    pub fn diagnostics(&self) -> ObservedAdmissionView<'_> {
        ObservedAdmissionView {
            completion: &self.completion,
        }
    }
}
impl ObservedAdmissionFailure {
    pub fn diagnostics(&self) -> ObservedAdmissionView<'_> {
        ObservedAdmissionView {
            completion: &self.completion,
        }
    }
}
impl ObservedAdmissionView<'_> {
    pub fn daily_evidence(&self) -> Option<AdmissionDailyView<'_>> {
        admission_daily_view(
            self.completion.observations.daily.as_ref(),
            &self.completion.operation,
        )
    }

    pub fn observation(
        &self,
        channel: AdmissionChannel,
    ) -> Option<&CheckedCommittedEvidenceObservation> {
        self.completion.observations.get(channel)
    }
    pub fn first_kind(&self) -> Option<AdmissionFirstKind> {
        admission_first_kind(&self.completion.operation)
    }
    pub fn policy_failure(&self) -> Option<&EvidenceError> {
        admission_policy_failure(&self.completion.operation)
    }
    pub fn context_failure(&self) -> Option<&PaperEligibilityReasonCode> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Context(source)) => Some(source),
            Some(AdmissionSource::Config(source)) => source.reason(),
            _ => None,
        }
    }
    pub fn observation_failure(&self) -> Option<&CheckedCommittedEvidenceObservationFailure> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Observation { failure, .. }) => Some(failure),
            _ => None,
        }
    }
    pub fn execution_failure(&self) -> Option<&ExecutionGateFailure> {
        admission_execution_failure(self.completion.operation.as_ref()?.as_ref().err()?)
    }
    pub fn source_report(&self) -> Option<&Report> {
        original_report(self.completion, |source| {
            admission_report_with_config(source, self.completion.observations.config.as_ref())
        })
    }
    pub fn current_failure(&self) -> Option<&CheckedCurrentFailure> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Current(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn config_evidence(&self) -> Option<AdmissionConfigView<'_>> {
        self.completion
            .observations
            .config
            .as_ref()
            .map(|observed| AdmissionConfigView { observed })
    }
    pub fn config_ingest_failure(
        &self,
    ) -> Option<&crate::owner_beta::runner::daily_ingest::IngestError> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Config(source)) => source.ingest(),
            _ => None,
        }
    }

    pub fn historical_failure(&self) -> Option<&CheckedHistoricalApprovalFailure> {
        match original_historical_source(&self.completion.operation) {
            Some(AdmissionSource::Historical(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn historical_use_failure(&self) -> Option<&HistoricalUseFailure> {
        admission_historical_use(&self.completion.operation)
    }
    pub fn historical_replay_result(&self) -> Option<&Result<(), DailyReplayError>> {
        match original_historical_source(&self.completion.operation) {
            Some(AdmissionSource::HistoricalUse { replay, .. }) => replay.as_ref(),
            _ => None,
        }
    }
    pub fn historical_completion(
        &self,
    ) -> Option<&Result<CheckedHistoricalApprovalCompletion, CheckedHistoricalApprovalFailure>>
    {
        self.completion.observations.historical.as_ref()
    }
    pub fn raw_evidence(&self) -> Option<AdmissionRawView<'_>> {
        admission_raw_view(
            self.completion.observations.raw.as_deref(),
            &self.completion.operation,
        )
    }
    pub fn current_completion(&self) -> Option<&CheckedCurrentCompletion> {
        self.completion.observations.current.as_ref()
    }
    pub fn expected_scope(&self) -> &UnitSettlement {
        &self.completion.expected_scope
    }
    pub fn captured_scope(&self) -> &UnitSettlement {
        &self.completion.captured_scope
    }
    pub fn local_settlement(&self) -> &UnitSettlement {
        &self.completion.local
    }
    pub fn terminal_settlement(&self) -> &UnitSettlement {
        &self.completion.terminal
    }
}
impl fmt::Debug for ObservedAdmissionView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedAdmission")
            .field("first_kind", &self.first_kind())
            .field(
                "kis_records",
                &self
                    .observation(AdmissionChannel::Kis)
                    .map(|o| o.records().len()),
            )
            .field(
                "calendar_records",
                &self
                    .observation(AdmissionChannel::KisCalendar)
                    .map(|o| o.records().len()),
            )
            .field("source_report_present", &self.source_report().is_some())
            .field("local_failed", &self.completion.local.is_err())
            .field("terminal_failed", &self.completion.terminal.is_err())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for ObservedAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.diagnostics(), f)
    }
}
impl fmt::Debug for ObservedAdmissionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostics().fmt(f)
    }
}

/// Verifies the existing admission policy using both actual observed channels.
/// The Raw ledger excludes protected/config/snapshot/SQLx resource completion.
pub fn verify_admission_observed(
    source: &ProtectedSourceChain,
    pins: &DailyPricePinsV1Wire,
) -> Result<ObservedAdmission, ObservedAdmissionFailure> {
    let attempt = Attempt::default();
    let captured = attempt.clone();
    let expected = attempt.clone();
    let completion = Box::new(complete_admission(
        attempt,
        &expected,
        &captured,
        |invocation, observations| {
            verify_policy(
                &AdmissionChain::Legacy(source),
                pins,
                invocation,
                &expected,
                &captured,
                observations,
            )
        },
    ));
    if completion.is_clean() {
        Ok(ObservedAdmission { completion })
    } else {
        Err(ObservedAdmissionFailure { completion })
    }
}

fn required(ok: bool) -> PolicyResult<()> {
    if ok { Ok(()) } else { Err(EvidenceError) }
}
fn string<'a>(v: &'a Value, k: &str) -> PolicyResult<&'a str> {
    v[k].as_str().ok_or(EvidenceError)
}

fn revalidate_boundary(
    current: &ProtectedExecutionContext<'_, '_>,
    binding: &Value,
    context: &DailyContext,
    source: &AdmissionChain<'_, '_, '_>,
    session: &mut super::ConfigSession<'_, '_>,
    retained: &mut super::ContextLoadRetention,
) -> Result<(), AdmissionSource> {
    let fresh = current
        .with_binding(Clone::clone)
        .map_err(AdmissionSource::from)?;
    required(
        !fresh.is_null() && &fresh == binding && fresh["cohort"] == source.first()?["cohort"],
    )?;
    context
        .revalidate_in(session, retained, &fresh)
        .map_err(AdmissionSource::from)?;
    source.revalidate()
}

struct AcceptedRow<'a, K> {
    provider: &'a str,
    market: &'a str,
    batch: K,
    line_number: usize,
    line_bytes: &'a [u8],
    line_sha256: &'a ContentHash,
}

// Whole accepted sets, with duplicate IDs rejected rather than deduplicated.
fn match_accepted_rows<K: Ord>(
    provider: &str,
    observed: &[AcceptedRow<'_, K>],
    legacy: &[AcceptedRow<'_, K>],
) -> PolicyResult<()> {
    required(observed.len() == legacy.len())?;
    let mut by_id = BTreeMap::new();
    for row in legacy {
        required(row.provider == provider && row.market == "kr" && row.line_number > 0)?;
        required(by_id.insert(&row.batch, row).is_none())?;
    }
    let mut seen = BTreeSet::new();
    for row in observed {
        required(row.provider == provider && row.market == "kr" && row.line_number > 0)?;
        required(seen.insert(&row.batch))?;
        let expected = by_id.get(&row.batch).ok_or(EvidenceError)?;
        required(
            row.line_number == expected.line_number
                && row.line_bytes == expected.line_bytes
                && row.line_sha256 == expected.line_sha256
                && ContentHash::from_bytes(row.line_bytes) == *row.line_sha256,
        )?;
    }
    Ok(())
}

fn match_snapshot(
    channel: AdmissionChannel,
    observation: &CheckedCommittedEvidenceObservation,
    snapshot: &CheckedRawSnapshotView<'_>,
) -> PolicyResult<()> {
    let scope = snapshot.scope().ok_or(EvidenceError)?;
    required(scope.provider() == channel.provider() && scope.market() == "kr")?;
    let observed: Vec<_> = observation
        .records()
        .iter()
        .map(|record| {
            let entry = record.entry();
            AcceptedRow {
                provider: &entry.provider,
                market: &entry.market,
                batch: entry.batch_id,
                line_number: record.line_number(),
                line_bytes: record.line_bytes(),
                line_sha256: record.line_sha256(),
            }
        })
        .collect();
    let legacy: Vec<_> = snapshot
        .batches()
        .map(|batch| AcceptedRow {
            provider: scope.provider(),
            market: scope.market(),
            batch: batch.batch_id(),
            line_number: batch.manifest_line_number(),
            line_bytes: batch.manifest_line_bytes(),
            line_sha256: batch.manifest_line_sha256(),
        })
        .collect();
    match_accepted_rows(channel.provider(), &observed, &legacy)
}

fn observed_reference(
    observations: &Observations<CheckedCommittedEvidenceObservation>,
    reference: &SourceFileReference,
    line_sha256: &ContentHash,
) -> PolicyResult<()> {
    let channel = match reference.provider.as_str() {
        "kis" => AdmissionChannel::Kis,
        "kis-calendar" => AdmissionChannel::KisCalendar,
        _ => return Err(EvidenceError),
    };
    required(reference.market == "kr")?;
    let observation = observations.get(channel).ok_or(EvidenceError)?;
    let record = observation
        .records()
        .iter()
        .find(|record| record.entry().batch_id == reference.batch_id)
        .ok_or(EvidenceError)?;
    let entry = record.entry();
    required(
        entry.provider == reference.provider
            && entry.market == reference.market
            && entry.retrieved_at == reference.retrieved_at
            && record.line_sha256() == line_sha256
            && entry.files.iter().any(|file| {
                file.file_name == reference.file_name && file.content_hash == reference.content_hash
            }),
    )
}

fn observed_file(
    observations: &Observations<CheckedCommittedEvidenceObservation>,
    evidence: &DailyFileEvidenceV1Wire,
    reference: &SourceFileReference,
) -> PolicyResult<()> {
    observed_reference(observations, reference, &evidence.manifest_line_sha256)?;
    for origin in &evidence.copied_from {
        let reference = SourceFileReference {
            provider: origin.provider.clone(),
            market: origin.market.clone(),
            batch_id: origin.batch_id,
            file_name: origin.file_name.clone(),
            content_hash: origin.wire_sha256.clone(),
            retrieved_at: origin.retrieved_at.as_timestamp(),
        };
        observed_reference(observations, &reference, &origin.manifest_line_sha256)?;
    }
    Ok(())
}

#[expect(
    clippy::drop_non_drop,
    reason = "Consumes the bound invocation to end authority borrows before finishing the same Current; this is a lifetime boundary, not native close evidence."
)]
fn verify_policy(
    source: &AdmissionChain<'_, '_, '_>,
    pins: &DailyPricePinsV1Wire,
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
    observations: &mut Observations<CheckedCommittedEvidenceObservation>,
) -> Result<(), AdmissionSource> {
    let bootstrap = invocation.unbound_bootstrap();
    let current = ProtectedCurrentBinding::open_checked_in(&bootstrap, expected, captured)?
        .into_execution_context();
    let owned_invocation = current.invocation();
    let invocation = &owned_invocation;
    let mut config_session = super::ConfigSession::borrowed_in(invocation, expected, captured);
    let mut config_retained = super::ContextLoadRetention::default();
    let mut initial_context = None;
    let mut raw_retained = None;
    let mut historical_lease = None;
    let mut daily_retained = NativeDailyRetention::new();
    let result = (|| -> Result<(), AdmissionSource> {
        let binding = current
            .with_binding(|binding| binding.clone())
            .map_err(AdmissionSource::from)?;
        required(
            !binding.is_null()
                && !binding["cohort"].is_null()
                && binding["cohort"] == source.first()?["cohort"],
        )?;
        initial_context = Some(
            DailyContext::load_in(&mut config_session, &mut config_retained, &binding)
                .map_err(AdmissionSource::from)?,
        );
        let context = initial_context.as_ref().expect("real initial context");
        revalidate_boundary(
            &current,
            &binding,
            context,
            source,
            &mut config_session,
            &mut config_retained,
        )?;
        let config = &context.config;
        let first = source.first()?;
        retain_historical_acquisition(
            approve_historical_price_only_artifact_checked(
                invocation,
                expected,
                captured,
                config.historical_path(),
            ),
            &mut historical_lease,
        )
        .map_err(|failure| AdmissionSource::Historical(Box::new(failure)))?;
        let historical = historical_lease
            .as_ref()
            .expect("actual historical lease retained");
        historical
            .approved_in(expected, captured)
            .map(|_| ())
            .map_err(|failure| AdmissionSource::HistoricalUse {
                failure: Box::new(failure),
                replay: None,
            })?;
        let root = CheckedDailyAdmissionRoot::open_in(
            invocation,
            expected,
            captured,
            config.admission_path(),
            10003,
            10001,
        )
        .map_err(|failure| {
            AdmissionSource::Daily(Box::new(DailySource::Root(Box::new(failure))))
        })?;
        let (root_result, root_completion) = run_owned_region(
            root,
            |root| {
                historical_checkpoint(historical, expected, captured)?;
                let mut replay_first = None;
                let replayed = read_sealed_daily_chain_checked(
                    invocation,
                    expected,
                    captured,
                    root,
                    pins,
                    historical,
                    config.policy(),
                    config.replay_limits(),
                    || {
                        replay_checkpoint(
                            || historical.approved_in(expected, captured).map(|_| ()),
                            &mut replay_first,
                        )
                    },
                );
                let chain = intake_daily_replay(replayed, replay_first, |chain| {
                    if captured.check() {
                        captured.gap(Gap::Abandoned);
                    }
                    chain.finish_local()
                })
                .map_err(|failure| {
                    AdmissionSource::Daily(Box::new(DailySource::Replay(failure)))
                })?;
                let (chain_result, chain_completion) = run_owned_region(
                    chain,
                    |chain| {
                        let parent = chain.as_parent();
                        daily_checkpoint(historical, expected, captured, || {
                            parent.view_in(expected, captured).map(|_| ())
                        })?;
                        let admitted = chain
                            .admitted(expected, captured)
                            .map_err(AdmissionSource::from)?;
                        daily_checkpoint(historical, expected, captured, || {
                            admitted.view_in(expected, captured).map(|_| ())
                        })?;
                        let daily_view = admitted
                            .view_in(expected, captured)
                            .map_err(AdmissionSource::from)?;
                        let session = daily_view
                            .manifest()
                            .daily_sources
                            .iter()
                            .find(|s| {
                                s.session_date.to_iso() == first["session"].as_str().unwrap_or("")
                            })
                            .ok_or(EvidenceError)?;
                        daily_checkpoint(historical, expected, captured, || {
                            admitted.view_in(expected, captured).map(|_| ())
                        })?;
                        required(daily_view.manifest().range_end == session.session_date)?;
                        daily_checkpoint(historical, expected, captured, || {
                            admitted.view_in(expected, captured).map(|_| ())
                        })?;
                        let limits =
                            ScanLimits::new(context.scan_limits).map_err(|_| EvidenceError)?;
                        let manifest_limits = CommittedManifestReadLimits {
                            max_manifest_bytes: context.scan_limits.max_manifest_bytes,
                            max_records: context.scan_limits.max_manifest_records,
                        };
                        let store = RawStore::new(&context.data_root);
                        read_channels(
                            &mut (&current, &mut config_session, &mut config_retained),
                            observations,
                            |(current, config_reads, retained), channel| {
                                historical_unit_boundary(
                                    historical,
                                    expected,
                                    captured,
                                    || admitted.view_in(expected, captured).map(|_| ()),
                                    || {
                                        revalidate_boundary(
                                            current,
                                            &binding,
                                            context,
                                            source,
                                            config_reads,
                                            retained,
                                        )
                                    },
                                )?;
                                match store.read_committed_manifest_evidence_observed_checked(
                                    invocation,
                                    expected,
                                    channel.provider(),
                                    "kr",
                                    context.raw_owner_uid,
                                    &manifest_limits,
                                ) {
                                    Ok(observation) => Ok(observation),
                                    Err(failure) => Err(AdmissionSource::Observation {
                                        channel,
                                        failure: Box::new(failure),
                                    }),
                                }
                            },
                            |(current, config_reads, retained)| {
                                historical_unit_boundary(
                                    historical,
                                    expected,
                                    captured,
                                    || admitted.view_in(expected, captured).map(|_| ()),
                                    || {
                                        revalidate_boundary(
                                            current,
                                            &binding,
                                            context,
                                            source,
                                            config_reads,
                                            retained,
                                        )
                                    },
                                )
                            },
                            |(current, config_reads, retained), observations| {
                                // Original snapshot/content policy runs only after BOTH observations.
                                // Its existing markers stay policy errors; no Raw failure is flattened.
                                let mut raw_slots = [None, None];
                                let mut raw_lineage = Vec::new();
                                let mut raw_unwind = AdmissionUnwind {
                                    attempt: captured.clone(),
                                    armed: true,
                                };
                                let original = (|| -> Result<(), RawBodySource> {
                                    let reader = CommittedRawReader::new(&context.data_root);
                                    capture_raw_pair_with_checkpoint(
                                        &mut raw_slots,
                                        |channel| {
                                            reader
                                                .snapshot_checked_in(
                                                    invocation,
                                                    expected,
                                                    captured,
                                                    ApprovedRawScope::new(
                                                        channel.provider(),
                                                        "kr",
                                                        context.raw_owner_uid,
                                                    )
                                                    .map_err(|_| EvidenceError)?,
                                                    ScanLimits::new(limits.values())
                                                        .map_err(|_| EvidenceError)?,
                                                )
                                                .map_err(|failure| RawBodySource::Snapshot {
                                                    channel,
                                                    failure: Box::new(failure),
                                                })
                                        },
                                        || {
                                            daily_checkpoint(historical, expected, captured, || {
                                                admitted.view_in(expected, captured).map(|_| ())
                                            })
                                            .map_err(RawBodySource::from)
                                        },
                                    )?;
                                    let kis = raw_slots[AdmissionChannel::Kis.index()]
                                        .as_ref()
                                        .expect("actual KIS success retained");
                                    let calendar = raw_slots[AdmissionChannel::KisCalendar.index()]
                                        .as_ref()
                                        .expect("actual calendar success retained");
                                    let snapshots = CheckedRawEvidenceSnapshots::new_in(
                                        invocation,
                                        expected,
                                        captured,
                                        &[kis, calendar],
                                        &context.entitlement_reference,
                                        domain::UtcTimestamp::now(),
                                    )?;
                                    historical_unit_boundary(
                                        historical,
                                        expected,
                                        captured,
                                        || admitted.view_in(expected, captured).map(|_| ()),
                                        || -> Result<(), RawBodySource> {
                                            match_snapshot(
                                                AdmissionChannel::Kis,
                                                observations
                                                    .get(AdmissionChannel::Kis)
                                                    .ok_or(EvidenceError)?,
                                                &snapshots.snapshot_view_in("kis", "kr")?,
                                            )
                                            .map_err(RawBodySource::from)
                                        },
                                    )?;
                                    historical_unit_boundary(
                                        historical,
                                        expected,
                                        captured,
                                        || admitted.view_in(expected, captured).map(|_| ()),
                                        || -> Result<(), RawBodySource> {
                                            match_snapshot(
                                                AdmissionChannel::KisCalendar,
                                                observations
                                                    .get(AdmissionChannel::KisCalendar)
                                                    .ok_or(EvidenceError)?,
                                                &snapshots
                                                    .snapshot_view_in("kis-calendar", "kr")?,
                                            )
                                            .map_err(RawBodySource::from)
                                        },
                                    )?;
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    let finished = &source.event("SOURCE_FINISHED")?["data"];
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    let originals =
                                        finished["originals"].as_array().ok_or(EvidenceError)?;
                                    let mut actual = Vec::new();
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    let mut files: Vec<_> = session
                                        .bars
                                        .iter()
                                        .chain(session.reference.iter())
                                        .collect();
                                    files.extend(
                                        session
                                            .calendar
                                            .iter()
                                            .filter(|c| c.civil_date == session.session_date)
                                            .map(|c| &c.source_file),
                                    );
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    for f in files {
                                        daily_checkpoint(historical, expected, captured, || {
                                            admitted.view_in(expected, captured).map(|_| ())
                                        })?;
                                        let reference = market_data::SourceFileReference {
                                            provider: f.provider.clone(),
                                            market: f.market.clone(),
                                            batch_id: f.batch_id,
                                            file_name: f.file_name.clone(),
                                            content_hash: f.wire_sha256.clone(),
                                            retrieved_at: f.retrieved_at.as_timestamp(),
                                        };
                                        historical_unit_boundary(
                                            historical,
                                            expected,
                                            captured,
                                            || admitted.view_in(expected, captured).map(|_| ()),
                                            || {
                                                observed_file(observations, f, &reference)
                                                    .map_err(RawBodySource::from)
                                            },
                                        )?;
                                        daily_checkpoint(historical, expected, captured, || {
                                            admitted.view_in(expected, captured).map(|_| ())
                                        })?;
                                        let resolved =
                                            resolve_original_checked(&snapshots, &reference)?;
                                        retain_raw_use(
                                            resolved,
                                            &mut raw_lineage,
                                            |resolved| -> Result<(), RawBodySource> {
                                                daily_checkpoint(
                                                    historical,
                                                    expected,
                                                    captured,
                                                    || {
                                                        admitted
                                                            .view_in(expected, captured)
                                                            .map(|_| ())
                                                    },
                                                )?;
                                                let view = resolved.view_in()?;
                                                daily_checkpoint(
                                                    historical,
                                                    expected,
                                                    captured,
                                                    || {
                                                        admitted
                                                            .view_in(expected, captured)
                                                            .map(|_| ())
                                                    },
                                                )?;
                                                required(view.evidence() == f)?;
                                                daily_checkpoint(
                                                    historical,
                                                    expected,
                                                    captured,
                                                    || {
                                                        admitted
                                                            .view_in(expected, captured)
                                                            .map(|_| ())
                                                    },
                                                )?;
                                                let evidence = serde_json::to_value(f)
                                                    .map_err(|_| EvidenceError)?;
                                                let original = evidence["copied_from"]
                                                    .as_array()
                                                    .and_then(|c| c.last())
                                                    .unwrap_or(&evidence);
                                                let mut item = serde_json::Map::new();
                                                for key in [
                                                    "provider",
                                                    "market",
                                                    "batch_id",
                                                    "batch_date",
                                                    "file_name",
                                                    "wire_sha256",
                                                    "size_bytes",
                                                    "batch_json_sha256",
                                                    "manifest_line_sha256",
                                                    "retrieved_at",
                                                ] {
                                                    item.insert(key.into(), original[key].clone());
                                                }
                                                let original_time = DateTime::parse_from_rfc3339(
                                                    item["retrieved_at"]
                                                        .as_str()
                                                        .ok_or(EvidenceError)?,
                                                )
                                                .map_err(|_| EvidenceError)?;
                                                if let Some(expected) = originals.iter().find(|o| {
                                                    o["provider"] == item["provider"]
                                                        && o["market"] == item["market"]
                                                        && o["batch_id"] == item["batch_id"]
                                                        && o["file_name"] == item["file_name"]
                                                }) {
                                                    required(
                                                        DateTime::parse_from_rfc3339(string(
                                                            expected,
                                                            "retrieved_at",
                                                        )?)
                                                        .map_err(|_| EvidenceError)?
                                                            == original_time,
                                                    )?;
                                                    item.insert(
                                                        "retrieved_at".into(),
                                                        expected["retrieved_at"].clone(),
                                                    );
                                                }
                                                actual.push(Value::Object(item));
                                                Ok(())
                                            },
                                            |resolved| resolved.retain_diagnostics(),
                                        )?;
                                    }
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    actual.sort_by_key(|o| {
                                        [
                                            "provider",
                                            "market",
                                            "batch_id",
                                            "file_name",
                                            "wire_sha256",
                                        ]
                                        .map(|k| o[k].as_str().unwrap_or("").to_owned())
                                    });
                                    required(
                                        actual == *originals
                                            && wire::original_set_hash(&actual)?
                                                == string(finished, "original_set_sha256")?,
                                    )?;
                                    // All action dependencies are independently reopened, including legitimate
                                    // historical reuse. The supplier allowance is never an actual GET count.
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    for action in &session.actions {
                                        for f in &action.pages {
                                            daily_checkpoint(
                                                historical,
                                                expected,
                                                captured,
                                                || admitted.view_in(expected, captured).map(|_| ()),
                                            )?;
                                            let reference = market_data::SourceFileReference {
                                                provider: f.provider.clone(),
                                                market: f.market.clone(),
                                                batch_id: f.batch_id,
                                                file_name: f.file_name.clone(),
                                                content_hash: f.wire_sha256.clone(),
                                                retrieved_at: f.retrieved_at.as_timestamp(),
                                            };
                                            historical_unit_boundary(
                                                historical,
                                                expected,
                                                captured,
                                                || admitted.view_in(expected, captured).map(|_| ()),
                                                || {
                                                    observed_file(observations, f, &reference)
                                                        .map_err(RawBodySource::from)
                                                },
                                            )?;
                                            daily_checkpoint(
                                                historical,
                                                expected,
                                                captured,
                                                || admitted.view_in(expected, captured).map(|_| ()),
                                            )?;
                                            let resolved =
                                                resolve_original_checked(&snapshots, &reference)?;
                                            retain_raw_use(
                                                resolved,
                                                &mut raw_lineage,
                                                |resolved| -> Result<(), RawBodySource> {
                                                    daily_checkpoint(
                                                        historical,
                                                        expected,
                                                        captured,
                                                        || {
                                                            admitted
                                                                .view_in(expected, captured)
                                                                .map(|_| ())
                                                        },
                                                    )?;
                                                    let view = resolved.view_in()?;
                                                    daily_checkpoint(
                                                        historical,
                                                        expected,
                                                        captured,
                                                        || {
                                                            admitted
                                                                .view_in(expected, captured)
                                                                .map(|_| ())
                                                        },
                                                    )?;
                                                    required(view.evidence() == f)?;
                                                    daily_checkpoint(
                                                        historical,
                                                        expected,
                                                        captured,
                                                        || {
                                                            admitted
                                                                .view_in(expected, captured)
                                                                .map(|_| ())
                                                        },
                                                    )?;
                                                    Ok(())
                                                },
                                                |resolved| resolved.retain_diagnostics(),
                                            )?;
                                        }
                                    }
                                    daily_checkpoint(historical, expected, captured, || {
                                        admitted.view_in(expected, captured).map(|_| ())
                                    })?;
                                    let action = &finished["action"];
                                    if !action["batch_id"].is_null() {
                                        required(session.actions.iter().any(|a| {
                                            a.range_start.to_iso()
                                                == action["start"].as_str().unwrap_or("")
                                                && a.range_end.to_iso()
                                                    == action["end"].as_str().unwrap_or("")
                                                && a.pages.iter().all(|f| {
                                                    serde_json::to_value(f.batch_id).ok().as_ref()
                                                        == Some(&action["batch_id"])
                                                })
                                        }))?;
                                    }
                                    historical_unit_boundary(
                                        historical,
                                        expected,
                                        captured,
                                        || admitted.view_in(expected, captured).map(|_| ()),
                                        || {
                                            admitted
                                                .revalidate_in(expected, captured)
                                                .map_err(AdmissionSource::from)
                                                .map_err(RawBodySource::from)
                                        },
                                    )?;
                                    historical_unit_boundary(
                                        historical,
                                        expected,
                                        captured,
                                        || admitted.view_in(expected, captured).map(|_| ()),
                                        || source.revalidate().map_err(RawBodySource::from),
                                    )
                                })();
                                if original.is_err() && captured.check() {
                                    captured.gap(Gap::Abandoned);
                                }
                                let completed =
                                    finish_raw_pair(original, raw_slots, raw_lineage, |snapshot| {
                                        snapshot.finish_local()
                                    });
                                raw_unwind.armed = false;
                                retain_raw_completion(
                                    completed,
                                    CheckedRawSnapshotCompletion::is_clean,
                                    &mut raw_retained,
                                )
                                .map_err(AdmissionSource::Raw)?;
                                historical_unit_boundary(
                                    historical,
                                    expected,
                                    captured,
                                    || admitted.view_in(expected, captured).map(|_| ()),
                                    || {
                                        revalidate_boundary(
                                            current,
                                            &binding,
                                            context,
                                            source,
                                            config_reads,
                                            retained,
                                        )
                                    },
                                )
                            },
                        )?;
                        daily_checkpoint(historical, expected, captured, || {
                            admitted.view_in(expected, captured).map(|_| ())
                        })
                    },
                    |original| mark_region_failure(captured, original),
                    |chain| chain.finish_local(),
                );
                daily_retained.chain = Some(chain_completion);
                chain_result
            },
            |original| mark_region_failure(captured, original),
            |root| root.finish_local(),
        );
        daily_retained.root = Some(root_completion);
        root_result
    })();
    observations.raw = raw_retained;
    observations.daily = Some(daily_retained);
    // Preserve the actual first source before any permanent gap/cleanup.
    if result.is_err() && captured.check() {
        captured.gap(Gap::Abandoned);
    }
    let result = retain_historical_completion(
        result,
        historical_lease.take(),
        |held| held.finish_local(),
        &mut observations.historical,
    );
    drop(historical_lease);
    let config_completion = config_session.finish_local();
    observations.config = Some(ConfigObservations {
        initial: initial_context,
        retained: config_retained,
        completion: config_completion,
    });
    drop(owned_invocation);
    let completion = current.finish_local();
    retain_current_result(result, completion, &mut observations.current)
}

// BEGIN C9 retained config observations
struct ConfigObservations {
    initial: Option<DailyContext>,
    retained: super::ContextLoadRetention,
    completion: super::ConfigCompletion,
}
impl ConfigObservations {
    fn is_clean(&self) -> bool {
        self.completion.is_clean()
    }
}
/// Counts and borrowed real completion only; private context/settings never escape.
pub struct AdmissionConfigView<'a> {
    observed: &'a ConfigObservations,
}
impl<'a> AdmissionConfigView<'a> {
    pub fn initial_context_present(&self) -> bool {
        self.observed.initial.is_some()
    }
    pub fn renewed_contexts(&self) -> usize {
        self.observed.retained.renewed.len()
    }
    pub fn partial_objects(&self) -> usize {
        self.observed.retained.partial_count()
    }
    pub fn completion(&self) -> &'a super::ConfigCompletion {
        &self.observed.completion
    }
}
impl fmt::Debug for AdmissionConfigView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmissionConfig")
            .field("initial", &self.initial_context_present())
            .field("renewed", &self.renewed_contexts())
            .field("partial", &self.partial_objects())
            .finish_non_exhaustive()
    }
}
impl From<super::ContextConfigFailure> for AdmissionSource {
    fn from(source: super::ContextConfigFailure) -> Self {
        match source {
            super::ContextConfigFailure::Reason(reason) => Self::Context(reason),
            other => Self::Config(Box::new(other)),
        }
    }
}
fn admission_report_with_config<'a>(
    source: &'a AdmissionSource,
    config: Option<&'a ConfigObservations>,
) -> Option<&'a Report> {
    match source {
        AdmissionSource::Config(failure) => {
            config.and_then(|config| failure.report(&config.completion))
        }
        _ => admission_source_report(source),
    }
}
// END C9 retained config observations

// BEGIN C8 borrowed Source admission
enum AdmissionChain<'a, 'scope, 'attempt> {
    Legacy(&'a ProtectedSourceChain),
    Checked(&'a CheckedProtectedSourceChain<'scope, 'attempt>),
}
impl AdmissionChain<'_, '_, '_> {
    fn first(&self) -> Result<&Value, AdmissionSource> {
        match self {
            Self::Legacy(source) => Ok(source.first()),
            Self::Checked(source) => source.first().map_err(AdmissionSource::from),
        }
    }
    fn event(&self, kind: &str) -> Result<&Value, AdmissionSource> {
        match self {
            Self::Legacy(source) => source.event(kind).map_err(AdmissionSource::from),
            Self::Checked(source) => source.event(kind).map_err(AdmissionSource::from),
        }
    }
    fn revalidate(&self) -> Result<(), AdmissionSource> {
        match self {
            Self::Legacy(source) => source.revalidate().map_err(AdmissionSource::from),
            Self::Checked(source) => source.revalidate().map_err(AdmissionSource::from),
        }
    }
}
impl From<CheckedSourceFailure> for AdmissionSource {
    fn from(source: CheckedSourceFailure) -> Self {
        Self::Source(Box::new(source))
    }
}
type NativeLocalCompletion = LocalCompletion<CheckedCommittedEvidenceObservation, AdmissionSource>;
/// Real local observations; no terminal result exists in this carrier.
pub struct LocalObservedAdmission {
    completion: Box<NativeLocalCompletion>,
}
pub struct LocalObservedAdmissionFailure {
    completion: Box<NativeLocalCompletion>,
}
pub struct LocalObservedAdmissionView<'a> {
    completion: &'a NativeLocalCompletion,
}
impl LocalObservedAdmission {
    pub fn diagnostics(&self) -> LocalObservedAdmissionView<'_> {
        LocalObservedAdmissionView {
            completion: &self.completion,
        }
    }
}
impl LocalObservedAdmissionFailure {
    pub fn diagnostics(&self) -> LocalObservedAdmissionView<'_> {
        LocalObservedAdmissionView {
            completion: &self.completion,
        }
    }
}
impl LocalObservedAdmissionView<'_> {
    pub fn daily_evidence(&self) -> Option<AdmissionDailyView<'_>> {
        admission_daily_view(
            self.completion.observations.daily.as_ref(),
            &self.completion.operation,
        )
    }

    pub fn observation(
        &self,
        channel: AdmissionChannel,
    ) -> Option<&CheckedCommittedEvidenceObservation> {
        self.completion.observations.get(channel)
    }
    pub fn first_kind(&self) -> Option<AdmissionFirstKind> {
        admission_first_kind(&self.completion.operation)
    }
    pub fn source_failure(&self) -> Option<&CheckedSourceFailure> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Source(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn current_failure(&self) -> Option<&CheckedCurrentFailure> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Current(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn observation_failure(&self) -> Option<&CheckedCommittedEvidenceObservationFailure> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Observation { failure, .. }) => Some(failure),
            _ => None,
        }
    }
    pub fn config_evidence(&self) -> Option<AdmissionConfigView<'_>> {
        self.completion
            .observations
            .config
            .as_ref()
            .map(|observed| AdmissionConfigView { observed })
    }
    pub fn config_ingest_failure(
        &self,
    ) -> Option<&crate::owner_beta::runner::daily_ingest::IngestError> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Config(source)) => source.ingest(),
            _ => None,
        }
    }

    pub fn historical_failure(&self) -> Option<&CheckedHistoricalApprovalFailure> {
        match original_historical_source(&self.completion.operation) {
            Some(AdmissionSource::Historical(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn historical_use_failure(&self) -> Option<&HistoricalUseFailure> {
        admission_historical_use(&self.completion.operation)
    }
    pub fn historical_replay_result(&self) -> Option<&Result<(), DailyReplayError>> {
        match original_historical_source(&self.completion.operation) {
            Some(AdmissionSource::HistoricalUse { replay, .. }) => replay.as_ref(),
            _ => None,
        }
    }
    pub fn historical_completion(
        &self,
    ) -> Option<&Result<CheckedHistoricalApprovalCompletion, CheckedHistoricalApprovalFailure>>
    {
        self.completion.observations.historical.as_ref()
    }
    pub fn raw_evidence(&self) -> Option<AdmissionRawView<'_>> {
        admission_raw_view(
            self.completion.observations.raw.as_deref(),
            &self.completion.operation,
        )
    }
    pub fn current_completion(&self) -> Option<&CheckedCurrentCompletion> {
        self.completion.observations.current.as_ref()
    }
    pub fn source_report(&self) -> Option<&Report> {
        match &self.completion.operation {
            Some(Err(source)) => {
                admission_report_with_config(source, self.completion.observations.config.as_ref())
            }
            Some(Ok(())) => None,
            None => self
                .completion
                .expected_scope
                .as_ref()
                .err()
                .or_else(|| self.completion.captured_scope.as_ref().err())
                .map(Failure::report),
        }
    }
    pub fn policy_failure(&self) -> Option<&EvidenceError> {
        admission_policy_failure(&self.completion.operation)
    }
    pub fn context_failure(&self) -> Option<&PaperEligibilityReasonCode> {
        match self
            .completion
            .operation
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .map(original_admission_source)
        {
            Some(AdmissionSource::Context(source)) => Some(source),
            Some(AdmissionSource::Config(source)) => source.reason(),
            _ => None,
        }
    }
    pub fn expected_scope(&self) -> &UnitSettlement {
        &self.completion.expected_scope
    }
    pub fn captured_scope(&self) -> &UnitSettlement {
        &self.completion.captured_scope
    }
    pub fn local_settlement(&self) -> &UnitSettlement {
        &self.completion.local
    }
}
impl fmt::Debug for LocalObservedAdmissionView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalObservedAdmission")
            .field("first_kind", &self.first_kind())
            .field("source_report_present", &self.source_report().is_some())
            .field("local_failed", &self.local_settlement().is_err())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for LocalObservedAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostics().fmt(f)
    }
}
impl fmt::Debug for LocalObservedAdmissionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostics().fmt(f)
    }
}
fn admission_first_kind(
    operation: &Option<Result<(), AdmissionSource>>,
) -> Option<AdmissionFirstKind> {
    match operation {
        None => Some(AdmissionFirstKind::Scope),
        Some(Ok(())) => None,
        Some(Err(source)) => admission_source_kind(source),
    }
}
fn admission_source_kind(source: &AdmissionSource) -> Option<AdmissionFirstKind> {
    match source {
        AdmissionSource::Execution { .. } => Some(AdmissionFirstKind::Execution),
        AdmissionSource::Daily(source) => source.kind(),
        AdmissionSource::Historical(failure) => historical_failure_kind(failure),
        AdmissionSource::HistoricalUse { failure, .. } => historical_use_kind(failure),
        AdmissionSource::Raw(raw) => raw.first_kind(),
        AdmissionSource::Source(failure) => {
            failure.diagnostics().source_kind().map(|kind| match kind {
                SourceChainSourceKind::Policy => AdmissionFirstKind::Policy,
                SourceChainSourceKind::Scope => AdmissionFirstKind::Scope,
            })
        }
        AdmissionSource::Current(failure) => {
            failure.diagnostics().source_kind().map(|kind| match kind {
                CurrentSourceKind::Policy => AdmissionFirstKind::Policy,
                CurrentSourceKind::Scope => AdmissionFirstKind::Scope,
            })
        }
        AdmissionSource::Config(failure) => match failure.kind() {
            super::ConfigFirstKind::Policy => Some(AdmissionFirstKind::Context),
            super::ConfigFirstKind::Scope => Some(AdmissionFirstKind::Scope),
            super::ConfigFirstKind::Late => None,
        },
        AdmissionSource::Policy(_) => Some(AdmissionFirstKind::Policy),
        AdmissionSource::Context(_) => Some(AdmissionFirstKind::Context),
        AdmissionSource::Observation { channel, .. } => {
            Some(AdmissionFirstKind::Observation(*channel))
        }
    }
}
fn admission_execution_failure(source: &AdmissionSource) -> Option<&ExecutionGateFailure> {
    match source {
        AdmissionSource::Execution { failure } => Some(failure),
        AdmissionSource::Daily(source) => source.execution_failure(),
        AdmissionSource::Historical(failure) => failure.execution_failure(),
        AdmissionSource::HistoricalUse { failure, .. } => failure.checks().execution_failure(),
        AdmissionSource::Raw(raw) => raw
            .operation
            .as_ref()
            .err()
            .and_then(RawBodySource::execution_failure),
        AdmissionSource::Observation { failure, .. } => failure.source().execution_failure(),
        _ => None,
    }
}
fn admission_source_report(source: &AdmissionSource) -> Option<&Report> {
    match source {
        AdmissionSource::Execution { failure, .. } => failure.source_report(),
        AdmissionSource::Daily(source) => source.report(),
        AdmissionSource::Historical(failure) => historical_failure_report(failure),
        AdmissionSource::HistoricalUse { failure, .. } => historical_use_report(failure),
        AdmissionSource::Raw(raw) => raw.source_report(),
        AdmissionSource::Source(failure) => failure.diagnostics().source_report(),
        AdmissionSource::Current(failure) => failure.diagnostics().source_report(),
        AdmissionSource::Observation { failure, .. } => failure.source().source_report(),
        AdmissionSource::Config(_) => None,
        AdmissionSource::Policy(_) | AdmissionSource::Context(_) => None,
    }
}
fn admission_policy_failure(
    operation: &Option<Result<(), AdmissionSource>>,
) -> Option<&EvidenceError> {
    match operation {
        Some(Err(source)) => match original_admission_source(source) {
            AdmissionSource::Raw(raw) => {
                raw.operation.as_ref().err().and_then(RawBodySource::policy)
            }
            AdmissionSource::Source(failure) => failure.diagnostics().policy_source(),
            AdmissionSource::Current(failure) => failure.diagnostics().policy_source(),
            AdmissionSource::Policy(source) => Some(source),
            _ => None,
        },
        _ => None,
    }
}
/// Uses the caller's live SourceChain scope; completion is strictly local.
pub fn verify_admission_observed_in(
    source: &CheckedProtectedSourceChain<'_, '_>,
    pins: &DailyPricePinsV1Wire,
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
) -> Result<LocalObservedAdmission, LocalObservedAdmissionFailure> {
    // This real check visits all Source owners even when an earlier scope is dirty.
    let preflight = source
        .check_scope_in(invocation, expected, captured)
        .map_err(AdmissionSource::from);
    let completion = Box::new(complete_admission_local(
        invocation,
        expected,
        captured,
        preflight,
        |invocation, observations| {
            verify_policy(
                &AdmissionChain::Checked(source),
                pins,
                invocation,
                expected,
                captured,
                observations,
            )
        },
    ));
    if completion.is_clean() {
        Ok(LocalObservedAdmission { completion })
    } else {
        Err(LocalObservedAdmissionFailure { completion })
    }
}
// END C8 borrowed Source admission

// BEGIN C12 Raw pair custody
struct RawPairCompletion<T, E, C, D> {
    operation: Result<T, E>,
    kis: Option<C>,
    calendar: Option<C>,
    lineage: Vec<D>,
}
#[cfg(test)]
fn capture_raw_pair<S, E>(
    slots: &mut [Option<S>; 2],
    read: impl FnMut(AdmissionChannel) -> Result<S, E>,
) -> Result<(), E> {
    capture_raw_pair_with_checkpoint(slots, read, || Ok::<(), E>(()))
}
fn capture_raw_pair_with_checkpoint<S, E>(
    slots: &mut [Option<S>; 2],
    mut read: impl FnMut(AdmissionChannel) -> Result<S, E>,
    mut checkpoint: impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    for channel in [AdmissionChannel::Kis, AdmissionChannel::KisCalendar] {
        checkpoint()?;
        let original = read(channel);
        slots[channel.index()] = Some(original?);
        checkpoint()?;
    }
    Ok(())
}
fn retain_raw_use<T, D, E>(
    value: T,
    destination: &mut Vec<D>,
    inspect: impl FnOnce(&T) -> Result<(), E>,
    retain: impl FnOnce(T) -> D,
) -> Result<(), E> {
    let original = inspect(&value);
    destination.push(retain(value));
    original
}
fn finish_raw_pair<S, T, E, C, D>(
    original: Result<T, E>,
    mut slots: [Option<S>; 2],
    lineage: Vec<D>,
    mut finish: impl FnMut(S) -> C,
) -> RawPairCompletion<T, E, C, D> {
    // Each lower graph has its own lock-last discipline. Only lexical pair order here.
    let calendar = slots[AdmissionChannel::KisCalendar.index()]
        .take()
        .map(&mut finish);
    let kis = slots[AdmissionChannel::Kis.index()].take().map(&mut finish);
    RawPairCompletion {
        operation: original,
        kis,
        calendar,
        lineage,
    }
}
impl<T, E, C, D> RawPairCompletion<T, E, C, D> {
    fn source_report_with<'s>(
        &'s self,
        report: impl FnOnce(&'s E) -> Option<&'s Report>,
    ) -> Option<&'s Report> {
        self.operation.as_ref().err().and_then(report)
    }
    fn is_clean_with(&self, clean: impl Fn(&C) -> bool) -> bool {
        self.operation.is_ok()
            & self.calendar.as_ref().is_none_or(&clean)
            & self.kis.as_ref().is_none_or(&clean)
    }
}

fn retain_raw_completion<T, E, C, D>(
    completion: RawPairCompletion<T, E, C, D>,
    clean: impl Fn(&C) -> bool,
    destination: &mut Option<Box<RawPairCompletion<T, E, C, D>>>,
) -> Result<(), Box<RawPairCompletion<T, E, C, D>>> {
    let completion = Box::new(completion);
    if completion.is_clean_with(clean) {
        *destination = Some(completion);
        Ok(())
    } else {
        Err(completion)
    }
}
impl<T, E, C, D> fmt::Debug for RawPairCompletion<T, E, C, D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawPairCompletion")
            .field("original_ok", &self.operation.is_ok())
            .field("kis_present", &self.kis.is_some())
            .field("calendar_present", &self.calendar.is_some())
            .field("lineage_count", &self.lineage.len())
            .finish_non_exhaustive()
    }
}

enum RawBodySource {
    Snapshot {
        channel: AdmissionChannel,
        failure: Box<CheckedRawSnapshotFailure>,
    },
    Lineage(Box<CheckedLineageFailure>),
    Prior(Box<AdmissionSource>),
}
impl From<AdmissionSource> for RawBodySource {
    fn from(source: AdmissionSource) -> Self {
        Self::Prior(Box::new(source))
    }
}
impl From<EvidenceError> for RawBodySource {
    fn from(source: EvidenceError) -> Self {
        AdmissionSource::from(source).into()
    }
}
impl From<CheckedLineageFailure> for RawBodySource {
    fn from(source: CheckedLineageFailure) -> Self {
        Self::Lineage(Box::new(source))
    }
}
impl RawBodySource {
    fn execution_failure(&self) -> Option<&ExecutionGateFailure> {
        match self {
            Self::Snapshot { failure, .. } => failure.execution_failure(),
            Self::Prior(source) => admission_execution_failure(source),
            Self::Lineage(_) => None,
        }
    }

    fn kind(&self) -> Option<AdmissionFirstKind> {
        match self {
            Self::Snapshot { failure, .. } => Some(match failure.kind() {
                SnapshotFirstKind::Policy => AdmissionFirstKind::Policy,
                SnapshotFirstKind::Scope | SnapshotFirstKind::Binding => AdmissionFirstKind::Scope,
                SnapshotFirstKind::Execution => AdmissionFirstKind::Execution,
            }),
            Self::Lineage(failure) => failure.first_kind().map(|kind| match kind {
                CheckedLineageFirstKind::Policy => AdmissionFirstKind::Policy,
                CheckedLineageFirstKind::Scope => AdmissionFirstKind::Scope,
            }),
            Self::Prior(source) => admission_source_kind(source),
        }
    }
    fn report(&self) -> Option<&Report> {
        match self {
            Self::Snapshot { failure, .. } => failure.source_report(),
            Self::Lineage(failure) => failure.source_report(),
            Self::Prior(source) => admission_source_report(source),
        }
    }
    fn policy(&self) -> Option<&EvidenceError> {
        match self {
            Self::Prior(source) => match original_admission_source(source) {
                AdmissionSource::Policy(error) => Some(error),
                AdmissionSource::Source(failure) => failure.diagnostics().policy_source(),
                AdmissionSource::Current(failure) => failure.diagnostics().policy_source(),
                _ => None,
            },
            _ => None,
        }
    }
}
type NativeRawCompletion =
    RawPairCompletion<(), RawBodySource, CheckedRawSnapshotCompletion, CheckedLineageData>;
impl NativeRawCompletion {
    fn is_clean(&self) -> bool {
        self.is_clean_with(CheckedRawSnapshotCompletion::is_clean)
    }
    fn first_kind(&self) -> Option<AdmissionFirstKind> {
        self.operation.as_ref().err().and_then(RawBodySource::kind)
    }
    fn source_report(&self) -> Option<&Report> {
        // A successful operation followed only by late cleanup has no first error.
        self.source_report_with(RawBodySource::report)
    }
}

/// Real retained Raw outcomes only, never a lookup or binding capability.
pub struct AdmissionRawView<'a> {
    completion: &'a NativeRawCompletion,
}
impl AdmissionRawView<'_> {
    pub fn original_succeeded(&self) -> bool {
        self.completion.operation.is_ok()
    }
    pub fn first_kind(&self) -> Option<AdmissionFirstKind> {
        self.completion.first_kind()
    }
    pub fn source_report(&self) -> Option<&Report> {
        self.completion.source_report()
    }
    pub fn snapshot_failure(
        &self,
        channel: AdmissionChannel,
    ) -> Option<&CheckedRawSnapshotFailure> {
        match &self.completion.operation {
            Err(RawBodySource::Snapshot {
                channel: actual,
                failure,
            }) if *actual == channel => Some(failure),
            _ => None,
        }
    }
    pub fn lineage_failure(&self) -> Option<&CheckedLineageFailure> {
        match &self.completion.operation {
            Err(RawBodySource::Lineage(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn snapshot_completion(
        &self,
        channel: AdmissionChannel,
    ) -> Option<&CheckedRawSnapshotCompletion> {
        match channel {
            AdmissionChannel::Kis => self.completion.kis.as_ref(),
            AdmissionChannel::KisCalendar => self.completion.calendar.as_ref(),
        }
    }
    pub fn retained_lineage(&self) -> &[CheckedLineageData] {
        &self.completion.lineage
    }
    pub fn is_clean(&self) -> bool {
        self.completion.is_clean()
    }
}
impl fmt::Debug for AdmissionRawView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmissionRawView")
            .field("first_kind", &self.first_kind())
            .field("original_ok", &self.original_succeeded())
            .field("lineage_count", &self.completion.lineage.len())
            .field("clean", &self.is_clean())
            .finish_non_exhaustive()
    }
}
// Borrow the same original object through the new whole Raw outcome wrapper.
fn original_admission_source(source: &AdmissionSource) -> &AdmissionSource {
    match source {
        AdmissionSource::Raw(raw) => match &raw.operation {
            Err(RawBodySource::Prior(source)) => original_admission_source(source),
            _ => source,
        },
        _ => source,
    }
}
fn admission_raw_view<'a>(
    saved: Option<&'a NativeRawCompletion>,
    operation: &'a Option<Result<(), AdmissionSource>>,
) -> Option<AdmissionRawView<'a>> {
    fn from_source(source: &AdmissionSource) -> Option<&NativeRawCompletion> {
        match source {
            AdmissionSource::Raw(completion) => Some(completion),
            _ => None,
        }
    }
    let completion = operation
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .and_then(from_source)
        .or(saved)?;
    Some(AdmissionRawView { completion })
}
// END C12 Raw pair custody

// BEGIN C13 historical consumer custody
type HistoricalCompletion =
    Result<CheckedHistoricalApprovalCompletion, CheckedHistoricalApprovalFailure>;

fn retain_historical_acquisition<H, E>(
    original: Result<H, E>,
    destination: &mut Option<H>,
) -> Result<(), E> {
    *destination = Some(original?);
    Ok(())
}
fn retain_historical_completion<T, E, H, C>(
    original: Result<T, E>,
    held: Option<H>,
    finish: impl FnOnce(H) -> C,
    destination: &mut Option<C>,
) -> Result<T, E> {
    if let Some(held) = held {
        *destination = Some(finish(held));
    }
    original
}
fn historical_checkpoint(
    historical: &CheckedHistoricalApproval<'_>,
    expected: &Attempt,
    captured: &Attempt,
) -> Result<(), AdmissionSource> {
    historical
        .approved_in(expected, captured)
        .map(|_| ())
        .map_err(|failure| AdmissionSource::HistoricalUse {
            failure: Box::new(failure),
            replay: None,
        })
}
fn checked_historical_unit<E>(
    mut check: impl FnMut() -> Result<(), E>,
    operation: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    check()?;
    let original = operation();
    original?;
    check()
}
fn historical_unit_boundary<E: From<AdmissionSource>>(
    historical: &CheckedHistoricalApproval<'_>,
    expected: &Attempt,
    captured: &Attempt,
    mut daily: impl FnMut() -> Result<(), DailyPolicySource>,
    operation: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    checked_historical_unit(
        || daily_checkpoint(historical, expected, captured, &mut daily).map_err(E::from),
        operation,
    )
}
fn replay_checkpoint<U>(mut check: impl FnMut() -> Result<(), U>, first: &mut Option<U>) -> bool {
    if first.is_some() {
        return false;
    }
    match check() {
        Ok(()) => true,
        Err(source) => {
            *first = Some(source);
            false
        }
    }
}
#[cfg(test)]
enum HistoricalReplayFailure<T, U, E> {
    Use { first: U, original: Result<T, E> },
    Policy(E),
}
#[cfg(test)]
impl<T, U, E> fmt::Debug for HistoricalReplayFailure<T, U, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use { original, .. } => f
                .debug_struct("HistoricalReplayUse")
                .field("original_ok", &original.is_ok())
                .finish_non_exhaustive(),
            Self::Policy(_) => f
                .debug_struct("HistoricalReplayPolicy")
                .finish_non_exhaustive(),
        }
    }
}
#[cfg(test)]
fn retain_replay_result<T, U, E>(
    original: Result<T, E>,
    first: Option<U>,
) -> Result<T, Box<HistoricalReplayFailure<T, U, E>>> {
    match first {
        Some(first) => Err(Box::new(HistoricalReplayFailure::Use { first, original })),
        None => original.map_err(|error| Box::new(HistoricalReplayFailure::Policy(error))),
    }
}

fn historical_kind(
    kind: Option<HistoricalApprovalFirstKind>,
    nested: Option<HistoricalV3FirstKind>,
) -> Option<AdmissionFirstKind> {
    match kind {
        Some(HistoricalApprovalFirstKind::Approval) => Some(AdmissionFirstKind::Policy),
        Some(HistoricalApprovalFirstKind::Scope | HistoricalApprovalFirstKind::Binding) => {
            Some(AdmissionFirstKind::Scope)
        }
        Some(HistoricalApprovalFirstKind::Artifact) => match nested {
            Some(HistoricalV3FirstKind::Artifact) => Some(AdmissionFirstKind::Policy),
            Some(HistoricalV3FirstKind::Scope | HistoricalV3FirstKind::Binding) => {
                Some(AdmissionFirstKind::Scope)
            }
            Some(HistoricalV3FirstKind::Late) | None => None,
        },
        Some(HistoricalApprovalFirstKind::Late) | None => None,
    }
}
fn historical_failure_kind(
    failure: &CheckedHistoricalApprovalFailure,
) -> Option<AdmissionFirstKind> {
    historical_kind(
        failure.first_kind(),
        failure.artifact_failure().and_then(|f| f.first_kind()),
    )
}
fn historical_failure_report(failure: &CheckedHistoricalApprovalFailure) -> Option<&Report> {
    if failure.primary_execution_failure().is_none() {
        historical_failure_kind(failure)?;
    }
    failure.first_report()
}
fn historical_use_kind(failure: &HistoricalUseFailure) -> Option<AdmissionFirstKind> {
    let checks = failure.checks();
    if checks.expected().is_ok()
        && checks.captured().is_ok()
        && checks.execution_failure().is_some()
    {
        None
    } else {
        Some(AdmissionFirstKind::Scope)
    }
}
fn historical_use_report(failure: &HistoricalUseFailure) -> Option<&Report> {
    failure.checks().source_report()
}

fn original_historical_source(
    operation: &Option<Result<(), AdmissionSource>>,
) -> Option<&AdmissionSource> {
    operation
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .map(original_admission_source)
}
// END C13 historical consumer custody

// BEGIN C16 checked Daily consumer custody
fn run_owned_region<H, T, E, C>(
    held: H,
    body: impl for<'borrow> FnOnce(&'borrow H) -> Result<T, E>,
    before_finish: impl FnOnce(&Result<T, E>),
    finish: impl FnOnce(H) -> C,
) -> (Result<T, E>, C) {
    let original = body(&held);
    before_finish(&original);
    let completion = finish(held);
    (original, completion)
}
fn mark_region_failure<T, E>(captured: &Attempt, original: &Result<T, E>) {
    if original.is_err() && captured.check() {
        captured.gap(Gap::Abandoned);
    }
}

// Successful H never escapes inside E. The unexpected saved-use + Ok case
// consumes the actual H and retains its actual completion, not a unit projection.
enum DailyReplayIntakeFailure<U, E, C> {
    Use { first: U, original: Result<C, E> },
    Policy(E),
}
fn intake_daily_replay<H, U, E, C>(
    original: Result<H, E>,
    first: Option<U>,
    finish: impl FnOnce(H) -> C,
) -> Result<H, DailyReplayIntakeFailure<U, E, C>> {
    match (first, original) {
        (None, Ok(held)) => Ok(held),
        (None, Err(error)) => Err(DailyReplayIntakeFailure::Policy(error)),
        (Some(first), Err(error)) => Err(DailyReplayIntakeFailure::Use {
            first,
            original: Err(error),
        }),
        (Some(first), Ok(held)) => {
            let completion = finish(held);
            Err(DailyReplayIntakeFailure::Use {
                first,
                original: Ok(completion),
            })
        }
    }
}
impl<U, E, C> fmt::Debug for DailyReplayIntakeFailure<U, E, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(_) => f.debug_struct("DailyReplayPolicy").finish_non_exhaustive(),
            Self::Use { original, .. } => f
                .debug_struct("DailyReplayUse")
                .field("original_ok", &original.is_ok())
                .finish_non_exhaustive(),
        }
    }
}

struct JoinedDailyUse<H, D> {
    historical: Result<(), H>,
    daily: Result<(), D>,
}
fn join_daily_checks<H, D>(
    historical: impl FnOnce() -> Result<(), H>,
    daily: impl FnOnce() -> Result<(), D>,
) -> Result<(), JoinedDailyUse<H, D>> {
    let historical = historical();
    let daily = daily();
    if historical.is_ok() && daily.is_ok() {
        Ok(())
    } else {
        Err(JoinedDailyUse { historical, daily })
    }
}
impl<H, D> JoinedDailyUse<H, D> {
    // Some(None) is a chosen absent first report; it never searches later.
    fn first_with<'a, T>(
        &'a self,
        historical: impl FnOnce(&'a H) -> T,
        daily: impl FnOnce(&'a D) -> T,
    ) -> Option<T> {
        if let Err(first) = &self.historical {
            Some(historical(first))
        } else {
            self.daily.as_ref().err().map(daily)
        }
    }
}
fn daily_checkpoint(
    historical: &CheckedHistoricalApproval<'_>,
    expected: &Attempt,
    captured: &Attempt,
    daily: impl FnOnce() -> Result<(), DailyPolicySource>,
) -> Result<(), AdmissionSource> {
    join_daily_checks(
        || historical.approved_in(expected, captured).map(|_| ()),
        daily,
    )
    .map_err(|joined| AdmissionSource::Daily(Box::new(DailySource::Joined(joined))))
}

struct DailyRetention<R, C> {
    root: Option<R>,
    chain: Option<C>,
}
impl<R, C> DailyRetention<R, C> {
    fn new() -> Self {
        Self {
            root: None,
            chain: None,
        }
    }
    fn check(
        &self,
        root_clean: impl FnOnce(&R) -> bool,
        chain_clean: impl FnOnce(&C) -> bool,
    ) -> bool {
        let root = self.root.as_ref().is_none_or(root_clean);
        let chain = self.chain.as_ref().is_none_or(chain_clean);
        root && chain
    }
}
type NativeDailyRetention = DailyRetention<CheckedDailyCompletion, CheckedDailyPolicyCompletion>;
impl NativeDailyRetention {
    fn is_clean(&self) -> bool {
        self.check(
            CheckedDailyCompletion::is_clean,
            CheckedDailyPolicyCompletion::is_clean,
        )
    }
}
type NativeDailyReplayIntake = DailyReplayIntakeFailure<
    HistoricalUseFailure,
    CheckedDailyPolicyFailure,
    CheckedDailyPolicyCompletion,
>;
enum DailySource {
    Root(Box<CheckedDailyArtifactFailure>),
    Replay(NativeDailyReplayIntake),
    Use(DailyPolicySource),
    Joined(JoinedDailyUse<HistoricalUseFailure, DailyPolicySource>),
}
fn daily_kind(kind: Option<DailyPolicyFirstKind>) -> Option<AdmissionFirstKind> {
    kind.map(|kind| match kind {
        DailyPolicyFirstKind::Scope => AdmissionFirstKind::Scope,
        DailyPolicyFirstKind::Artifact
        | DailyPolicyFirstKind::Policy
        | DailyPolicyFirstKind::Replay => AdmissionFirstKind::Policy,
    })
}
fn daily_root_kind(kind: Option<DailyArtifactFirstKind>) -> Option<AdmissionFirstKind> {
    kind.map(|kind| match kind {
        DailyArtifactFirstKind::Scope | DailyArtifactFirstKind::Binding => {
            AdmissionFirstKind::Scope
        }
        DailyArtifactFirstKind::Artifact => AdmissionFirstKind::Policy,
    })
}
impl DailySource {
    fn execution_failure(&self) -> Option<&ExecutionGateFailure> {
        match self {
            Self::Root(f) => f.execution_failure(),
            Self::Replay(DailyReplayIntakeFailure::Policy(f)) => f.execution_failure(),
            Self::Replay(DailyReplayIntakeFailure::Use { first, .. }) => {
                first.checks().execution_failure()
            }
            Self::Use(source) => source.execution_failure(),
            Self::Joined(joined) => joined
                .historical
                .as_ref()
                .err()
                .and_then(|f| f.checks().execution_failure())
                .or_else(|| {
                    joined
                        .daily
                        .as_ref()
                        .err()
                        .and_then(DailyPolicySource::execution_failure)
                }),
        }
    }

    fn kind(&self) -> Option<AdmissionFirstKind> {
        match self {
            Self::Root(failure) => daily_root_kind(failure.first_kind()),
            Self::Replay(DailyReplayIntakeFailure::Policy(failure)) => {
                daily_kind(failure.first_kind())
            }
            Self::Replay(DailyReplayIntakeFailure::Use { first, .. }) => historical_use_kind(first),
            Self::Use(source) => daily_kind(source.first_kind()),
            Self::Joined(joined) => joined
                .first_with(historical_use_kind, |source| {
                    daily_kind(source.first_kind())
                })
                .flatten(),
        }
    }
    fn report(&self) -> Option<&Report> {
        match self {
            Self::Root(failure) => failure.source_report(),
            Self::Replay(DailyReplayIntakeFailure::Policy(failure)) => failure.source_report(),
            Self::Replay(DailyReplayIntakeFailure::Use { first, .. }) => {
                historical_use_report(first)
            }
            Self::Use(source) => source.report(),
            Self::Joined(joined) => joined
                .first_with(historical_use_report, DailyPolicySource::report)
                .flatten(),
        }
    }
    fn historical_use(&self) -> Option<&HistoricalUseFailure> {
        match self {
            Self::Replay(DailyReplayIntakeFailure::Use { first, .. }) => Some(first),
            Self::Replay(DailyReplayIntakeFailure::Policy(failure)) => {
                failure.source().and_then(daily_historical_use)
            }
            Self::Use(source) => daily_historical_use(source),
            Self::Joined(joined) => joined
                .historical
                .as_ref()
                .err()
                .or_else(|| joined.daily.as_ref().err().and_then(daily_historical_use)),
            Self::Root(_) => None,
        }
    }
}
impl From<DailyPolicySource> for AdmissionSource {
    fn from(source: DailyPolicySource) -> Self {
        Self::Daily(Box::new(DailySource::Use(source)))
    }
}
// This is a typed diagnostic traversal, not first-report selection. It borrows
// the real historical error, including one stored in a later joined check.
fn daily_historical_use(source: &DailyPolicySource) -> Option<&HistoricalUseFailure> {
    match source {
        DailyPolicySource::Use(joined) => joined.historical().as_ref().err(),
        DailyPolicySource::Prior(failure) => failure.source().and_then(daily_historical_use),
        DailyPolicySource::Joined(pair) => pair
            .0
            .as_ref()
            .err()
            .and_then(daily_historical_use)
            .or_else(|| pair.1.as_ref().err().and_then(daily_historical_use)),
        _ => None,
    }
}
fn admission_historical_use(
    operation: &Option<Result<(), AdmissionSource>>,
) -> Option<&HistoricalUseFailure> {
    match original_historical_source(operation) {
        Some(AdmissionSource::HistoricalUse { failure, .. }) => Some(failure),
        Some(AdmissionSource::Daily(source)) => source.historical_use(),
        _ => None,
    }
}

/// Borrowed real Daily errors and completed data; not an admission factory.
pub struct AdmissionDailyView<'a> {
    retained: Option<&'a NativeDailyRetention>,
    source: Option<&'a DailySource>,
}
impl AdmissionDailyView<'_> {
    pub fn first_kind(&self) -> Option<AdmissionFirstKind> {
        self.source.and_then(DailySource::kind)
    }
    pub fn source_report(&self) -> Option<&Report> {
        self.source.and_then(DailySource::report)
    }
    pub fn root_failure(&self) -> Option<&CheckedDailyArtifactFailure> {
        match self.source {
            Some(DailySource::Root(failure)) => Some(failure),
            _ => None,
        }
    }
    pub fn replay_failure(&self) -> Option<&CheckedDailyPolicyFailure> {
        match self.source {
            Some(DailySource::Replay(DailyReplayIntakeFailure::Policy(failure))) => Some(failure),
            Some(DailySource::Replay(DailyReplayIntakeFailure::Use {
                original: Err(failure),
                ..
            })) => Some(failure),
            _ => None,
        }
    }
    pub fn use_failure(&self) -> Option<&DailyPolicySource> {
        match self.source {
            Some(DailySource::Use(source)) => Some(source),
            Some(DailySource::Joined(joined)) => joined.daily.as_ref().err(),
            _ => self
                .replay_failure()
                .and_then(CheckedDailyPolicyFailure::source),
        }
    }
    pub fn historical_use_failure(&self) -> Option<&HistoricalUseFailure> {
        self.source.and_then(DailySource::historical_use)
    }
    pub fn callback_replay_result(
        &self,
    ) -> Option<&Result<CheckedDailyPolicyCompletion, CheckedDailyPolicyFailure>> {
        match self.source {
            Some(DailySource::Replay(DailyReplayIntakeFailure::Use { original, .. })) => {
                Some(original)
            }
            _ => None,
        }
    }
    pub fn root_completion(&self) -> Option<&CheckedDailyCompletion> {
        self.retained.and_then(|r| r.root.as_ref())
    }
    pub fn chain_completion(&self) -> Option<&CheckedDailyPolicyCompletion> {
        self.retained
            .and_then(|r| r.chain.as_ref())
            .or_else(|| self.callback_replay_result().and_then(|r| r.as_ref().ok()))
    }
    pub fn replay_original_ok(&self) -> Option<bool> {
        if let Some(original) = self.callback_replay_result() {
            return Some(original.is_ok());
        }
        if self.replay_failure().is_some() {
            return Some(false);
        }
        self.retained.and_then(|r| r.chain.as_ref()).map(|_| true)
    }
}
impl fmt::Debug for AdmissionDailyView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmissionDaily")
            .field("first_kind", &self.first_kind())
            .field("root_completed", &self.root_completion().is_some())
            .field("chain_completed", &self.chain_completion().is_some())
            .field("replay_original_ok", &self.replay_original_ok())
            .finish_non_exhaustive()
    }
}
fn admission_daily_view<'a>(
    retained: Option<&'a NativeDailyRetention>,
    operation: &'a Option<Result<(), AdmissionSource>>,
) -> Option<AdmissionDailyView<'a>> {
    let source = operation
        .as_ref()
        .and_then(|r| r.as_ref().err())
        .map(original_admission_source)
        .and_then(|source| match source {
            AdmissionSource::Daily(source) => Some(source.as_ref()),
            _ => None,
        });
    if retained.is_none() && source.is_none() {
        None
    } else {
        Some(AdmissionDailyView { retained, source })
    }
}
// END C16 checked Daily consumer custody

// BEGIN C16 production-shared tests

#[cfg(test)]
mod daily_consumer_tests {
    use super::*;
    use market_data::checked_close::{Close, Errno, Owner, Role};
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };
    struct Data(Box<str>);
    struct ErrorBody(Box<str>);
    struct Counted {
        name: &'static str,
        journal: Rc<RefCell<Vec<&'static str>>>,
        known: bool,
    }
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.journal.borrow_mut().push(self.name);
            if self.known {
                Ok(())
            } else {
                Err(Errno::BadDescriptor)
            }
        }
    }
    struct Holder {
        owner: Owner<Counted>,
        value: Data,
    }
    struct Completed {
        value: Data,
        clean: bool,
    }
    fn holder(
        scope: &Attempt,
        journal: &Rc<RefCell<Vec<&'static str>>>,
        name: &'static str,
        known: bool,
    ) -> Holder {
        Holder {
            owner: scope.adopt(
                Counted {
                    name,
                    journal: Rc::clone(journal),
                    known,
                },
                Role::Source,
            ),
            value: Data(name.into()),
        }
    }
    fn finish(held: Holder) -> Completed {
        let Holder { owner, value } = held;
        // This bool is aggregate ledger state, not an isolated native-close proof.
        let clean = owner.finish();
        Completed { value, clean }
    }

    #[test]
    fn nested_owned_regions_finish_same_child_then_root_with_outer_live() {
        let scope = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = holder(&scope, &journal, "outer", true);
        let root = holder(&scope, &journal, "root", true);
        let root_address = root.value.0.as_ptr();
        let (original, root_completion) = run_owned_region(
            root,
            |root| {
                assert_eq!(root.value.0.as_ptr(), root_address);
                let chain = holder(&scope, &journal, "chain", true);
                let address = chain.value.0.as_ptr();
                let (result, completion) = run_owned_region(
                    chain,
                    |chain| {
                        assert_eq!(chain.value.0.as_ptr(), address);
                        Ok::<_, ErrorBody>(Data("returned policy value".into()))
                    },
                    |original| mark_region_failure(&scope, original),
                    finish,
                );
                assert_eq!(completion.value.0.as_ptr(), address);
                assert!(completion.clean);
                result
            },
            |original| mark_region_failure(&scope, original),
            finish,
        );
        assert_eq!(original.ok().unwrap().0.as_ref(), "returned policy value");
        assert_eq!(root_completion.value.0.as_ptr(), root_address);
        assert!(root_completion.clean);
        assert_eq!(&*journal.borrow(), &["chain", "root"]);
        assert_eq!(scope.report().live(), 1);
        assert!(finish(outer).clean);
        assert_eq!(&*journal.borrow(), &["chain", "root", "outer"]);
    }

    #[test]
    fn every_owned_early_error_keeps_original_some_and_none_before_completion() {
        for has_body in [true, false] {
            let scope = Attempt::default();
            let invocation = RawInvocation::new(&scope);
            let journal = Rc::new(RefCell::new(Vec::new()));
            let held = holder(&scope, &journal, "root", false);
            let failure = if has_body {
                invocation
                    .settle_local_in(&scope, Err::<(), _>(ErrorBody("whole first body".into())))
                    .unwrap_err()
            } else {
                scope.gap(Gap::Unbound);
                invocation
                    .settle_local_in(&scope, Ok::<(), ErrorBody>(()))
                    .unwrap_err()
            };
            let address = failure.body().map(|body| body.0.as_ptr());
            let (original, completed) = run_owned_region(
                held,
                |_| Err::<(), _>(failure),
                |original| mark_region_failure(&scope, original),
                finish,
            );
            let first = original.unwrap_err();
            assert_eq!(first.body().map(|body| body.0.as_ptr()), address);
            assert_eq!(first.body().is_some(), has_body);
            assert_eq!(first.report().live(), 1);
            assert_eq!(first.report().unknown(), 0);
            assert!(!completed.clean);
            assert_eq!(scope.report().unknown(), 1);
            assert_eq!(&*journal.borrow(), &["root"]);
        }
    }

    #[test]
    fn replay_intake_matrix_retains_actual_ok_completion_and_error_without_double_finish() {
        let scope = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let calls = Cell::new(0);
        let held = holder(&scope, &journal, "accepted", true);
        let address = held.value.0.as_ptr();
        let accepted = intake_daily_replay(Ok::<_, ErrorBody>(held), None::<ErrorBody>, |held| {
            calls.set(calls.get() + 1);
            finish(held)
        });
        let accepted = match accepted {
            Ok(held) => held,
            Err(_) => panic!("no saved source"),
        };
        assert_eq!(calls.get(), 0);
        assert_eq!(finish(accepted).value.0.as_ptr(), address);
        let failure = ErrorBody("whole replay error".into());
        let address = failure.0.as_ptr();
        let denied =
            intake_daily_replay::<Holder, ErrorBody, _, Completed>(Err(failure), None, |_| {
                calls.set(calls.get() + 1);
                panic!("Err already completed")
            });
        match denied {
            Err(DailyReplayIntakeFailure::Policy(error)) => assert_eq!(error.0.as_ptr(), address),
            _ => panic!("whole replay error"),
        }
        let first = ErrorBody("saved callback first".into());
        let first_address = first.0.as_ptr();
        let secondary = ErrorBody("actual returned replay error".into());
        let secondary_address = secondary.0.as_ptr();
        let denied =
            intake_daily_replay::<Holder, _, _, Completed>(Err(secondary), Some(first), |_| {
                calls.set(calls.get() + 1);
                panic!("no successful holder")
            });
        match denied {
            Err(DailyReplayIntakeFailure::Use {
                first,
                original: Err(secondary),
            }) => {
                assert_eq!(first.0.as_ptr(), first_address);
                assert_eq!(secondary.0.as_ptr(), secondary_address);
            }
            _ => panic!("both actual errors"),
        }
        let held = holder(&scope, &journal, "saved-ok", true);
        let address = held.value.0.as_ptr();
        let first = ErrorBody("actual callback cause".into());
        let first_address = first.0.as_ptr();
        let denied = intake_daily_replay(Ok::<_, ErrorBody>(held), Some(first), |held| {
            calls.set(calls.get() + 1);
            scope.gap(Gap::Abandoned);
            finish(held)
        });
        match denied {
            Err(DailyReplayIntakeFailure::Use {
                first,
                original: Ok(completed),
            }) => {
                assert_eq!(first.0.as_ptr(), first_address);
                assert_eq!(completed.value.0.as_ptr(), address);
                assert!(!completed.clean);
            }
            _ => panic!("successful live holder must be explicitly completed"),
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(&*journal.borrow(), &["accepted", "saved-ok"]);
    }

    #[test]
    fn joined_dirty_foreign_scopes_are_both_visited_before_stop() {
        let actual = Attempt::default();
        let historical = Attempt::default();
        let daily = Attempt::default();
        actual.gap(Gap::Unbound);
        let invocation = RawInvocation::new(&actual);
        let visits = Cell::new(0);
        let result = join_daily_checks(
            || {
                visits.set(visits.get() + 1);
                invocation.settle_local_in(&historical, Ok::<(), Infallible>(()))
            },
            || {
                visits.set(visits.get() + 1);
                invocation.settle_local_in(&daily, Ok::<(), Infallible>(()))
            },
        );
        let downstream = Cell::new(0);
        if result.is_ok() {
            downstream.set(1);
        }
        let joined = result.unwrap_err();
        assert!(joined.historical.is_err() && joined.daily.is_err());
        assert!(!historical.check() && !daily.check());
        assert_eq!(visits.get(), 2);
        assert_eq!(downstream.get(), 0);
    }

    #[test]
    fn first_absent_report_in_join_never_falls_through_to_later_report() {
        let visited = Cell::new(0);
        let joined = join_daily_checks(
            || {
                visited.set(visited.get() + 1);
                Err(ErrorBody("first without report".into()))
            },
            || {
                visited.set(visited.get() + 1);
                Err(ErrorBody("later rich diagnostic".into()))
            },
        )
        .unwrap_err();
        let later_reports = Cell::new(0);
        let report = joined
            .first_with(
                |_| None::<&str>,
                |_| {
                    later_reports.set(later_reports.get() + 1);
                    Some("later report")
                },
            )
            .flatten();
        assert!(report.is_none());
        assert_eq!(visited.get(), 2);
        assert_eq!(later_reports.get(), 0);
    }

    #[test]
    fn actual_daily_root_none_refusal_prevents_body_replay_raw_and_link() {
        let scope = Attempt::default();
        let invocation = RawInvocation::new(&scope);
        let callbacks = Cell::new(0);
        let result = CheckedDailyAdmissionRoot::open_in(
            &invocation,
            &scope,
            &scope,
            std::path::Path::new("not-accessed-by-Daily-None"),
            10003,
            10001,
        )
        .map(|root| {
            callbacks.set(callbacks.get() + 1);
            root.finish_local()
        });
        let failure = result.unwrap_err();
        assert_eq!(failure.first_kind(), Some(DailyArtifactFirstKind::Binding));
        assert_eq!(callbacks.get(), 0);
        assert_eq!(scope.report().live(), 0);
        assert_eq!(scope.report().attempted(), 0);
    }

    #[test]
    fn daily_clean_kernel_observes_both_late_completions_and_keeps_returned_ok() {
        let scope = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let root = finish(holder(&scope, &journal, "root", false));
        let chain = finish(holder(&scope, &journal, "chain", true));
        let returned = Ok::<_, ErrorBody>(Data("actual successful body".into()));
        let address = returned.as_ref().ok().unwrap().0.as_ptr();
        let retained = DailyRetention {
            root: Some(root),
            chain: Some(chain),
        };
        let visits = Cell::new(0);
        assert!(!retained.check(
            |root| {
                visits.set(visits.get() + 1);
                root.clean
            },
            |chain| {
                visits.set(visits.get() + 1);
                chain.clean
            },
        ));
        assert_eq!(visits.get(), 2);
        assert_eq!(returned.as_ref().ok().unwrap().0.as_ptr(), address);
        assert!(returned.is_ok());
        assert_eq!(scope.report().unknown(), 1);
        assert_eq!(&*journal.borrow(), &["root", "chain"]);
    }

    fn journal_policy(raw_clean: bool) -> Vec<&'static str> {
        let scope = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let root = holder(&scope, &journal, "root", true);
        let historical = holder(&scope, &journal, "historical", true);
        let config = holder(&scope, &journal, "config", true);
        let current = holder(&scope, &journal, "current", true);
        let (original, root_completed) = run_owned_region(
            root,
            |_| {
                let chain = holder(&scope, &journal, "chain", true);
                let (original, chain_completed) = run_owned_region(
                    chain,
                    |_| {
                        let slots = [
                            Some(holder(&scope, &journal, "kis", true)),
                            Some(holder(&scope, &journal, "calendar", raw_clean)),
                        ];
                        let original = Ok::<_, ErrorBody>(());
                        let completed =
                            finish_raw_pair(original, slots, Vec::<Data>::new(), finish);
                        let mut retained = None;
                        retain_raw_completion(completed, |c| c.clean, &mut retained)
                            .map_err(|_| ErrorBody("observed Raw refusal".into()))?;
                        journal.borrow_mut().push("final-boundary");
                        Ok::<(), ErrorBody>(())
                    },
                    |original| mark_region_failure(&scope, original),
                    finish,
                );
                assert!(chain_completed.value.0.as_ref() == "chain");
                original
            },
            |original| mark_region_failure(&scope, original),
            finish,
        );
        assert_eq!(root_completed.value.0.as_ref(), "root");
        let mut historical_retained = None;
        let original = retain_historical_completion(
            original,
            Some(historical),
            finish,
            &mut historical_retained,
        );
        let config_completed = finish(config);
        assert_eq!(config_completed.value.0.as_ref(), "config");
        let current_completed = finish(current);
        let mut current_retained = None;
        let original = retain_current_result(original, current_completed, &mut current_retained);
        assert_eq!(original.is_ok(), raw_clean);
        journal.borrow().clone()
    }
    #[test]
    fn raw_calendar_kis_then_daily_chain_root_and_outer_tail_preserves_journal() {
        assert_eq!(
            journal_policy(true),
            [
                "calendar",
                "kis",
                "final-boundary",
                "chain",
                "root",
                "historical",
                "config",
                "current"
            ]
        );
    }
    #[test]
    fn dirty_raw_skips_final_boundary_but_all_held_regions_still_finish() {
        assert_eq!(
            journal_policy(false),
            [
                "calendar",
                "kis",
                "chain",
                "root",
                "historical",
                "config",
                "current"
            ]
        );
    }

    fn raw_prior_daily(source: DailyPolicySource) -> AdmissionSource {
        AdmissionSource::Raw(Box::new(NativeRawCompletion {
            operation: Err(RawBodySource::Prior(Box::new(AdmissionSource::from(
                source,
            )))),
            kis: None,
            calendar: None,
            lineage: Vec::new(),
        }))
    }
    #[test]
    fn daily_typed_diagnostics_recurse_raw_prior_without_legacy_replay_projection() {
        let scope = Attempt::default();
        let invocation = RawInvocation::new(&scope);
        let completion =
            complete_admission_local(&invocation, &scope, &scope, Ok(()), |invocation, _| {
                let whole = invocation
                    .settle_local_in(&scope, Err::<(), _>(DailyReplayError::Interrupted))
                    .unwrap_err();
                Err(raw_prior_daily(DailyPolicySource::Replay(Box::new(whole))))
            });
        let observed = LocalObservedAdmissionFailure {
            completion: Box::new(completion),
        };
        let diagnostic = observed.diagnostics();
        assert_eq!(diagnostic.first_kind(), Some(AdmissionFirstKind::Policy));
        assert!(diagnostic.source_report().is_none());
        assert!(diagnostic.policy_failure().is_none());
        assert!(diagnostic.historical_replay_result().is_none());
        let daily = diagnostic.daily_evidence().unwrap();
        match daily.use_failure() {
            Some(DailyPolicySource::Replay(first)) => {
                assert_eq!(first.body(), Some(&DailyReplayError::Interrupted))
            }
            _ => panic!("actual typed replay source"),
        }
        assert!(daily.root_completion().is_none() && daily.chain_completion().is_none());
    }

    #[test]
    fn plain_daily_policy_and_replay_first_report_remains_absent_in_owned_and_local() {
        let scope = Attempt::default();
        let expected = scope.clone();
        let captured = scope.clone();
        let complete = complete_admission(scope, &expected, &captured, |invocation, _| {
            let first = invocation.settle_local_in(&expected, Err::<(), _>(
                market_data::daily_price_only::admission::DailyAdmissionVerificationError::InvalidCandidate,
            )).unwrap_err();
            Err::<(), _>(AdmissionSource::from(DailyPolicySource::Policy(Box::new(
                first,
            ))))
        });
        let observed = ObservedAdmissionFailure {
            completion: Box::new(complete),
        };
        let diagnostic = observed.diagnostics();
        assert_eq!(diagnostic.first_kind(), Some(AdmissionFirstKind::Policy));
        assert!(diagnostic.source_report().is_none());
        assert!(diagnostic.historical_replay_result().is_none());
        assert!(diagnostic.daily_evidence().unwrap().use_failure().is_some());
        assert!(!format!("{:?}", diagnostic.daily_evidence().unwrap()).contains("Report"));
    }

    #[test]
    fn original_late_daily_kind_none_survives_later_actual_scope_source() {
        let scope = Attempt::default();
        let invocation = RawInvocation::new(&scope);
        let later = CheckedDailyAdmissionRoot::open_in(
            &invocation,
            &scope,
            &scope,
            std::path::Path::new("not-accessed-by-later-None"),
            10003,
            10001,
        )
        .unwrap_err();
        assert!(later.source_report().is_some());
        let source = DailyPolicySource::Joined(Box::new((
            Err(DailyPolicySource::Late),
            Err(DailyPolicySource::Artifact(Box::new(later))),
        )));
        let source = AdmissionSource::from(source);
        assert_eq!(admission_source_kind(&source), None);
        assert!(admission_source_report(&source).is_none());
        assert_eq!(daily_root_kind(None), None);
        assert_eq!(daily_kind(None), None);
    }

    #[test]
    fn owning_region_unwind_marks_gap_and_records_no_normal_completion() {
        let scope = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = holder(&scope, &journal, "outer", true);
        let normal = Cell::new(0);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _unwind = AdmissionUnwind {
                attempt: scope.clone(),
                armed: true,
            };
            let root = holder(&scope, &journal, "root-fallback", true);
            let _: (Result<(), ErrorBody>, Completed) = run_owned_region(
                root,
                |_| panic!("ordinary unwind"),
                |_| normal.set(normal.get() + 1),
                |held| {
                    normal.set(normal.get() + 1);
                    finish(held)
                },
            );
        }));
        assert!(result.is_err());
        assert_eq!(normal.get(), 0);
        assert_eq!(&*journal.borrow(), &["root-fallback"]);
        assert_eq!(scope.report().live(), 1);
        assert!(!finish(outer).clean);
    }

    #[test]
    fn intake_debug_is_bounded_without_body_clone_or_debug() {
        let error = DailyReplayIntakeFailure::<ErrorBody, ErrorBody, Data>::Use {
            first: ErrorBody("private original body".into()),
            original: Ok(Data("private returned bytes".into())),
        };
        let text = format!("{error:?}");
        assert!(text.contains("original_ok"));
        assert!(!text.contains("private") && !text.contains("bytes"));
        assert!(text.len() < 160);
    }

    #[test]
    fn raw_post_checkpoint_keeps_actual_success_before_join_refuses_second() {
        let scope = Attempt::default();
        let invocation = RawInvocation::new(&scope);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let reads = Cell::new(0);
        let checks = Cell::new(0);
        let mut slots = [None, None];
        let original = capture_raw_pair_with_checkpoint(
            &mut slots,
            |channel| {
                reads.set(reads.get() + 1);
                Ok::<_, ErrorBody>(holder(&scope, &journal, channel.provider(), true))
            },
            || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    scope.gap(Gap::Unbound);
                }
                join_daily_checks(
                    || invocation.settle_local_in(&scope, Ok::<(), Infallible>(())),
                    || invocation.settle_local_in(&scope, Ok::<(), Infallible>(())),
                )
                .map_err(|_| ErrorBody("actual joined refusal".into()))
            },
        );
        assert!(original.is_err());
        assert_eq!(reads.get(), 1);
        assert!(slots[0].is_some() && slots[1].is_none());
        let completed = finish_raw_pair(original, slots, Vec::<Data>::new(), finish);
        assert!(completed.operation.is_err());
        assert_eq!(&*journal.borrow(), &["kis"]);
    }
}

// END C16 production-shared tests

#[cfg(test)]
mod tests {
    use super::*;
    use market_data::{
        checked_close::{Close, Errno, Issue, Role},
        storage::CheckedCommittedEvidencePhase,
    };
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };

    struct Row {
        drops: Rc<Cell<usize>>,
        value: Box<str>,
    }
    impl Drop for Row {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    struct Body(Box<str>);
    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }
    fn model_row(drops: &Rc<Cell<usize>>) -> Row {
        Row {
            drops: drops.clone(),
            value: "retained ordinary row".into(),
        }
    }

    #[test]
    fn observed_none_binding_retained_before_any_second_or_downstream_callback() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let reads = Cell::new([0usize; 2]);
        let snapshots = Cell::new(0);
        let originals = Cell::new(0);
        let links = Cell::new(0);
        let result = complete_admission(attempt, &expected, &captured, |invocation, bags| {
            read_channels(
                &mut (),
                bags,
                |_, channel| {
                    let mut counts = reads.get();
                    counts[channel.index()] += 1;
                    reads.set(counts);
                    // Real public refusal: no protected object, filesystem or UID probe.
                    let result = RawStore::new("not-accessed-by-unbound-admission")
                        .read_committed_manifest_evidence_observed_checked(
                            invocation,
                            &expected,
                            channel.provider(),
                            "kr",
                            0,
                            &CommittedManifestReadLimits {
                                max_manifest_bytes: 1,
                                max_records: 1,
                            },
                        );
                    match result {
                        Ok(observation) => Ok(observation),
                        Err(failure) => Err(Box::new(failure)),
                    }
                },
                |_| Ok(()),
                |_, _| {
                    snapshots.set(snapshots.get() + 1);
                    originals.set(originals.get() + 1);
                    links.set(links.get() + 1);
                    Ok(())
                },
            )
        });
        assert!(!result.is_clean());
        let failure = match &result.operation {
            Some(Err(failure)) => failure,
            _ => panic!("actual canonical None gate must refuse"),
        };
        assert_eq!(
            failure.source().phase(),
            Some(CheckedCommittedEvidencePhase::Binding)
        );
        assert!(failure.source().body().is_none());
        assert!(failure.records().is_none());
        assert_eq!(reads.get(), [1, 0]);
        assert_eq!((snapshots.get(), originals.get(), links.get()), (0, 0, 0));
        assert!(result.observations.channels.iter().all(Option::is_none));
        assert!(result.local.is_err() && result.terminal.is_err());
    }

    fn row<'a>(
        batch: u8,
        line_number: usize,
        bytes: &'a [u8],
        hash: &'a ContentHash,
    ) -> AcceptedRow<'a, u8> {
        AcceptedRow {
            provider: "kis",
            market: "kr",
            batch,
            line_number,
            line_bytes: bytes,
            line_sha256: hash,
        }
    }

    #[test]
    fn complete_accepted_sets_match_without_discarding_unselected_rows() {
        let a = ContentHash::from_bytes(b"first");
        let b = ContentHash::from_bytes(b"second");
        let observed = [row(1, 1, b"first", &a), row(2, 4, b"second", &b)];
        let legacy = [row(2, 4, b"second", &b), row(1, 1, b"first", &a)];
        assert!(match_accepted_rows("kis", &observed, &legacy).is_ok());
        assert!(match_accepted_rows::<u8>("kis", &[], &[]).is_ok());
        assert!(match_accepted_rows("kis", &observed[..1], &legacy).is_err());
        assert!(match_accepted_rows("kis", &observed, &legacy[..1]).is_err());
        let duplicate = [row(1, 1, b"first", &a), row(1, 1, b"first", &a)];
        assert!(match_accepted_rows("kis", &duplicate, &duplicate).is_err());
    }

    #[test]
    fn channel_bytes_hash_and_first_physical_ordinal_are_required() {
        let hash = ContentHash::from_bytes(b"first");
        let other_hash = ContentHash::from_bytes(b"later");
        let legacy = [row(1, 2, b"first", &hash)];
        let mut changed = row(1, 2, b"first", &hash);
        changed.provider = "kis-calendar";
        assert!(match_accepted_rows("kis", &[changed], &legacy).is_err());
        let mut changed = row(1, 2, b"first", &hash);
        changed.market = "us";
        assert!(match_accepted_rows("kis", &[changed], &legacy).is_err());
        assert!(match_accepted_rows("kis", &[row(1, 2, b"later", &other_hash)], &legacy).is_err());
        assert!(match_accepted_rows("kis", &[row(1, 2, b"first", &other_hash)], &legacy).is_err());
        // Identical later JSON bytes are still not genuine FIRST evidence.
        assert!(match_accepted_rows("kis", &[row(1, 7, b"first", &hash)], &legacy).is_err());
        assert!(match_accepted_rows("kis", &[row(2, 2, b"first", &hash)], &legacy).is_err());
        let invalid_hash = [row(1, 2, b"first", &other_hash)];
        assert!(match_accepted_rows("kis", &invalid_hash, &invalid_hash).is_err());
    }

    #[test]
    fn first_non_clone_some_moves_whole_before_later_unit_failure() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let source = attempt.clone();
        let reads = Cell::new(0);
        let result: Completion<Row, Failure<Body>> =
            complete_admission(attempt, &expected, &captured, |_, bags| {
                read_channels(
                    &mut (),
                    bags,
                    |_, _| {
                        reads.set(reads.get() + 1);
                        Err(source.failure(Some(Body("original body".into()))))
                    },
                    |_| Ok(()),
                    |_, _| panic!("downstream after first error"),
                )
            });
        assert_eq!(reads.get(), 1);
        let first = result.operation.as_ref().unwrap().as_ref().err().unwrap();
        assert_eq!(first.body().unwrap().0.as_ref(), "original body");
        assert_eq!(first.report().first(), Some(Issue::Gap(Gap::Abandoned)));
        assert!(first.report().secondary().is_empty());
        assert!(
            !result
                .local
                .as_ref()
                .err()
                .unwrap()
                .report()
                .secondary()
                .is_empty()
        );
        assert!(result.terminal.is_err());
        assert_eq!(
            original_report(&result, |failure| Some(failure.report()))
                .unwrap()
                .first(),
            first.report().first()
        );
    }

    #[test]
    fn second_bodyless_none_keeps_prior_non_debug_bag_until_outer_drop() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let source = attempt.clone();
        let drops = Rc::new(Cell::new(0));
        let reads = Cell::new(0);
        let result: Completion<Row, Failure<Body>> =
            complete_admission(attempt, &expected, &captured, |_, bags| {
                read_channels(
                    &mut (),
                    bags,
                    |_, channel| {
                        reads.set(reads.get() + 1);
                        match channel {
                            AdmissionChannel::Kis => Ok(model_row(&drops)),
                            AdmissionChannel::KisCalendar => Err(source.failure(None)),
                        }
                    },
                    |_| Ok(()),
                    |_, _| panic!("downstream after second error"),
                )
            });
        assert_eq!(reads.get(), 2);
        assert_eq!(drops.get(), 0);
        assert!(result.observations.get(AdmissionChannel::Kis).is_some());
        assert!(
            result
                .observations
                .get(AdmissionChannel::KisCalendar)
                .is_none()
        );
        assert!(
            result
                .operation
                .as_ref()
                .unwrap()
                .as_ref()
                .err()
                .unwrap()
                .body()
                .is_none()
        );
        assert!(result.local.is_err() && result.terminal.is_err());
        drop(result);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn absent_original_report_never_falls_back_to_later_real_report() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let result: Completion<Row, Body> =
            complete_admission(attempt, &expected, &captured, |_, _| {
                Err(Body("policy without Raw report".into()))
            });
        assert!(result.local.is_err() && result.terminal.is_err());
        assert!(original_report(&result, |_| None).is_none());
        let body = result.operation.as_ref().unwrap().as_ref().err().unwrap();
        assert_eq!(body.0.as_ref(), "policy without Raw report");
    }

    #[test]
    fn late_dirt_refuses_value_without_inventing_first_error_or_losing_rows() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let source = attempt.clone();
        let drops = Rc::new(Cell::new(0));
        let result: Completion<Row, Body> =
            complete_admission(attempt, &expected, &captured, |_, bags| {
                read_channels(
                    &mut (),
                    bags,
                    |_, _| Ok(model_row(&drops)),
                    |_| Ok(()),
                    |_, _| {
                        source.gap(Gap::TerminalPublication);
                        Ok(())
                    },
                )
            });
        assert!(!result.is_clean());
        assert!(matches!(result.operation, Some(Ok(()))));
        assert!(original_report(&result, |_| None).is_none());
        assert!(result.local.is_err() && result.terminal.is_err());
        assert_eq!(drops.get(), 0);
        assert_eq!(
            result
                .observations
                .get(AdmissionChannel::Kis)
                .unwrap()
                .value
                .as_ref(),
            "retained ordinary row"
        );
        drop(result);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn checkpoint_error_retains_just_completed_channel_before_next_read() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let drops = Rc::new(Cell::new(0));
        let reads = Cell::new(0);
        let result: Completion<Row, Body> =
            complete_admission(attempt, &expected, &captured, |_, bags| {
                read_channels(
                    &mut (),
                    bags,
                    |_, _| {
                        reads.set(reads.get() + 1);
                        Ok(model_row(&drops))
                    },
                    |_| Err(Body("boundary changed".into())),
                    |_, _| panic!("no policy after changed boundary"),
                )
            });
        assert_eq!(reads.get(), 1);
        assert!(result.observations.get(AdmissionChannel::Kis).is_some());
        assert!(
            result
                .observations
                .get(AdmissionChannel::KisCalendar)
                .is_none()
        );
        assert_eq!(drops.get(), 0);
        drop(result);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn both_foreign_scope_comparisons_poison_even_when_already_dirty() {
        let attempt = Attempt::default();
        attempt.gap(Gap::Reuse);
        let actual = attempt.clone();
        let expected = Attempt::default();
        let captured = Attempt::default();
        let calls = Cell::new(0);
        let result: Completion<Row, Body> =
            complete_admission(attempt, &expected, &captured, |_, _| {
                calls.set(calls.get() + 1);
                Ok(())
            });
        assert_eq!(calls.get(), 0);
        assert!(result.operation.is_none());
        assert!(result.expected_scope.is_err() && result.captured_scope.is_err());
        assert_eq!(
            expected.report().first(),
            Some(Issue::Gap(Gap::ScopeMismatch))
        );
        assert_eq!(
            captured.report().first(),
            Some(Issue::Gap(Gap::ScopeMismatch))
        );
        assert_eq!(actual.report().first(), Some(Issue::Gap(Gap::Reuse)));
        assert!(
            actual
                .report()
                .secondary()
                .iter()
                .filter(|issue| **issue == Issue::Gap(Gap::ScopeMismatch))
                .count()
                >= 2
        );
    }

    #[test]
    fn clean_completion_closes_counted_owner_once_and_keeps_both_rows() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let source = attempt.clone();
        let closes = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let result: Completion<Row, Body> =
            complete_admission(attempt, &expected, &captured, |_, bags| {
                let owner = source.adopt(Counted(closes.clone()), Role::Source);
                let read = read_channels(
                    &mut (),
                    bags,
                    |_, _| Ok(model_row(&drops)),
                    |_| Ok(()),
                    |_, _| Ok(()),
                );
                owner.finish();
                read
            });
        assert!(result.is_clean());
        assert_eq!(closes.get(), 1);
        assert_eq!(source.report().attempted(), 1);
        assert_eq!(drops.get(), 0);
        drop(result);
        assert_eq!(drops.get(), 2);
        assert_eq!(closes.get(), 1);
    }

    #[test]
    fn unwind_adds_only_gap_and_never_drains_outer_counted_owner() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let source = attempt.clone();
        let closes = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let owner = source.adopt(Counted(closes.clone()), Role::Source);
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _: Completion<Row, Body> =
                complete_admission(attempt, &expected, &captured, |_, bags| {
                    bags.channels[0] = Some(model_row(&drops));
                    panic!("ordinary callback unwind")
                });
        }));
        assert!(caught.is_err());
        assert_eq!(source.report().first(), Some(Issue::Gap(Gap::Abandoned)));
        assert_eq!(
            (
                closes.get(),
                source.report().attempted(),
                source.report().live()
            ),
            (0, 0, 1)
        );
        assert_eq!(drops.get(), 1);
        owner.finish();
        assert_eq!(closes.get(), 1);
        assert!(source.finish(Ok::<(), Infallible>(())).is_err());
    }
}

#[cfg(test)]
mod current_completion_tests {
    use super::*;
    use market_data::checked_close::{Close, Errno, Role};
    use std::{cell::Cell, rc::Rc};

    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Err(Errno::Interrupted)
        }
    }
    struct Body(Box<str>);
    struct Row(Rc<Cell<usize>>);
    impl Drop for Row {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn admission_original_failure_survives_current_completion() {
        for with_body in [true, false] {
            let attempt = Attempt::default();
            let expected = attempt.clone();
            let captured = attempt.clone();
            let calls = Rc::new(Cell::new(0));
            let drops = Rc::new(Cell::new(0));
            let mut later_current = None;
            let mut address = None;
            let completion =
                complete_admission(attempt, &expected, &captured, |invocation, rows| {
                    rows.channels[0] = Some(Row(Rc::clone(&drops)));
                    let owner = captured.adopt(Counted(Rc::clone(&calls)), Role::Current);
                    let original = if with_body {
                        let body = Body("nonClone original".into());
                        address = Some(body.0.as_ptr());
                        invocation.settle_local_in(&expected, Err::<(), _>(body))
                    } else {
                        captured.gap(Gap::TerminalPublication);
                        invocation.settle_local_in(&expected, Ok::<(), Body>(()))
                    };
                    let source = match original {
                        Err(source) => source,
                        Ok(()) => panic!("actual canonical failure"),
                    };
                    // Whole source snapshot predates the close and is never rebuilt.
                    assert_eq!(source.report().attempted(), 0);
                    let original = Err::<(), _>(source);
                    let _ = owner.finish();
                    let later = invocation.settle_local_in(&captured, Ok::<(), Infallible>(()));
                    retain_current_result(original, later, &mut later_current)
                });
            assert!(!completion.is_clean());
            let source = match &completion.operation {
                Some(Err(source)) => source,
                _ => panic!("original failure retained"),
            };
            assert_eq!(source.body().is_some(), with_body);
            if let Some(body) = source.body() {
                assert_eq!(Some(body.0.as_ptr()), address);
            }
            assert_eq!(source.report().attempted(), 0);
            assert_eq!(calls.get(), 1);
            let later = later_current.as_ref().unwrap().as_ref().err().unwrap();
            assert_eq!(later.report().attempted(), 1);
            assert!(later.body().is_none());
            assert_eq!(drops.get(), 0);
            assert!(completion.observations.channels[0].is_some());
            assert!(completion.observations.channels[1].is_none());
            // An absent original report is intentionally not replaced by later.
            assert!(original_report(&completion, |_| None).is_none());
            drop(completion);
            assert_eq!(drops.get(), 1);
        }
    }

    #[test]
    fn admission_success_then_current_close_failure_retains_ok() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let calls = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let mut later_current = None;
        let completion = complete_admission(attempt, &expected, &captured, |invocation, rows| {
            rows.channels[0] = Some(Row(Rc::clone(&drops)));
            rows.channels[1] = Some(Row(Rc::clone(&drops)));
            let owner = captured.adopt(Counted(Rc::clone(&calls)), Role::Current);
            let original = Ok::<(), Body>(());
            let _ = owner.finish();
            let later = invocation.settle_local_in(&captured, Ok::<(), Infallible>(()));
            retain_current_result(original, later, &mut later_current)
        });
        assert!(matches!(completion.operation, Some(Ok(()))));
        assert!(!completion.is_clean());
        assert!(completion.local.as_ref().err().unwrap().body().is_none());
        assert!(completion.terminal.as_ref().err().unwrap().body().is_none());
        assert!(later_current.as_ref().unwrap().is_err());
        assert_eq!(calls.get(), 1);
        assert_eq!(drops.get(), 0);
        assert!(completion.observations.channels.iter().all(Option::is_some));
        assert!(original_report(&completion, |_| None).is_none());
        drop(completion);
        assert_eq!(drops.get(), 2);
    }
}

#[cfg(test)]
mod borrowed_source_tests {
    use super::*;
    use market_data::checked_close::{Close, Errno, Role};
    use std::{cell::Cell, rc::Rc};
    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }
    struct Row(Box<str>);
    struct Body(Box<str>);

    #[test]
    fn borrowed_admission_keeps_outer_chain_live() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(Counted(calls.clone()), Role::Source);
        let invocation = RawInvocation::new(&attempt);
        let local = complete_admission_local(
            &invocation,
            &attempt,
            &attempt,
            Ok::<(), Body>(()),
            |_, observations| {
                observations.channels[0] = Some(Row("first".into()));
                observations.channels[1] = Some(Row("second".into()));
                Ok(())
            },
        );
        assert!(local.is_clean());
        assert_eq!(
            &*local.observations.channels[0].as_ref().unwrap().0,
            "first"
        );
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(calls.get(), 0);
        assert!(owner.finish());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_ok());

        let attempt = Attempt::default();
        let foreign = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let effects = Cell::new(0);
        let local: LocalCompletion<Row, Body> =
            complete_admission_local(&invocation, &foreign, &attempt, Ok(()), |_, _| {
                effects.set(effects.get() + 1);
                Ok(())
            });
        assert!(local.operation.is_none());
        assert!(local.expected_scope.is_err() && local.captured_scope.is_err());
        assert_eq!(effects.get(), 0);
        assert!(!foreign.check());

        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(Counted(calls.clone()), Role::Source);
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: LocalCompletion<Row, Body> =
                complete_admission_local(&invocation, &attempt, &attempt, Ok(()), |_, _| {
                    panic!("ordinary unwind")
                });
        }));
        assert!(unwind.is_err());
        assert_eq!(calls.get(), 0);
        assert_eq!(attempt.report().live(), 1);
        assert!(!owner.finish());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    fn borrowed_admission_first_some_and_none_keep_prior_bags() {
        for with_body in [false, true] {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let local = complete_admission_local(
                &invocation,
                &attempt,
                &attempt,
                Ok(()),
                |_, observations| {
                    observations.channels[0] = Some(Row("prior bag".into()));
                    Err::<(), _>(attempt.failure(if with_body {
                        Some(Body("original".into()))
                    } else {
                        None
                    }))
                },
            );
            let source = local.operation.as_ref().unwrap().as_ref().err().unwrap();
            assert_eq!(source.body().is_some(), with_body);
            if let Some(body) = source.body() {
                assert_eq!(&*body.0, "original");
            }
            assert_eq!(
                &*local.observations.channels[0].as_ref().unwrap().0,
                "prior bag"
            );
            assert!(local.local.is_err());
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let local = complete_admission_local(
            &invocation,
            &attempt,
            &attempt,
            Ok::<(), Body>(()),
            |_, observations| {
                observations.channels[0] = Some(Row("retained on late dirt".into()));
                attempt.gap(Gap::TerminalPublication);
                Ok(())
            },
        );
        assert!(matches!(local.operation, Some(Ok(()))));
        assert!(!local.is_clean());
        assert!(local.local.is_err());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());

        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let effects = Cell::new(0);
        let source = attempt.failure::<Body>(None);
        let local: LocalCompletion<Row, Failure<Body>> =
            complete_admission_local(&invocation, &attempt, &attempt, Err(source), |_, _| {
                effects.set(effects.get() + 1);
                Ok(())
            });
        assert!(
            local
                .operation
                .as_ref()
                .unwrap()
                .as_ref()
                .err()
                .unwrap()
                .body()
                .is_none()
        );
        assert_eq!(effects.get(), 0);
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        // The original body-less-report lane is not replaced by later local errors.
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let local =
            complete_admission_local::<Row, _>(&invocation, &attempt, &attempt, Ok(()), |_, _| {
                Err(AdmissionSource::Policy(EvidenceError))
            });
        let first = local.operation.as_ref().unwrap().as_ref().err().unwrap();
        assert!(admission_source_report(first).is_none());
        assert!(local.local.is_err());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }
}

#[cfg(test)]
mod config_observation_tests {
    use super::*;
    use crate::owner_beta::runner::daily_ingest::{ConfigSession, IngestError};
    use market_data::checked_close::{Close, Errno, Role};
    use std::{cell::Cell, rc::Rc};
    struct Row(Box<str>);
    struct Body(Box<str>);
    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Err(Errno::Io)
        }
    }

    #[test]
    fn whole_config_ingest_and_absent_first_report_survive_later_units() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let completion = complete_admission_local(
            &invocation,
            &attempt,
            &attempt,
            Ok(()),
            |invocation, observations| {
                let mut session = ConfigSession::borrowed_in(invocation, &attempt, &attempt);
                observations.channels[0] = Some(Row("prior row".into()));
                let source = session.policy_error(IngestError::Configuration);
                let original = Err::<(), _>(AdmissionSource::from(
                    super::super::ContextConfigFailure::Ingest(source),
                ));
                observations.config = Some(ConfigObservations {
                    initial: None,
                    retained: Default::default(),
                    completion: session.finish_local(),
                });
                original
            },
        );
        let source = completion
            .operation
            .as_ref()
            .unwrap()
            .as_ref()
            .err()
            .unwrap();
        let failure = match source {
            AdmissionSource::Config(failure) => failure,
            _ => panic!("whole config lane"),
        };
        assert_eq!(failure.ingest(), Some(&IngestError::Configuration));
        assert_eq!(
            &*completion.observations.channels[0].as_ref().unwrap().0,
            "prior row"
        );
        assert!(
            admission_report_with_config(source, completion.observations.config.as_ref()).is_none()
        );
        assert!(completion.local.is_err());
        assert_eq!(
            admission_first_kind(&completion.operation),
            Some(AdmissionFirstKind::Context)
        );
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());

        for some in [false, true] {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let completion = complete_admission_local::<Row, _>(
                &invocation,
                &attempt,
                &attempt,
                Ok(()),
                |_, observations| {
                    observations.channels[0] = Some(Row("retained".into()));
                    Err(attempt.failure(if some {
                        Some(Body("original".into()))
                    } else {
                        None
                    }))
                },
            );
            let first = completion
                .operation
                .as_ref()
                .unwrap()
                .as_ref()
                .err()
                .unwrap();
            assert_eq!(first.body().is_some(), some);
            if let Some(body) = first.body() {
                assert_eq!(&*body.0, "original");
            }
            assert!(completion.local.is_err());
            assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
        }
    }

    #[test]
    fn original_success_and_prior_bags_survive_late_config_uncertainty() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let calls = Rc::new(Cell::new(0));
        let completion = complete_admission_local::<Row, AdmissionSource>(
            &invocation,
            &attempt,
            &attempt,
            Ok(()),
            |invocation, observations| {
                let mut session = ConfigSession::borrowed_in(invocation, &attempt, &attempt);
                observations.channels[0] = Some(Row("first".into()));
                observations.channels[1] = Some(Row("second".into()));
                let owner = attempt.adopt(Counted(calls.clone()), Role::Source);
                let later = session.retire(
                    owner,
                    Ok::<(), crate::owner_beta::runner::daily_ingest::ConfigFailure<Body>>(()),
                );
                assert_eq!(
                    later.err().unwrap().kind(),
                    super::super::ConfigFirstKind::Late
                );
                observations.config = Some(ConfigObservations {
                    initial: None,
                    retained: Default::default(),
                    completion: session.finish_local(),
                });
                Ok(())
            },
        );
        assert!(matches!(completion.operation, Some(Ok(()))));
        assert_eq!(admission_first_kind(&completion.operation), None);
        assert_eq!(
            &*completion.observations.channels[0].as_ref().unwrap().0,
            "first"
        );
        assert_eq!(
            &*completion.observations.channels[1].as_ref().unwrap().0,
            "second"
        );
        assert!(!completion.is_clean());
        assert_eq!(calls.get(), 1);
        assert!(completion.local.is_err());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }
}

#[cfg(test)]
mod raw_pair_tests {
    use super::*;
    use market_data::{
        checked_close::{Close, Errno, Role},
        daily_price_only::source::ScanLimitValues,
    };
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };

    struct Body(Box<str>);
    struct Row(Box<str>);
    struct Counted {
        name: &'static str,
        journal: Rc<RefCell<Vec<&'static str>>>,
        result: Result<(), Errno>,
    }
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.journal.borrow_mut().push(self.name);
            self.result
        }
    }
    #[test]
    fn raw_pair_retains_second_failure_then_finishes_first() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let mut slots = [None, None];
        let first_body = Body("ordinary second failure".into());
        let address = first_body.0.as_ptr();
        let mut first_body = Some(first_body);
        let original = capture_raw_pair(&mut slots, |channel| {
            let owner = attempt.adopt(
                Counted {
                    name: channel.provider(),
                    journal: journal.clone(),
                    result: Ok(()),
                },
                Role::Source,
            );
            match channel {
                AdmissionChannel::Kis => Ok(owner),
                AdmissionChannel::KisCalendar => {
                    assert!(owner.finish());
                    invocation.settle_local_in(
                        &attempt,
                        Err(first_body.take().expect("one original error")),
                    )
                }
            }
        });
        assert_eq!(&*journal.borrow(), &["kis-calendar"]);
        let complete = finish_raw_pair(original, slots, Vec::<Row>::new(), |owner| owner.finish());
        assert_eq!(&*journal.borrow(), &["kis-calendar", "kis"]);
        assert!(complete.calendar.is_none());
        assert!(complete.kis.is_some());
        let source = complete.operation.as_ref().err().unwrap();
        assert_eq!(source.body().unwrap().0.as_ptr(), address);
        assert!(!complete.is_clean_with(|clean| *clean));
        assert_eq!(attempt.report().live(), 0);
    }
    #[test]
    fn raw_pair_success_cleanup_is_calendar_then_kis_without_finishing_outer() {
        let attempt = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(
            Counted {
                name: "outer",
                journal: journal.clone(),
                result: Ok(()),
            },
            Role::Source,
        );
        let mut slots = [None, None];
        let original = capture_raw_pair(&mut slots, |channel| {
            Ok::<_, Body>(attempt.adopt(
                Counted {
                    name: channel.provider(),
                    journal: journal.clone(),
                    result: Ok(()),
                },
                Role::Source,
            ))
        });
        assert!(original.is_ok());
        let complete = finish_raw_pair(
            original,
            slots,
            vec![Row("actual ordinary data".into())],
            |owner| owner.finish(),
        );
        assert!(complete.is_clean_with(|clean| *clean));
        assert_eq!(&*journal.borrow(), &["kis-calendar", "kis"]);
        assert_eq!(complete.lineage[0].0.as_ref(), "actual ordinary data");
        assert_eq!(attempt.report().live(), 1);
        assert!(outer.finish());
        assert_eq!(&*journal.borrow(), &["kis-calendar", "kis", "outer"]);
    }
    #[test]
    fn raw_pair_late_unknown_keeps_ok_and_prevents_next_policy() {
        let attempt = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let owner = attempt.adopt(
            Counted {
                name: "kis",
                journal: journal.clone(),
                result: Err(Errno::BadDescriptor),
            },
            Role::Source,
        );
        let original = Ok::<_, Body>(Row("original Ok".into()));
        let complete = finish_raw_pair(original, [Some(owner), None], Vec::<Row>::new(), |owner| {
            owner.finish()
        });
        let mut retained = None;
        let next = Cell::new(0);
        let result = retain_raw_completion(complete, |clean| *clean, &mut retained);
        if result.is_ok() {
            next.set(next.get() + 1);
        }
        let complete = result.err().unwrap();
        assert_eq!(
            complete.operation.as_ref().ok().unwrap().0.as_ref(),
            "original Ok"
        );
        let invocation = RawInvocation::new(&attempt);
        let late = invocation
            .settle_local_in(&attempt, Ok::<(), Infallible>(()))
            .err()
            .unwrap();
        let report_calls = Cell::new(0);
        assert!(
            complete
                .source_report_with(|_| {
                    report_calls.set(report_calls.get() + 1);
                    Some(late.report())
                })
                .is_none()
        );
        assert_eq!(report_calls.get(), 0);
        assert_eq!(next.get(), 0);
        assert!(retained.is_none());
        assert_eq!(attempt.report().unknown(), 1);
        assert_eq!(&*journal.borrow(), &["kis"]);
        assert!(!format!("{complete:?}").contains("original Ok"));
    }
    #[test]
    fn raw_use_retains_non_clone_data_even_when_inspection_fails() {
        let mut retained = Vec::new();
        let body = Body("first use error".into());
        let address = body.0.as_ptr();
        let result = retain_raw_use(
            Row("successful resolution".into()),
            &mut retained,
            |_| Err::<(), _>(body),
            |row| row,
        );
        assert_eq!(result.err().unwrap().0.as_ptr(), address);
        assert_eq!(retained[0].0.as_ref(), "successful resolution");
    }
    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "The public refusal test preserves the whole native failure without boxing."
    )]
    fn raw_pair_public_none_refusal_never_reads_second_or_runs_downstream() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let reads = Cell::new([0usize; 2]);
        let downstream = Cell::new(0);
        let mut slots = [None, None];
        let reader = CommittedRawReader::new(std::path::Path::new("not-accessed-by-None"));
        let original = capture_raw_pair(&mut slots, |channel| {
            let mut counts = reads.get();
            counts[channel.index()] += 1;
            reads.set(counts);
            reader.snapshot_checked_in(
                &invocation,
                &attempt,
                &attempt,
                ApprovedRawScope::new(channel.provider(), "kr", 0).unwrap(),
                ScanLimits::new(ScanLimitValues {
                    max_manifest_bytes: 1,
                    max_manifest_records: 1,
                    max_batches: 1,
                    max_files_per_batch: 1,
                    max_files: 1,
                    max_batch_metadata_bytes: 1,
                    max_source_file_bytes: 1,
                    max_retained_bytes: 1,
                    max_open_descriptors: 1,
                    max_data_root_bytes: 1,
                    max_data_root_components: 1,
                    max_metadata_json_depth: 1,
                })
                .unwrap(),
            )
        });
        if original.is_ok() {
            downstream.set(downstream.get() + 1);
        }
        let complete = finish_raw_pair(original, slots, Vec::<Row>::new(), |snapshot| {
            snapshot.finish_local()
        });
        assert_eq!(reads.get(), [1, 0]);
        assert_eq!(downstream.get(), 0);
        let source = complete.operation.as_ref().err().unwrap();
        assert_eq!(source.kind(), SnapshotFirstKind::Binding);
        assert!(source.binding_failure().unwrap().body().is_none());
        assert!(complete.kis.is_none() && complete.calendar.is_none());
    }
    #[test]
    fn raw_matcher_rejects_missing_extra_duplicate_and_later_ordinal() {
        let hash = ContentHash::from_bytes(b"line");
        let row = |id, line| AcceptedRow {
            provider: "kis",
            market: "kr",
            batch: id,
            line_number: line,
            line_bytes: b"line",
            line_sha256: &hash,
        };
        assert!(match_accepted_rows("kis", &[row(1, 1)], &[row(1, 1)]).is_ok());
        assert!(match_accepted_rows("kis", &[row(1, 1)], &[]).is_err());
        assert!(match_accepted_rows("kis", &[row(1, 1)], &[row(1, 1), row(2, 2)]).is_err());
        assert!(
            match_accepted_rows("kis", &[row(1, 1), row(1, 1)], &[row(1, 1), row(2, 2)]).is_err()
        );
        assert!(match_accepted_rows("kis", &[row(1, 2)], &[row(1, 1)]).is_err());
    }
    #[test]
    fn raw_pair_unwind_keeps_outer_owner_and_marks_actual_gap() {
        let attempt = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(
            Counted {
                name: "outer",
                journal: journal.clone(),
                result: Ok(()),
            },
            Role::Source,
        );
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = AdmissionUnwind {
                attempt: attempt.clone(),
                armed: true,
            };
            let mut slots = [None::<Row>, None];
            let _ = capture_raw_pair(&mut slots, |channel| -> Result<Row, Body> {
                if channel == AdmissionChannel::Kis {
                    Ok(Row("retained".into()))
                } else {
                    panic!("ordinary capture panic")
                }
            });
        }));
        assert!(result.is_err());
        assert!(!attempt.check());
        assert_eq!(attempt.report().live(), 1);
        assert!(journal.borrow().is_empty());
        assert!(!outer.finish());
        assert_eq!(&*journal.borrow(), &["outer"]);
    }
}

#[cfg(test)]
mod historical_consumer_tests {
    use super::*;
    use market_data::{
        checked_close::{Close, Errno, Owner, Role},
        daily_price_only::replay::DailyReplayLimits,
    };
    use std::{
        cell::{Cell, RefCell},
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };
    struct Row(Box<str>);
    struct Body(Box<str>);
    struct Counted {
        journal: Rc<RefCell<Vec<&'static str>>>,
        name: &'static str,
        result: Result<(), Errno>,
    }
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.journal.borrow_mut().push(self.name);
            self.result
        }
    }
    struct Lease {
        owner: Owner<Counted>,
        data: Row,
    }
    fn lease(
        attempt: &Attempt,
        journal: &Rc<RefCell<Vec<&'static str>>>,
        result: Result<(), Errno>,
    ) -> Lease {
        Lease {
            owner: attempt.adopt(
                Counted {
                    journal: journal.clone(),
                    name: "historical",
                    result,
                },
                Role::Source,
            ),
            data: Row("ordinary lease data".into()),
        }
    }
    fn finish_model(
        held: Lease,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
    ) -> Result<Row, (Row, Failure<Infallible>)> {
        let Lease { owner, data } = held;
        owner.finish();
        match invocation.settle_local_in(expected, Ok::<(), Infallible>(())) {
            Ok(()) => Ok(data),
            Err(error) => Err((data, error)),
        }
    }

    #[test]
    fn historical_public_none_failure_is_retained_before_root_raw_or_next() {
        let attempt = Attempt::default();
        let expected = attempt.clone();
        let captured = attempt.clone();
        let root = Cell::new(0);
        let raw = Cell::new(0);
        let finishes = Cell::new(0);
        let completion =
            complete_admission(attempt, &expected, &captured, |invocation, observations| {
                let mut held = None;
                let original = (|| {
                    retain_historical_acquisition(
                        approve_historical_price_only_artifact_checked(
                            invocation,
                            &expected,
                            &captured,
                            std::path::Path::new("not-accessed-by-historical-None"),
                        ),
                        &mut held,
                    )
                    .map_err(|failure| AdmissionSource::Historical(Box::new(failure)))?;
                    root.set(root.get() + 1);
                    raw.set(raw.get() + 1);
                    Ok(())
                })();
                retain_historical_completion(
                    original,
                    held,
                    |held| {
                        finishes.set(finishes.get() + 1);
                        held.finish_local()
                    },
                    &mut observations.historical,
                )
            });
        let observed = ObservedAdmissionFailure {
            completion: Box::new(completion),
        };
        let view = observed.diagnostics();
        let actual = view.historical_failure().unwrap();
        assert_eq!(
            actual.first_kind(),
            Some(HistoricalApprovalFirstKind::Binding)
        );
        assert!(actual.binding_failure().unwrap().body().is_none());
        assert_eq!(view.first_kind(), Some(AdmissionFirstKind::Scope));
        assert!(view.historical_completion().is_none());
        assert_eq!((root.get(), raw.get(), finishes.get()), (0, 0, 0));
        assert!(!format!("{observed:?}").contains("not-accessed"));
    }

    #[test]
    fn historical_foreign_dirty_preflight_visits_both_actual_scopes() {
        let actual = Attempt::default();
        let expected = Attempt::default();
        let captured = Attempt::default();
        actual.gap(Gap::Unbound);
        let invocation = RawInvocation::new(&actual);
        let failure = match approve_historical_price_only_artifact_checked(
            &invocation,
            &expected,
            &captured,
            std::path::Path::new("not-accessed-by-foreign-scopes"),
        ) {
            Err(error) => error,
            Ok(_) => panic!("foreign scopes must refuse"),
        };
        assert_eq!(
            historical_failure_kind(&failure),
            Some(AdmissionFirstKind::Scope)
        );
        assert!(!actual.check() && !expected.check() && !captured.check());
        assert!(
            failure
                .local_results()
                .iter()
                .any(|checks| { checks.expected().is_err() && checks.captured().is_err() })
        );
    }

    #[test]
    fn retained_non_clone_lease_finishes_once_after_original_error() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let mut held = None;
        assert!(
            retain_historical_acquisition(
                Ok::<_, Body>(lease(&attempt, &journal, Ok(()))),
                &mut held
            )
            .is_ok()
        );
        assert!(held.is_some());
        let body = Body("first ordinary error".into());
        let address = body.0.as_ptr();
        let original = Err::<(), _>(body);
        attempt.gap(Gap::Abandoned);
        let mut completion = None;
        let returned = retain_historical_completion(
            original,
            held.take(),
            |held| finish_model(held, &invocation, &attempt),
            &mut completion,
        );
        assert_eq!(returned.err().unwrap().0.as_ptr(), address);
        assert_eq!(&*journal.borrow(), &["historical"]);
        let (data, later) = completion.as_ref().unwrap().as_ref().err().unwrap();
        assert_eq!(data.0.as_ref(), "ordinary lease data");
        assert!(later.body().is_none());
        let returned = retain_historical_completion(
            Ok::<(), Body>(()),
            held.take(),
            |_: Lease| panic!("absent holder must not finish"),
            &mut completion,
        );
        assert!(returned.is_ok());
        assert_eq!(&*journal.borrow(), &["historical"]);
    }

    #[test]
    fn original_some_and_none_failures_survive_whole_later_completion() {
        for with_body in [true, false] {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let journal = Rc::new(RefCell::new(Vec::new()));
            let held = lease(&attempt, &journal, Ok(()));
            let body = Body("stable original body".into());
            let address = body.0.as_ptr();
            let original = if with_body {
                invocation.settle_local_in(&attempt, Err::<(), _>(body))
            } else {
                attempt.gap(Gap::Unbound);
                invocation.settle_local_in(&attempt, Ok::<(), Body>(()))
            };
            let first = original.as_ref().err().unwrap().report().first();
            let mut completion = None;
            let returned = retain_historical_completion(
                original,
                Some(held),
                |held| finish_model(held, &invocation, &attempt),
                &mut completion,
            );
            let failure = returned.err().unwrap();
            assert_eq!(failure.report().first(), first);
            assert_eq!(failure.body().is_some(), with_body);
            if with_body {
                assert_eq!(failure.body().unwrap().0.as_ptr(), address);
            }
            assert!(completion.as_ref().unwrap().is_err());
            assert_eq!(&*journal.borrow(), &["historical"]);
        }
    }

    #[test]
    fn original_ok_and_data_survive_late_result_without_next_policy() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let held = lease(&attempt, &journal, Err(Errno::BadDescriptor));
        let row = Row("original policy success".into());
        let address = row.0.as_ptr();
        let mut completion = None;
        let returned = retain_historical_completion(
            Ok::<_, Body>(row),
            Some(held),
            |held| finish_model(held, &invocation, &attempt),
            &mut completion,
        );
        assert_eq!(returned.ok().unwrap().0.as_ptr(), address);
        let next = Cell::new(0);
        if completion.as_ref().is_some_and(Result::is_ok) {
            next.set(next.get() + 1);
        }
        assert_eq!(next.get(), 0);
        let (data, actual_late) = completion.as_ref().unwrap().as_ref().err().unwrap();
        assert_eq!(data.0.as_ref(), "ordinary lease data");
        assert!(actual_late.body().is_none());
        assert_eq!(attempt.report().unknown(), 1);
    }

    #[test]
    fn replay_checkpoint_keeps_first_failure_and_real_secondary() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let mut first = None;
        assert!(replay_checkpoint(
            || invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())),
            &mut first
        ));
        attempt.gap(Gap::Unbound);
        assert!(!replay_checkpoint(
            || invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())),
            &mut first
        ));
        let later_calls = Cell::new(0);
        assert!(!replay_checkpoint(
            || {
                later_calls.set(later_calls.get() + 1);
                invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()))
            },
            &mut first
        ));
        assert_eq!(later_calls.get(), 0);
        // Genuine public pure error; no invented native Interrupted result.
        let secondary = DailyReplayLimits::new(0).err().unwrap();
        let result = retain_replay_result(Err::<Row, _>(secondary), first);
        let failure = result.err().unwrap();
        match *failure {
            HistoricalReplayFailure::Use {
                first,
                original: Err(error),
            } => {
                assert!(first.body().is_none());
                assert!(matches!(error, DailyReplayError::InvalidLimits));
            }
            _ => panic!("keep primary scope failure and actual secondary"),
        }
        let policy = DailyReplayLimits::new(0).err().unwrap();
        let result = retain_replay_result::<Row, (), _>(Err(policy), None);
        assert!(matches!(
            *result.err().unwrap(),
            HistoricalReplayFailure::Policy(DailyReplayError::InvalidLimits)
        ));
    }

    #[test]
    fn replay_use_error_keeps_returned_ok_and_has_bound_free_debug() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let body = Body("primary secret ordinary model".into());
        let address = body.0.as_ptr();
        let first = invocation
            .settle_local_in(&attempt, Err::<(), _>(body))
            .err()
            .unwrap();
        let original = Row("actual returned ordinary value".into());
        let result = retain_replay_result(Ok::<_, Body>(original), Some(first));
        let failure = result.err().unwrap();
        assert!(!format!("{failure:?}").contains("ordinary"));
        match *failure {
            HistoricalReplayFailure::Use {
                first,
                original: Ok(row),
            } => {
                assert_eq!(first.body().unwrap().0.as_ptr(), address);
                assert_eq!(row.0.as_ref(), "actual returned ordinary value");
            }
            _ => panic!("preserve returned value before scope refusal"),
        }
    }

    #[test]
    fn renewed_unit_check_stops_after_dirt_and_never_masks_prior_error() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let bodies = Cell::new(0);
        let result = checked_historical_unit(
            || invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())),
            || {
                bodies.set(bodies.get() + 1);
                attempt.gap(Gap::Abandoned);
                Ok(())
            },
        );
        assert!(result.is_err());
        let result = checked_historical_unit(
            || invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())),
            || {
                bodies.set(bodies.get() + 1);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(bodies.get(), 1);

        let checks = Cell::new(0);
        let body = Body("prior body".into());
        let address = body.0.as_ptr();
        let result = checked_historical_unit(
            || {
                checks.set(checks.get() + 1);
                Ok::<(), Body>(())
            },
            || Err(body),
        );
        assert_eq!(result.err().unwrap().0.as_ptr(), address);
        assert_eq!(checks.get(), 1);
    }

    #[test]
    fn raw_success_is_saved_before_historical_postcheck_refuses_second() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let mut slots = [None, None];
        let reads = Cell::new(0);
        let checks = Cell::new(0);
        let result = capture_raw_pair_with_checkpoint(
            &mut slots,
            |channel| {
                reads.set(reads.get() + 1);
                Ok(attempt.adopt(
                    Counted {
                        journal: journal.clone(),
                        name: channel.provider(),
                        result: Ok(()),
                    },
                    Role::Source,
                ))
            },
            || {
                checks.set(checks.get() + 1);
                if checks.get() == 2 {
                    attempt.gap(Gap::Unbound);
                }
                invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()))
            },
        );
        assert!(result.is_err());
        assert_eq!(reads.get(), 1);
        assert!(slots[0].is_some() && slots[1].is_none());
        let complete = finish_raw_pair(result, slots, Vec::<Row>::new(), |owner| owner.finish());
        assert!(complete.operation.is_err());
        assert_eq!(&*journal.borrow(), &["kis"]);
    }

    #[test]
    fn borrowed_diagnostics_keep_actual_none_without_finishing_outer_owner() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(
            Counted {
                journal: journal.clone(),
                name: "outer",
                result: Ok(()),
            },
            Role::Source,
        );
        let completed =
            complete_admission_local(&invocation, &attempt, &attempt, Ok(()), |invocation, _| {
                approve_historical_price_only_artifact_checked(
                    invocation,
                    &attempt,
                    &attempt,
                    std::path::Path::new("not-accessed-by-local-None"),
                )
                .map(|_| ())
                .map_err(|failure| AdmissionSource::Historical(Box::new(failure)))
            });
        let observed = LocalObservedAdmissionFailure {
            completion: Box::new(completed),
        };
        assert!(observed.diagnostics().historical_failure().is_some());
        assert!(observed.diagnostics().historical_completion().is_none());
        assert_eq!(attempt.report().live(), 1);
        assert!(journal.borrow().is_empty());
        assert!(!outer.finish());
        assert_eq!(&*journal.borrow(), &["outer"]);
    }

    #[test]
    fn coarse_historical_kind_keeps_late_and_nested_late_absent() {
        assert_eq!(
            historical_kind(Some(HistoricalApprovalFirstKind::Approval), None),
            Some(AdmissionFirstKind::Policy)
        );
        assert_eq!(
            historical_kind(
                Some(HistoricalApprovalFirstKind::Artifact),
                Some(HistoricalV3FirstKind::Artifact)
            ),
            Some(AdmissionFirstKind::Policy)
        );
        for nested in [HistoricalV3FirstKind::Scope, HistoricalV3FirstKind::Binding] {
            assert_eq!(
                historical_kind(Some(HistoricalApprovalFirstKind::Artifact), Some(nested)),
                Some(AdmissionFirstKind::Scope)
            );
        }
        assert_eq!(
            historical_kind(
                Some(HistoricalApprovalFirstKind::Artifact),
                Some(HistoricalV3FirstKind::Late)
            ),
            None
        );
        assert_eq!(
            historical_kind(Some(HistoricalApprovalFirstKind::Late), None),
            None
        );
        assert_eq!(historical_kind(None, None), None);
    }

    #[test]
    fn historical_unwind_only_adds_gap_and_keeps_outer_live() {
        let attempt = Attempt::default();
        let journal = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(
            Counted {
                journal: journal.clone(),
                name: "outer",
                result: Ok(()),
            },
            Role::Source,
        );
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = AdmissionUnwind {
                attempt: attempt.clone(),
                armed: true,
            };
            panic!("ordinary historical unwind");
        }));
        assert!(result.is_err());
        assert!(!attempt.check());
        assert_eq!(attempt.report().live(), 1);
        assert!(journal.borrow().is_empty());
        assert!(!outer.finish());
        assert_eq!(&*journal.borrow(), &["outer"]);
    }
}

#[cfg(test)]
mod execution_context_transport_tests {
    use super::retain_current_result;
    use std::convert::Infallible;

    struct Payload(Box<str>);
    struct LaterCompletion {
        report_present: bool,
    }

    #[test]
    fn late_only_completion_keeps_prior_ok_value() {
        let mut retained = None;
        let value = Payload("retained".into());
        let result = retain_current_result(
            Ok::<_, Infallible>(value),
            LaterCompletion {
                report_present: false,
            },
            &mut retained,
        );

        let value = result.expect("the original successful value remains primary");
        assert_eq!(&*value.0, "retained");
        assert!(
            !retained
                .expect("actual later completion retained")
                .report_present
        );
    }
}
