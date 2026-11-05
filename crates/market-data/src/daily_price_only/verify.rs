//! Offline lineage verification over existing locked Raw snapshots.
//!
//! The result proves committed copy lineage, not provider row semantics or
//! admission. Explicit entitlement/clock context is not installer or DB authority.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::SecondsFormat;
use domain::UtcTimestamp;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};

use super::contract::{canonical_bytes_limited, canonical_file_evidence};
use super::source::{
    CommittedRawBatch, CommittedRawFile, CommittedRawSnapshot, DailyEvidenceError,
};
use super::{
    DailyEvidenceKindV1, DailyFileEvidenceV1Wire, DailyFileOriginV1Wire,
    DailyRequestMetadataV1Wire, DailyWireTimestampV1,
};
use crate::contract::{FetchMode, ResponseKind, SourceFileReference};

pub(super) const MAX_SOURCE_BYTES: u64 = 67_108_864;
const MAX_COPY_LINKS: usize = 16;
const MAX_JSON_DEPTH: usize = 32;

/// Finite diagnostics only: no provider body, path, entitlement or message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyVerificationError {
    Snapshot(DailyEvidenceError),
    InvalidContext,
    NonCredentialed,
    EntitlementMismatch,
    CopyCycle,
    CopyDepth,
    CopyMismatch,
    FutureRetrieval,
    InvalidEvidence,
    InvalidProviderJson,
    ProviderResponse,
    InvalidCalendar,
    InvalidBars,
    InvalidReference,
    InvalidActions,
    NonCashAction,
    SourceCoverageGap,
    SourceConflict,
    CalendarTargetMissing,
    CalendarConflict,
    CalendarCoverageGap,
    NoTradingSession,
    LimitExceeded,
    InvalidIdentity,
    Encoding,
}

impl DailyVerificationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Snapshot(error) => error.code(),
            Self::InvalidContext => "INVALID_CONTEXT",
            Self::NonCredentialed => "NON_CREDENTIALED",
            Self::EntitlementMismatch => "ENTITLEMENT_MISMATCH",
            Self::CopyCycle => "COPY_CYCLE",
            Self::CopyDepth => "COPY_DEPTH",
            Self::CopyMismatch => "COPY_MISMATCH",
            Self::FutureRetrieval => "FUTURE_RETRIEVAL",
            Self::InvalidEvidence => "INVALID_EVIDENCE",
            Self::InvalidProviderJson => "INVALID_PROVIDER_JSON",
            Self::ProviderResponse => "PROVIDER_RESPONSE",
            Self::InvalidCalendar => "INVALID_CALENDAR",
            Self::InvalidBars => "INVALID_BARS",
            Self::InvalidReference => "INVALID_REFERENCE",
            Self::InvalidActions => "INVALID_ACTIONS",
            Self::NonCashAction => "NONCASH_ACTION",
            Self::SourceCoverageGap => "SOURCE_COVERAGE_GAP",
            Self::SourceConflict => "SOURCE_CONFLICT",
            Self::CalendarTargetMissing => "CALENDAR_TARGET_MISSING",
            Self::CalendarConflict => "CALENDAR_CONFLICT",
            Self::CalendarCoverageGap => "CALENDAR_COVERAGE_GAP",
            Self::NoTradingSession => "NO_TRADING_SESSION",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::InvalidIdentity => "INVALID_IDENTITY",
            Self::Encoding => "ENCODING",
        }
    }
}

impl fmt::Display for DailyVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for DailyVerificationError {}

/// Borrowed complete scopes, captured using S2. Construction performs no I/O.
pub struct RawEvidenceSnapshots<'snapshot> {
    scopes: Vec<&'snapshot CommittedRawSnapshot>,
    entitlement_reference: &'snapshot str,
    observed_at: UtcTimestamp,
}

impl<'snapshot> RawEvidenceSnapshots<'snapshot> {
    pub fn new(
        scopes: &[&'snapshot CommittedRawSnapshot],
        entitlement_reference: &'snapshot str,
        observed_at: UtcTimestamp,
    ) -> Result<Self, DailyVerificationError> {
        validate_context(scopes, entitlement_reference, |scope| ScopeFields {
            uid: scope.scope().trusted_raw_owner_uid(),
            provider: scope.scope().provider(),
        })?;
        Ok(Self {
            scopes: scopes.to_vec(),
            entitlement_reference,
            observed_at,
        })
    }

    pub(super) fn scopes(&self) -> &[&'snapshot CommittedRawSnapshot] {
        &self.scopes
    }

    /// Discovery selects only the configured entitlement. Other retained
    /// deliveries remain in the fully verified/budgeted snapshot but cannot
    /// supply evidence. Explicit references and every copy hop still use the
    /// strict entitlement check in resolve_original.
    pub fn matches_entitlement(&self, batch: &CommittedRawBatch) -> bool {
        batch.entry().entitlement_reference.as_deref() == Some(self.entitlement_reference)
    }

    pub(super) fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }

    fn lookup(
        &self,
        reference: &SourceFileReference,
    ) -> Result<CommittedRawFile<'snapshot>, DailyVerificationError> {
        let snapshot = find_snapshot(
            self.scopes.iter().copied(),
            &reference.provider,
            &reference.market,
            |snapshot| (snapshot.scope().provider(), snapshot.scope().market()),
        )?;
        snapshot
            .lookup_exact(reference)
            .map_err(DailyVerificationError::Snapshot)
    }

    pub(super) fn kis_snapshot(
        &self,
    ) -> Result<&'snapshot CommittedRawSnapshot, DailyVerificationError> {
        self.scopes
            .iter()
            .copied()
            .find(|snapshot| snapshot.scope().provider() == "kis")
            .ok_or(DailyVerificationError::InvalidContext)
    }
}

/// Verified lineage with an actual uncopied terminal file. Cannot be deserialized
/// or manufactured from public wire claims. Borrowing keeps snapshots alive.
pub struct ResolvedOriginal<'snapshot> {
    original: CommittedRawFile<'snapshot>,
    evidence: DailyFileEvidenceV1Wire,
    canonical_evidence: Vec<u8>,
    observed_at: UtcTimestamp,
}

impl<'snapshot> ResolvedOriginal<'snapshot> {
    pub fn evidence(&self) -> &DailyFileEvidenceV1Wire {
        &self.evidence
    }

    pub fn canonical_evidence_bytes(&self) -> &[u8] {
        &self.canonical_evidence
    }

    pub fn original_bytes(&self) -> &'snapshot [u8] {
        self.original.bytes()
    }

    pub fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }
}

// BEGIN C12 shared lineage adapters
struct ScopeFields<'s> {
    uid: u32,
    provider: &'s str,
}

fn validate_context<'s, S>(
    scopes: &[S],
    entitlement_reference: &str,
    fields: impl Fn(&S) -> ScopeFields<'s>,
) -> Result<(), DailyVerificationError> {
    if scopes.is_empty() || scopes.len() > 2 || entitlement_reference.trim().is_empty() {
        return Err(DailyVerificationError::InvalidContext);
    }
    let uid = fields(&scopes[0]).uid;
    let mut providers = BTreeSet::new();
    for scope in scopes {
        let scope = fields(scope);
        if scope.uid != uid || !providers.insert(scope.provider) {
            return Err(DailyVerificationError::InvalidContext);
        }
    }
    Ok(())
}

fn find_snapshot<'s, S: Copy>(
    mut scopes: impl Iterator<Item = S>,
    provider: &str,
    market: &str,
    fields: impl Fn(S) -> (&'s str, &'s str),
) -> Result<S, DailyVerificationError> {
    scopes
        .find(|snapshot| {
            let (scope_provider, scope_market) = fields(*snapshot);
            scope_provider == provider && scope_market == market
        })
        .ok_or(DailyVerificationError::Snapshot(
            DailyEvidenceError::ScopeMismatch,
        ))
}

struct BatchFields<'s> {
    provider: &'s str,
    market: &'s str,
    batch_id: domain::BatchId,
    date: domain::TradingDate,
    retrieved_at: UtcTimestamp,
    mode: FetchMode,
    entitlement_reference: Option<&'s str>,
    batch_hash: &'s domain::ContentHash,
    line_hash: &'s domain::ContentHash,
}
impl BatchFields<'_> {
    fn entry(&self) -> &Self {
        self
    }
    fn batch_json_sha256(&self) -> &domain::ContentHash {
        self.batch_hash
    }
    fn manifest_line_sha256(&self) -> &domain::ContentHash {
        self.line_hash
    }
}
struct FileFields<'s> {
    request: &'s crate::contract::RequestMetadata,
    copied_from: Option<&'s SourceFileReference>,
    response_continuation: &'s Option<String>,
}

// Sealed, borrowed projections; neither public implementations nor Native factories.
trait LineageFile<'s> {
    fn batch(&self) -> BatchFields<'s>;
    fn entry(&self) -> FileFields<'s>;
    fn file_name(&self) -> &'s str;
    fn kind(&self) -> ResponseKind;
    fn content_hash(&self) -> &'s domain::ContentHash;
    fn size_bytes(&self) -> u64;
    fn bytes(&self) -> &'s [u8];
}
trait LineageBackend<'s> {
    type File: LineageFile<'s>;
    type Error: From<DailyVerificationError>;
    type Output;
    fn lookup(&self, reference: &SourceFileReference) -> Result<Self::File, Self::Error>;
    fn entitlement_reference(&self) -> &str;
    fn observed_at(&self) -> UtcTimestamp;
    fn output(
        &self,
        original: Self::File,
        evidence: DailyFileEvidenceV1Wire,
        canonical_evidence: Vec<u8>,
        observed_at: UtcTimestamp,
    ) -> Self::Output;
}

impl<'s> LineageFile<'s> for CommittedRawFile<'s> {
    fn batch(&self) -> BatchFields<'s> {
        let batch = CommittedRawFile::batch(self);
        let entry = batch.entry();
        BatchFields {
            provider: &entry.provider,
            market: &entry.market,
            batch_id: entry.batch_id,
            date: entry.date,
            retrieved_at: entry.retrieved_at,
            mode: entry.mode,
            entitlement_reference: entry.entitlement_reference.as_deref(),
            batch_hash: batch.batch_json_sha256(),
            line_hash: batch.manifest_line_sha256(),
        }
    }
    fn entry(&self) -> FileFields<'s> {
        let entry = CommittedRawFile::entry(self);
        FileFields {
            request: &entry.request,
            copied_from: entry.copied_from.as_ref(),
            response_continuation: &entry.response_continuation,
        }
    }
    fn file_name(&self) -> &'s str {
        CommittedRawFile::file_name(self)
    }
    fn kind(&self) -> ResponseKind {
        CommittedRawFile::kind(self)
    }
    fn content_hash(&self) -> &'s domain::ContentHash {
        CommittedRawFile::content_hash(self)
    }
    fn size_bytes(&self) -> u64 {
        CommittedRawFile::size_bytes(self)
    }
    fn bytes(&self) -> &'s [u8] {
        CommittedRawFile::bytes(self)
    }
}
impl<'s> LineageBackend<'s> for RawEvidenceSnapshots<'s> {
    type File = CommittedRawFile<'s>;
    type Error = DailyVerificationError;
    type Output = ResolvedOriginal<'s>;
    fn lookup(&self, reference: &SourceFileReference) -> Result<Self::File, Self::Error> {
        RawEvidenceSnapshots::lookup(self, reference)
    }
    fn entitlement_reference(&self) -> &str {
        self.entitlement_reference
    }
    fn observed_at(&self) -> UtcTimestamp {
        self.observed_at
    }
    fn output(
        &self,
        original: Self::File,
        evidence: DailyFileEvidenceV1Wire,
        canonical_evidence: Vec<u8>,
        observed_at: UtcTimestamp,
    ) -> Self::Output {
        ResolvedOriginal {
            original,
            evidence,
            canonical_evidence,
            observed_at,
        }
    }
}
// END C12 shared lineage adapters

pub fn resolve_original<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    reference: &SourceFileReference,
) -> Result<ResolvedOriginal<'snapshot>, DailyVerificationError> {
    resolve_original_with(snapshots, reference)
}

fn resolve_original_with<'s, B: LineageBackend<'s>>(
    snapshots: &B,
    reference: &SourceFileReference,
) -> Result<B::Output, B::Error> {
    let mut current = snapshots.lookup(reference)?;
    let mut origins: Vec<DailyFileOriginV1Wire> = Vec::new();
    let mut identities = BTreeSet::new();
    let first_bytes = current.bytes();
    loop {
        let batch = current.batch();
        let entry = batch.entry();
        let file = current.entry();
        if !identities.insert((
            entry.provider,
            entry.market,
            entry.batch_id,
            current.file_name(),
        )) {
            return Err(DailyVerificationError::CopyCycle.into());
        }
        if entry.mode != FetchMode::Credentialed || file.request.mode != FetchMode::Credentialed {
            return Err(DailyVerificationError::NonCredentialed.into());
        }
        if entry.entitlement_reference != Some(snapshots.entitlement_reference()) {
            return Err(DailyVerificationError::EntitlementMismatch.into());
        }
        if entry.retrieved_at > snapshots.observed_at() {
            return Err(DailyVerificationError::FutureRetrieval.into());
        }
        if current.bytes() != first_bytes {
            return Err(DailyVerificationError::CopyMismatch.into());
        }
        let origin = file_origin_with(&current)?;
        if origins.first().is_some_and(|first| {
            first.kind != origin.kind
                || first.wire_sha256 != origin.wire_sha256
                || first.size_bytes != origin.size_bytes
                || first.request != origin.request
                || first.response_continuation != origin.response_continuation
                || origins.last().is_some_and(|prior| {
                    prior.retrieved_at.as_timestamp() < origin.retrieved_at.as_timestamp()
                })
        }) {
            return Err(DailyVerificationError::CopyMismatch.into());
        }
        origins.push(origin);
        let Some(next) = file.copied_from else {
            break;
        };
        if origins.len() > MAX_COPY_LINKS {
            return Err(DailyVerificationError::CopyDepth.into());
        }
        current = snapshots.lookup(next)?;
    }
    let first = origins.remove(0);
    let original_retrieved_at = origins
        .last()
        .map_or(first.retrieved_at, |last| last.retrieved_at);
    let evidence = DailyFileEvidenceV1Wire {
        provider: first.provider,
        market: first.market,
        batch_id: first.batch_id,
        batch_date: first.batch_date,
        batch_json_sha256: first.batch_json_sha256,
        manifest_line_sha256: first.manifest_line_sha256,
        file_name: first.file_name,
        kind: first.kind,
        wire_sha256: first.wire_sha256,
        size_bytes: first.size_bytes,
        request: first.request,
        response_continuation: first.response_continuation,
        retrieved_at: first.retrieved_at,
        retrieval_precision: first.retrieval_precision,
        original_retrieved_at,
        original_retrieval_precision: "batch".to_owned(),
        copied_from: origins,
    };
    let (evidence, canonical_evidence) =
        canonical_file_evidence(&evidence).map_err(|_| DailyVerificationError::InvalidEvidence)?;
    Ok(snapshots.output(
        current,
        evidence,
        canonical_evidence,
        snapshots.observed_at(),
    ))
}

fn file_origin_with<'s>(
    file: &impl LineageFile<'s>,
) -> Result<DailyFileOriginV1Wire, DailyVerificationError> {
    let batch = file.batch();
    let entry = batch.entry();
    let metadata = file.entry();
    let kind = match file.kind() {
        ResponseKind::Bars => DailyEvidenceKindV1::Bars,
        ResponseKind::Reference => DailyEvidenceKindV1::Reference,
        ResponseKind::Calendar => DailyEvidenceKindV1::Calendar,
        ResponseKind::CorporateActions => DailyEvidenceKindV1::CorporateActions,
        _ => return Err(DailyVerificationError::InvalidEvidence),
    };
    let mut request = DailyRequestMetadataV1Wire {
        endpoint: metadata.request.endpoint.clone(),
        query: metadata.request.query.clone(),
        headers: metadata.request.headers.clone(),
        mode: "credentialed".to_owned(),
    };
    request
        .query
        .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    request
        .headers
        .sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let retrieved_at = DailyWireTimestampV1::parse(
        &entry
            .retrieved_at
            .as_datetime()
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
    )
    .map_err(|_| DailyVerificationError::InvalidEvidence)?;
    Ok(DailyFileOriginV1Wire {
        provider: entry.provider.to_owned(),
        market: entry.market.to_owned(),
        batch_id: entry.batch_id,
        batch_date: entry.date,
        batch_json_sha256: batch.batch_json_sha256().clone(),
        manifest_line_sha256: batch.manifest_line_sha256().clone(),
        file_name: file.file_name().to_owned(),
        kind,
        wire_sha256: file.content_hash().clone(),
        size_bytes: file.size_bytes(),
        request,
        response_continuation: (*metadata.response_continuation).clone(),
        retrieved_at,
        retrieval_precision: "batch".to_owned(),
    })
}

// BEGIN C12 checked lineage
#[cfg(target_os = "linux")]
pub use checked_lineage::{
    CheckedLineageData, CheckedLineageFailure, CheckedLineageFirstKind, CheckedLineageLocal,
    CheckedLineageScope, CheckedRawEvidenceSnapshots, CheckedResolvedOriginal,
    CheckedResolvedOriginalView, resolve_original_checked,
};

#[cfg(target_os = "linux")]
mod checked_lineage {
    use super::super::source::{
        CheckedRawFileView, CheckedRawSnapshot, CheckedRawSnapshotUseFailure,
        CheckedRawSnapshotView,
    };
    use super::*;
    use crate::{
        checked_close::{Attempt, Failure, Gap, Report},
        raw_custody::RawInvocation,
    };
    use std::convert::Infallible;

    type Unit = Result<(), Failure<Infallible>>;
    enum UseCheck<E> {
        ViewReturned,
        Failed(E),
    }

    // Visits every actual holder even after earlier scope/holder errors.
    fn gather_views<S, V, E>(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        captured: &Attempt,
        scopes: &[S],
        mut visit: impl FnMut(&S) -> Result<V, E>,
    ) -> (Unit, Unit, Vec<UseCheck<E>>, Vec<V>) {
        let expected = invocation.settle_local_in(expected, Ok::<(), Infallible>(()));
        let captured = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
        let mut checks = Vec::with_capacity(scopes.len());
        let mut views = Vec::with_capacity(scopes.len());
        for scope in scopes {
            match visit(scope) {
                Ok(view) => {
                    views.push(view);
                    checks.push(UseCheck::ViewReturned);
                }
                Err(error) => checks.push(UseCheck::Failed(error)),
            }
        }
        (expected, captured, checks, views)
    }

    pub struct CheckedLineageScope {
        expected: Unit,
        captured: Unit,
        holders: Vec<UseCheck<CheckedRawSnapshotUseFailure>>,
    }
    impl CheckedLineageScope {
        pub fn expected(&self) -> &Unit {
            &self.expected
        }
        pub fn captured(&self) -> &Unit {
            &self.captured
        }
        pub fn use_failure(&self, index: usize) -> Option<&CheckedRawSnapshotUseFailure> {
            match self.holders.get(index) {
                Some(UseCheck::Failed(error)) => Some(error),
                _ => None,
            }
        }
        pub fn checked_holder_count(&self) -> usize {
            self.holders.len()
        }
        pub fn is_clean(&self) -> bool {
            self.expected.is_ok()
                & self.captured.is_ok()
                & self
                    .holders
                    .iter()
                    .all(|check| matches!(check, UseCheck::ViewReturned))
        }
        fn source_report(&self) -> Option<&Report> {
            if let Err(error) = &self.expected {
                return Some(error.report());
            }
            if let Err(error) = &self.captured {
                return Some(error.report());
            }
            for check in &self.holders {
                if let UseCheck::Failed(error) = check {
                    return error.source_report();
                }
            }
            None
        }
    }
    impl fmt::Debug for CheckedLineageScope {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedLineageScope")
                .field("holders", &self.holders.len())
                .field("clean", &self.is_clean())
                .finish_non_exhaustive()
        }
    }
    pub struct CheckedLineageLocal {
        before: Option<CheckedLineageScope>,
        after: CheckedLineageScope,
        local: Unit,
    }
    impl CheckedLineageLocal {
        pub fn before(&self) -> Option<&CheckedLineageScope> {
            self.before.as_ref()
        }
        pub fn after(&self) -> &CheckedLineageScope {
            &self.after
        }
        pub fn local(&self) -> &Unit {
            &self.local
        }
        pub fn is_clean(&self) -> bool {
            self.before
                .as_ref()
                .is_none_or(CheckedLineageScope::is_clean)
                & self.after.is_clean()
                & self.local.is_ok()
        }
    }
    impl fmt::Debug for CheckedLineageLocal {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedLineageLocal")
                .field("clean", &self.is_clean())
                .finish_non_exhaustive()
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CheckedLineageFirstKind {
        Scope,
        Policy,
    }
    enum First {
        Scope(Box<CheckedLineageScope>),
        Policy(DailyVerificationError),
    }

    /// Whole original source and actual later local results. No authority constructor.
    pub struct CheckedLineageFailure {
        first: Option<First>,
        retained: Option<Box<CheckedLineageData>>,
        later: Vec<CheckedLineageLocal>,
    }
    impl From<DailyVerificationError> for CheckedLineageFailure {
        fn from(error: DailyVerificationError) -> Self {
            Self {
                first: Some(First::Policy(error)),
                retained: None,
                later: Vec::new(),
            }
        }
    }
    impl CheckedLineageFailure {
        pub fn first_kind(&self) -> Option<CheckedLineageFirstKind> {
            self.first.as_ref().map(|first| match first {
                First::Scope(_) => CheckedLineageFirstKind::Scope,
                First::Policy(_) => CheckedLineageFirstKind::Policy,
            })
        }
        pub fn source_report(&self) -> Option<&Report> {
            match &self.first {
                Some(First::Scope(scope)) => scope.source_report(),
                _ => None,
            }
        }
        pub fn policy_source(&self) -> Option<&DailyVerificationError> {
            match &self.first {
                Some(First::Policy(error)) => Some(error),
                _ => None,
            }
        }
        pub fn scope_failure(&self) -> Option<&CheckedLineageScope> {
            match &self.first {
                Some(First::Scope(scope)) => Some(scope),
                _ => None,
            }
        }
        pub fn retained(&self) -> Option<&CheckedLineageData> {
            self.retained.as_deref()
        }
        pub fn later(&self) -> &[CheckedLineageLocal] {
            &self.later
        }
    }
    impl fmt::Debug for CheckedLineageFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedLineageFailure")
                .field("first_kind", &self.first_kind())
                .field("prior_ok", &self.retained.is_some())
                .field("later_count", &self.later.len())
                .finish_non_exhaustive()
        }
    }

    enum Dispatch<T, E, C> {
        Clean(T, C),
        Refused(Result<T, E>, C),
    }
    fn dispatch<T, E, C>(
        original: Result<T, E>,
        completion: C,
        clean: impl FnOnce(&C) -> bool,
    ) -> Dispatch<T, E, C> {
        if original.is_ok() && clean(&completion) {
            match original {
                Ok(value) => Dispatch::Clean(value, completion),
                Err(_) => unreachable!(),
            }
        } else {
            Dispatch::Refused(original, completion)
        }
    }

    struct Unwind {
        captured: Attempt,
        armed: bool,
    }
    impl Drop for Unwind {
        fn drop(&mut self) {
            if self.armed {
                self.captured.gap(Gap::Abandoned);
            }
        }
    }

    #[expect(
        clippy::result_large_err,
        reason = "Whole original and cleanup diagnostics stay typed without extra allocation."
    )]
    fn guard<'s, 'i, 'a>(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        captured: &Attempt,
        scopes: &[&'s CheckedRawSnapshot<'i, 'a>],
    ) -> (CheckedLineageScope, Vec<CheckedRawSnapshotView<'s>>) {
        let (expected, captured, holders, views) =
            gather_views(invocation, expected, captured, scopes, |scope| {
                (*scope).view_in(expected, captured)
            });
        (
            CheckedLineageScope {
                expected,
                captured,
                holders,
            },
            views,
        )
    }

    // Original Result and all returned data remain outside every unit T.
    fn run_scoped<'s, 'i, 'a, T>(
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        captured: &Attempt,
        scopes: &[&'s CheckedRawSnapshot<'i, 'a>],
        operation: impl FnOnce(Vec<CheckedRawSnapshotView<'s>>) -> Result<T, CheckedLineageFailure>,
        retain: impl FnOnce(T) -> Option<CheckedLineageData>,
    ) -> Result<(T, CheckedLineageLocal), CheckedLineageFailure> {
        let mut unwind = Unwind {
            captured: captured.clone(),
            armed: true,
        };
        let (before, views) = guard(invocation, expected, captured, scopes);
        let (original, before) = if before.is_clean() {
            (operation(views), Some(before))
        } else {
            (
                Err(CheckedLineageFailure {
                    first: Some(First::Scope(Box::new(before))),
                    retained: None,
                    later: Vec::new(),
                }),
                None,
            )
        };
        if original.is_err() && captured.check() {
            captured.gap(Gap::Abandoned);
        }
        let (after, _) = guard(invocation, expected, captured, scopes);
        let local = invocation.settle_local_in(captured, Ok::<(), Infallible>(()));
        let completion = CheckedLineageLocal {
            before,
            after,
            local,
        };
        unwind.armed = false;
        match dispatch(original, completion, CheckedLineageLocal::is_clean) {
            Dispatch::Clean(value, completion) => Ok((value, completion)),
            Dispatch::Refused(Err(mut error), completion) => {
                error.later.push(completion);
                Err(error)
            }
            Dispatch::Refused(Ok(value), completion) => Err(CheckedLineageFailure {
                first: None,
                retained: retain(value).map(Box::new),
                later: vec![completion],
            }),
        }
    }

    pub struct CheckedRawEvidenceSnapshots<'s, 'i, 'a> {
        invocation: &'i RawInvocation<'a>,
        expected: Attempt,
        captured: Attempt,
        scopes: Vec<&'s CheckedRawSnapshot<'i, 'a>>,
        entitlement_reference: &'s str,
        observed_at: UtcTimestamp,
        construction: CheckedLineageLocal,
    }
    fn view_scope<'s>(view: &CheckedRawSnapshotView<'s>) -> ScopeFields<'s> {
        // Only genuine live success wrappers are allowed into the constructor.
        let scope = view
            .scope()
            .expect("successful checked snapshot retains its scope");
        ScopeFields {
            uid: scope.trusted_raw_owner_uid(),
            provider: scope.provider(),
        }
    }
    fn view_provider<'s>(view: &CheckedRawSnapshotView<'s>) -> (&'s str, &'s str) {
        let scope = view
            .scope()
            .expect("successful checked snapshot retains its scope");
        (scope.provider(), scope.market())
    }
    impl<'s, 'i, 'a> CheckedRawEvidenceSnapshots<'s, 'i, 'a> {
        pub fn new_in(
            invocation: &'i RawInvocation<'a>,
            expected: &Attempt,
            captured: &Attempt,
            scopes: &[&'s CheckedRawSnapshot<'i, 'a>],
            entitlement_reference: &'s str,
            observed_at: UtcTimestamp,
        ) -> Result<Self, CheckedLineageFailure> {
            let ((), construction) = run_scoped(
                invocation,
                expected,
                captured,
                scopes,
                |views| {
                    validate_context(&views, entitlement_reference, view_scope)?;
                    Ok(())
                },
                |_| None,
            )?;
            Ok(Self {
                invocation,
                expected: expected.clone(),
                captured: captured.clone(),
                scopes: scopes.to_vec(),
                entitlement_reference,
                observed_at,
                construction,
            })
        }
        pub fn construction(&self) -> &CheckedLineageLocal {
            &self.construction
        }
        pub fn snapshot_view_in(
            &self,
            provider: &str,
            market: &str,
        ) -> Result<CheckedRawSnapshotView<'s>, CheckedLineageFailure> {
            self.snapshot_observed_view_in(provider, market)
                .map(|(view, _)| view)
        }
        /// Returns the actual local observation belonging to this view call.
        pub fn snapshot_observed_view_in(
            &self,
            provider: &str,
            market: &str,
        ) -> Result<(CheckedRawSnapshotView<'s>, CheckedLineageLocal), CheckedLineageFailure>
        {
            let (view, local) = run_scoped(
                self.invocation,
                &self.expected,
                &self.captured,
                &self.scopes,
                |views| {
                    let (index, _) =
                        find_snapshot(views.iter().enumerate(), provider, market, |(_, view)| {
                            view_provider(view)
                        })?;
                    Ok(views
                        .into_iter()
                        .nth(index)
                        .expect("selected genuine checked view"))
                },
                |_| None,
            )?;
            Ok((view, local))
        }
        pub fn into_diagnostics(self) -> CheckedLineageLocal {
            self.construction
        }
        /// Descriptive entitlement comparison; a genuine guarded batch supplies the entry.
        pub fn matches_entitlement(
            &self,
            batch: &super::super::source::CheckedRawBatchView<'_>,
        ) -> bool {
            batch.entry().entitlement_reference.as_deref() == Some(self.entitlement_reference)
        }
    }

    impl<'s> LineageFile<'s> for CheckedRawFileView<'s> {
        fn batch(&self) -> BatchFields<'s> {
            let batch = CheckedRawFileView::batch(self);
            let entry = batch.entry();
            BatchFields {
                provider: &entry.provider,
                market: &entry.market,
                batch_id: entry.batch_id,
                date: entry.date,
                retrieved_at: entry.retrieved_at,
                mode: entry.mode,
                entitlement_reference: entry.entitlement_reference.as_deref(),
                batch_hash: batch.batch_json_sha256(),
                line_hash: batch.manifest_line_sha256(),
            }
        }
        fn entry(&self) -> FileFields<'s> {
            let entry = CheckedRawFileView::entry(self);
            FileFields {
                request: &entry.request,
                copied_from: entry.copied_from.as_ref(),
                response_continuation: &entry.response_continuation,
            }
        }
        fn file_name(&self) -> &'s str {
            CheckedRawFileView::file_name(self)
        }
        fn kind(&self) -> ResponseKind {
            CheckedRawFileView::kind(self)
        }
        fn content_hash(&self) -> &'s domain::ContentHash {
            CheckedRawFileView::content_hash(self)
        }
        fn size_bytes(&self) -> u64 {
            CheckedRawFileView::size_bytes(self)
        }
        fn bytes(&self) -> &'s [u8] {
            CheckedRawFileView::bytes(self)
        }
    }
    pub(super) struct Pending<'s> {
        original: CheckedRawFileView<'s>,
        data: CheckedLineageData,
    }
    pub struct CheckedLineageData {
        evidence: DailyFileEvidenceV1Wire,
        canonical_evidence: Vec<u8>,
        observed_at: UtcTimestamp,
        local: Option<CheckedLineageLocal>,
    }
    impl CheckedLineageData {
        pub fn evidence(&self) -> &DailyFileEvidenceV1Wire {
            &self.evidence
        }
        pub fn canonical_evidence_bytes(&self) -> &[u8] {
            &self.canonical_evidence
        }
        pub fn observed_at(&self) -> UtcTimestamp {
            self.observed_at
        }
        pub fn local(&self) -> Option<&CheckedLineageLocal> {
            self.local.as_ref()
        }
    }
    impl fmt::Debug for CheckedLineageData {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CheckedLineageData")
                .field("canonical_bytes", &self.canonical_evidence.len())
                .field("local_present", &self.local.is_some())
                .finish_non_exhaustive()
        }
    }
    impl<'s> LineageBackend<'s> for CheckedRawEvidenceSnapshots<'s, '_, '_> {
        type File = CheckedRawFileView<'s>;
        type Error = CheckedLineageFailure;
        type Output = Pending<'s>;
        fn lookup(&self, reference: &SourceFileReference) -> Result<Self::File, Self::Error> {
            let (file, _) = run_scoped(
                self.invocation,
                &self.expected,
                &self.captured,
                &self.scopes,
                |views| {
                    let view = find_snapshot(
                        views.iter(),
                        &reference.provider,
                        &reference.market,
                        view_provider,
                    )?;
                    Ok(view
                        .lookup_exact(reference)
                        .map_err(DailyVerificationError::Snapshot)?)
                },
                |_| None,
            )?;
            Ok(file)
        }
        fn entitlement_reference(&self) -> &str {
            self.entitlement_reference
        }
        fn observed_at(&self) -> UtcTimestamp {
            self.observed_at
        }
        fn output(
            &self,
            original: Self::File,
            evidence: DailyFileEvidenceV1Wire,
            canonical_evidence: Vec<u8>,
            observed_at: UtcTimestamp,
        ) -> Self::Output {
            Pending {
                original,
                data: CheckedLineageData {
                    evidence,
                    canonical_evidence,
                    observed_at,
                    local: None,
                },
            }
        }
    }
    pub struct CheckedResolvedOriginal<'set, 's, 'i, 'a> {
        snapshots: &'set CheckedRawEvidenceSnapshots<'s, 'i, 'a>,
        original: CheckedRawFileView<'s>,
        data: CheckedLineageData,
    }
    pub struct CheckedResolvedOriginalView<'v, 's> {
        data: &'v CheckedLineageData,
        bytes: &'s [u8],
        local: CheckedLineageLocal,
    }
    impl<'v, 's> CheckedResolvedOriginalView<'v, 's> {
        pub fn evidence(&self) -> &DailyFileEvidenceV1Wire {
            &self.data.evidence
        }
        pub fn canonical_evidence_bytes(&self) -> &[u8] {
            &self.data.canonical_evidence
        }
        pub fn observed_at(&self) -> UtcTimestamp {
            self.data.observed_at
        }
        pub fn original_bytes(&self) -> &'s [u8] {
            self.bytes
        }
        pub fn local(&self) -> &CheckedLineageLocal {
            &self.local
        }
        pub(crate) fn into_local(self) -> CheckedLineageLocal {
            self.local
        }
        /// Consumes only this call's diagnostic observations, never a live holder.
        pub fn into_diagnostics(self) -> CheckedLineageLocal {
            self.into_local()
        }
    }
    impl<'s> CheckedResolvedOriginal<'_, 's, '_, '_> {
        pub fn view_in(
            &self,
        ) -> Result<CheckedResolvedOriginalView<'_, 's>, CheckedLineageFailure> {
            let snapshots = self.snapshots;
            let ((), local) = run_scoped(
                snapshots.invocation,
                &snapshots.expected,
                &snapshots.captured,
                &snapshots.scopes,
                |_| Ok(()),
                |_| None,
            )?;
            Ok(CheckedResolvedOriginalView {
                data: &self.data,
                bytes: self.original.bytes(),
                local,
            })
        }
        pub fn retain_diagnostics(self) -> CheckedLineageData {
            self.data
        }
    }
    pub fn resolve_original_checked<'set, 's, 'i, 'a>(
        snapshots: &'set CheckedRawEvidenceSnapshots<'s, 'i, 'a>,
        reference: &SourceFileReference,
    ) -> Result<CheckedResolvedOriginal<'set, 's, 'i, 'a>, CheckedLineageFailure> {
        let (mut pending, local) = run_scoped(
            snapshots.invocation,
            &snapshots.expected,
            &snapshots.captured,
            &snapshots.scopes,
            |_| resolve_original_with(snapshots, reference),
            |pending| Some(pending.data),
        )?;
        pending.data.local = Some(local);
        Ok(CheckedResolvedOriginal {
            snapshots,
            original: pending.original,
            data: pending.data,
        })
    }
    #[cfg(test)]
    mod scope_tests {
        use super::*;
        use crate::checked_close::{Close, Errno, Role};
        use std::{
            cell::Cell,
            panic::{AssertUnwindSafe, catch_unwind},
            rc::Rc,
        };
        struct Body(Box<str>);
        struct Counted(Rc<Cell<usize>>);
        impl Close for Counted {
            fn close(self) -> Result<(), Errno> {
                self.0.set(self.0.get() + 1);
                Ok(())
            }
        }
        #[test]
        fn scope_guard_visits_every_foreign_holder_even_when_dirty() {
            let actual = Attempt::default();
            let expected = Attempt::default();
            let captured = Attempt::default();
            let holders = [Attempt::default(), Attempt::default()];
            actual.gap(Gap::Unbound);
            let visits = Cell::new(0);
            let (e, c, checked, views) = gather_views(
                &RawInvocation::new(&actual),
                &expected,
                &captured,
                &holders,
                |holder| {
                    visits.set(visits.get() + 1);
                    RawInvocation::new(&actual).settle_local_in(holder, Ok::<(), Infallible>(()))
                },
            );
            assert_eq!(visits.get(), 2);
            assert!(e.is_err() && c.is_err() && views.is_empty());
            assert!(checked.iter().all(|c| matches!(c, UseCheck::Failed(_))));
            assert!(!actual.check() && !expected.check() && !captured.check());
            assert!(holders.iter().all(|holder| !holder.check()));
        }
        #[test]
        fn dispatch_keeps_whole_non_debug_error_and_late_ok_separate() {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let body = Body("first ordinary body".into());
            let address = body.0.as_ptr();
            attempt.gap(Gap::Unbound);
            let later = invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()));
            match dispatch(Err::<(), _>(body), later, Result::is_ok) {
                Dispatch::Refused(Err(body), later) => {
                    assert_eq!(body.0.as_ptr(), address);
                    assert!(later.is_err());
                }
                _ => panic!("retain original error"),
            }
            let later = invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()));
            match dispatch(Ok::<_, Body>(Vec::<u8>::new()), later, Result::is_ok) {
                Dispatch::Refused(Ok(rows), later) => {
                    assert!(rows.is_empty());
                    assert!(later.is_err());
                }
                _ => panic!("late-only refusal retains actual empty success"),
            }
            let clean = Attempt::default();
            let invocation = RawInvocation::new(&clean);
            let failure = match CheckedRawEvidenceSnapshots::new_in(
                &invocation,
                &clean,
                &clean,
                &[],
                "ordinary-license",
                UtcTimestamp::parse_rfc3339("2026-10-15T00:00:00Z").unwrap(),
            ) {
                Err(failure) => failure,
                Ok(_) => panic!("empty set policy"),
            };
            assert_eq!(failure.first_kind(), Some(CheckedLineageFirstKind::Policy));
            assert!(failure.source_report().is_none());
            assert!(failure.later().iter().any(|later| later.local().is_err()));
            assert!(!format!("{failure:?}").contains("ordinary-license"));
        }
        #[test]
        fn lineage_unwind_only_marks_gap_and_keeps_outer_owner_live() {
            let attempt = Attempt::default();
            let calls = Rc::new(Cell::new(0));
            let owner = attempt.adopt(Counted(calls.clone()), Role::Source);
            let result = catch_unwind(AssertUnwindSafe(|| {
                let _guard = Unwind {
                    captured: attempt.clone(),
                    armed: true,
                };
                panic!("ordinary unwind");
            }));
            assert!(result.is_err());
            assert_eq!(calls.get(), 0);
            assert_eq!(attempt.report().live(), 1);
            assert!(!attempt.check());
            assert!(!owner.finish());
            assert_eq!(calls.get(), 1);
        }
    }
}
// END C12 checked lineage

/// Reject duplicate keys before reading their second value, including escaped
/// aliases. Provider numbers are not restricted to D4's unsigned wire scalars.
pub(super) fn parse_provider_json(bytes: &[u8]) -> Result<Value, DailyVerificationError> {
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SOURCE_BYTES {
        return Err(DailyVerificationError::LimitExceeded);
    }
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let value = JsonSeed(MAX_JSON_DEPTH)
        .deserialize(&mut parser)
        .map_err(|_| DailyVerificationError::InvalidProviderJson)?;
    parser
        .end()
        .map_err(|_| DailyVerificationError::InvalidProviderJson)?;
    Ok(value)
}

struct JsonSeed(usize);

impl<'de> DeserializeSeed<'de> for JsonSeed {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for JsonSeed {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded provider JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("INVALID_NUMBER"))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        if self.0 == 0 {
            return Err(serde::de::Error::custom("JSON_DEPTH"));
        }
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(JsonSeed(self.0 - 1))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        if self.0 == 0 {
            return Err(serde::de::Error::custom("JSON_DEPTH"));
        }
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            match values.entry(key) {
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(serde::de::Error::custom("DUPLICATE_JSON_KEY"));
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(map.next_value_seed(JsonSeed(self.0 - 1))?);
                }
            }
        }
        Ok(Value::Object(values.into_iter().collect()))
    }
}

/// Sorting does not depend on serde_json's optional preserve_order feature.
struct SortedJson<'a>(&'a Value);

impl Serialize for SortedJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                let mut map = serializer.serialize_map(Some(keys.len()))?;
                for key in keys {
                    map.serialize_entry(key, &SortedJson(&object[key]))?;
                }
                map.end()
            }
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&SortedJson(value))?;
                }
                seq.end()
            }
            scalar => scalar.serialize(serializer),
        }
    }
}

pub(super) fn canonical_json(value: &Value) -> Result<Vec<u8>, DailyVerificationError> {
    canonical_bytes_limited(&SortedJson(value), MAX_SOURCE_BYTES)
        .map_err(|_| DailyVerificationError::Encoding)
}

#[cfg(test)]
mod lineage_policy_tests {
    use super::*;
    use crate::contract::RequestMetadata;
    use domain::{BatchId, ContentHash, TradingDate};
    use uuid::Uuid;

    // Ordinary nonClone/nonDebug rows, never Native files/entries/snapshots.
    struct Row {
        id: BatchId,
        provider: String,
        market: String,
        name: String,
        date: TradingDate,
        retrieved: UtcTimestamp,
        mode: FetchMode,
        kind: ResponseKind,
        entitlement: String,
        request: RequestMetadata,
        next: Option<SourceFileReference>,
        marker: Option<String>,
        hash: ContentHash,
        body: Box<[u8]>,
    }
    fn row(id: u128) -> Row {
        Row {
            id: BatchId::from_uuid(Uuid::from_u128(id)),
            provider: "kis".into(),
            market: "kr".into(),
            name: "bars.json".into(),
            date: TradingDate::parse("2026-10-13").unwrap(),
            retrieved: UtcTimestamp::parse_rfc3339("2026-10-14T01:02:03Z").unwrap(),
            mode: FetchMode::Credentialed,
            kind: ResponseKind::Bars,
            entitlement: "ordinary-license".into(),
            request: RequestMetadata {
                endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".into(),
                query: Vec::new(),
                headers: Vec::new(),
                mode: FetchMode::Credentialed,
            },
            next: None,
            marker: None,
            hash: ContentHash::from_bytes(b"body"),
            body: b"body".to_vec().into_boxed_slice(),
        }
    }
    fn reference(row: &Row) -> SourceFileReference {
        SourceFileReference {
            provider: row.provider.clone(),
            market: row.market.clone(),
            batch_id: row.id,
            file_name: row.name.clone(),
            content_hash: row.hash.clone(),
            retrieved_at: row.retrieved,
        }
    }
    impl<'s> LineageFile<'s> for &'s Row {
        fn batch(&self) -> BatchFields<'s> {
            BatchFields {
                provider: &self.provider,
                market: &self.market,
                batch_id: self.id,
                date: self.date,
                retrieved_at: self.retrieved,
                mode: self.mode,
                entitlement_reference: Some(&self.entitlement),
                batch_hash: &self.hash,
                line_hash: &self.hash,
            }
        }
        fn entry(&self) -> FileFields<'s> {
            FileFields {
                request: &self.request,
                copied_from: self.next.as_ref(),
                response_continuation: &self.marker,
            }
        }
        fn file_name(&self) -> &'s str {
            &self.name
        }
        fn kind(&self) -> ResponseKind {
            self.kind
        }
        fn content_hash(&self) -> &'s ContentHash {
            &self.hash
        }
        fn size_bytes(&self) -> u64 {
            self.body.len() as u64
        }
        fn bytes(&self) -> &'s [u8] {
            &self.body
        }
    }
    struct Model<'s> {
        rows: &'s [Row],
        observed: UtcTimestamp,
    }
    impl<'s> LineageBackend<'s> for Model<'s> {
        type File = &'s Row;
        type Error = DailyVerificationError;
        type Output = (&'s Row, DailyFileEvidenceV1Wire, Vec<u8>);
        fn lookup(&self, reference: &SourceFileReference) -> Result<Self::File, Self::Error> {
            self.rows
                .iter()
                .find(|row| row.id == reference.batch_id)
                .ok_or(DailyVerificationError::InvalidEvidence)
        }
        fn entitlement_reference(&self) -> &str {
            "ordinary-license"
        }
        fn observed_at(&self) -> UtcTimestamp {
            self.observed
        }
        fn output(
            &self,
            original: Self::File,
            evidence: DailyFileEvidenceV1Wire,
            canonical: Vec<u8>,
            _: UtcTimestamp,
        ) -> Self::Output {
            (original, evidence, canonical)
        }
    }
    fn model(rows: &[Row]) -> Model<'_> {
        Model {
            rows,
            observed: UtcTimestamp::parse_rfc3339("2026-10-15T00:00:00Z").unwrap(),
        }
    }
    fn error(rows: &[Row]) -> DailyVerificationError {
        match resolve_original_with(&model(rows), &reference(&rows[0])) {
            Err(error) => error,
            Ok(_) => panic!("policy must refuse"),
        }
    }
    #[test]
    fn shared_lineage_keeps_terminal_identity_and_canonical_evidence() {
        let mut rows = [row(1), row(2)];
        for row in &mut rows {
            row.request.headers = vec![
                ("appkey".into(), "[REDACTED]".into()),
                ("appsecret".into(), "[REDACTED]".into()),
                ("authorization".into(), "[REDACTED]".into()),
                ("tr_cont".into(), "".into()),
                ("tr_id".into(), "FHKST03010100".into()),
            ];
        }
        let (direct, evidence, _) =
            resolve_original_with(&model(&rows), &reference(&rows[0])).unwrap();
        assert!(std::ptr::eq(direct, &rows[0]));
        assert!(evidence.copied_from.is_empty());
        rows[0].next = Some(reference(&rows[1]));
        let (terminal, evidence, bytes) =
            resolve_original_with(&model(&rows), &reference(&rows[0])).unwrap();
        assert!(std::ptr::eq(terminal, &rows[1]));
        assert_eq!(evidence.batch_id, rows[0].id);
        assert_eq!(evidence.copied_from[0].batch_id, rows[1].id);
        assert_eq!(
            evidence.original_retrieved_at.as_timestamp(),
            rows[1].retrieved
        );
        assert!(!bytes.is_empty());
        // The same policy follows a genuine borrowed ordinary row in the other channel.
        rows[1].provider = "kis-calendar".into();
        for row in &mut rows {
            row.kind = ResponseKind::Calendar;
            row.request.endpoint = "/uapi/domestic-stock/v1/quotations/chk-holiday".into();
            row.request.headers.last_mut().unwrap().1 = "CTCA0903R".into();
        }
        rows[0].next = Some(reference(&rows[1]));
        let (terminal, evidence, _) =
            resolve_original_with(&model(&rows), &reference(&rows[0])).unwrap();
        assert!(std::ptr::eq(terminal, &rows[1]));
        assert_eq!(evidence.copied_from[0].provider, "kis-calendar");
    }
    #[test]
    fn shared_lineage_preserves_mode_entitlement_and_time_error_order() {
        let mut rows = [row(1)];
        rows[0].mode = FetchMode::Synthetic;
        rows[0].entitlement.clear();
        assert_eq!(error(&rows), DailyVerificationError::NonCredentialed);
        rows[0].mode = FetchMode::Credentialed;
        assert_eq!(error(&rows), DailyVerificationError::EntitlementMismatch);
        rows[0].entitlement = "ordinary-license".into();
        rows[0].retrieved = UtcTimestamp::parse_rfc3339("2026-10-16T00:00:00Z").unwrap();
        assert_eq!(error(&rows), DailyVerificationError::FutureRetrieval);
    }
    #[test]
    fn shared_lineage_checks_bytes_request_and_continuation_each_hop() {
        let mut rows = [row(1), row(2)];
        rows[0].next = Some(reference(&rows[1]));
        rows[1].body = b"different".to_vec().into_boxed_slice();
        assert_eq!(error(&rows), DailyVerificationError::CopyMismatch);
        rows[1].body = b"body".to_vec().into_boxed_slice();
        rows[1]
            .request
            .query
            .push(("date".into(), "changed".into()));
        assert_eq!(error(&rows), DailyVerificationError::CopyMismatch);
        rows[1].request.query.clear();
        rows[1].marker = Some("M".into());
        assert_eq!(error(&rows), DailyVerificationError::CopyMismatch);
        rows[1].marker = None;
        rows[1].hash = ContentHash::from_bytes(b"other hash");
        assert_eq!(error(&rows), DailyVerificationError::CopyMismatch);
    }
    #[test]
    fn shared_lineage_cycle_precedes_repeated_row_mode_and_depth_is_bounded() {
        let mut rows = [row(1), row(2)];
        rows[0].next = Some(reference(&rows[1]));
        rows[1].next = Some(reference(&rows[0]));
        assert_eq!(error(&rows), DailyVerificationError::CopyCycle);
        let mut rows: Vec<_> = (1..=18).map(row).collect();
        for index in 0..17 {
            rows[index].next = Some(reference(&rows[index + 1]));
        }
        assert_eq!(error(&rows), DailyVerificationError::CopyDepth);
    }
    #[test]
    fn shared_context_and_scope_selection_keep_original_rejections() {
        let fields = |scope: &&'static str| ScopeFields {
            uid: 9,
            provider: scope,
        };
        assert!(validate_context(&[] as &[&str], "license", fields).is_err());
        assert!(validate_context(&["kis", "kis"], "license", fields).is_err());
        assert!(validate_context(&["kis", "kis-calendar"], "license", fields).is_ok());
        let rows = [row(1)];
        let reference = reference(&rows[0]);
        let chosen = find_snapshot(rows.iter(), &reference.provider, &reference.market, |row| {
            (row.provider.as_str(), row.market.as_str())
        })
        .unwrap();
        assert!(std::ptr::eq(chosen, &rows[0]));
        assert!(
            find_snapshot(rows.iter(), "missing", "kr", |row| (
                row.provider.as_str(),
                row.market.as_str()
            ))
            .is_err()
        );
    }
}
