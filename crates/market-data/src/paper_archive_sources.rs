//! Read-only composition of the existing Daily source readers for archive use.
//!
//! These holders are descriptive evidence. They confer no database, execution,
//! stopped-writer, snapshot, release or restore authority. In particular, a
//! serialized digest cannot construct one. The caller retains the real source
//! owners and must consume the holder while that custody is still live.

use domain::{ContentHash, TradingDate, UtcTimestamp};

use crate::SourceFileReference;
use crate::checked_close::Attempt;
use crate::daily_price_only::admission::{
    SealedHistoricalDailyInput, verify_historical_daily_candidate,
};
use crate::daily_price_only::continuation::{
    SealedDailyContinuation, SealedDailyParent, verify_daily_continuation_candidate,
};
use crate::daily_price_only::frontier::{
    SealedSourceFrontier, prepare_continued_source_frontier, prepare_first_source_frontier,
    refresh_source_frontier,
};
use crate::daily_price_only::reconstruct::{VerifiedPriceDocuments, reconstruct_prices};
use crate::daily_price_only::source::CommittedRawSnapshot;
use crate::daily_price_only::verify::{RawEvidenceSnapshots, resolve_original};
use crate::daily_price_only::{DailyFileEvidenceV1Wire, DailyNormalizedDocumentV1Wire};
use crate::{ApprovedHistoricalPriceOnlyArtifact, CheckedHistoricalApproval};

/// Native original inventory and database-reference consumption. This module
/// deliberately exposes no completed archive or serving capability.
#[cfg(target_os = "linux")]
pub mod closure;

/// Finite diagnostics only. No paths, SQL, provider bytes or rights prose escape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveSourceError {
    #[error("ARCHIVE_SOURCE_CUSTODY")]
    Custody,
    #[error("ARCHIVE_SOURCE_CONTEXT")]
    Context,
    #[error("ARCHIVE_SOURCE_RECONSTRUCTION")]
    Reconstruction,
    #[error("ARCHIVE_SOURCE_PIN")]
    Pin,
    #[error("ARCHIVE_SOURCE_FRONTIER")]
    Frontier,
    #[error("ARCHIVE_SOURCE_HISTORY")]
    History,
    #[error("ARCHIVE_SOURCE_INTERRUPTED")]
    Interrupted,
    #[error("ARCHIVE_SOURCE_EXCLUSION_UNPROVEN")]
    ExclusionUnproven,
}

/// Borrows complete source-owned Raw snapshots, including their live lock FDs.
/// Context values select existing evidence; they do not authorize data rights.
pub struct RetainedRawEvidence<'a> {
    originals: Vec<&'a CommittedRawSnapshot>,
    evidence: RawEvidenceSnapshots<'a>,
}

pub fn retain_raw_evidence<'a>(
    originals: &[&'a CommittedRawSnapshot],
    entitlement_reference: &'a str,
    observed_at: UtcTimestamp,
) -> Result<RetainedRawEvidence<'a>, ArchiveSourceError> {
    // The existing reader rejects empty, duplicate and unsupported scope sets
    // before allocating this bounded (at most two scopes) borrow.
    let evidence = RawEvidenceSnapshots::new(originals, entitlement_reference, observed_at)
        .map_err(|_| ArchiveSourceError::Context)?;
    let value = RetainedRawEvidence {
        originals: originals.to_vec(),
        evidence,
    };
    value.revalidate()?;
    Ok(value)
}

impl RetainedRawEvidence<'_> {
    pub fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        // Visit every original even after an earlier failure. No cached clean bit.
        let mut changed = false;
        for original in &self.originals {
            changed |= original.revalidate().is_err();
        }
        if changed {
            Err(ArchiveSourceError::Custody)
        } else {
            Ok(())
        }
    }
}

/// A lexical projection over the actual complete captures lent by
/// with_checked_raw_originals. The original native owners remain held. Owned
/// StoredFile buffers are the existing normalizer's bounded input shape, not
/// replacement snapshots or independently authoritative file evidence.
#[cfg(target_os = "linux")]
struct CapturedPublicationReader<'s> {
    originals: &'s [&'s CommittedRawSnapshot],
    keep_reading: &'s dyn Fn() -> bool,
}
#[cfg(target_os = "linux")]
impl CapturedPublicationReader<'_> {
    fn revalidate(&self) -> Result<(), crate::storage::StoreError> {
        let mut clean = (self.keep_reading)();
        for original in self.originals {
            clean &= original.revalidate().is_ok();
        }
        if clean { Ok(()) } else { Err(Self::error()) }
    }
    fn error() -> crate::storage::StoreError {
        crate::storage::StoreError::Io {
            context: "ARCHIVE_RAW_ORIGINAL".into(),
            source: std::io::Error::other("ARCHIVE_RAW_ORIGINAL"),
        }
    }
    fn scope(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<&CommittedRawSnapshot, crate::storage::StoreError> {
        if market != "kr"
            || ![
                "kis",
                "kis-calendar",
                "kis-daily-range",
                "kis-daily-range-normalized",
                "krx",
                "kis-normalized",
                "kis-calendar-normalized",
            ]
            .contains(&provider)
        {
            return Err(Self::error());
        }
        let mut matches = self
            .originals
            .iter()
            .copied()
            .filter(|s| s.scope().provider() == provider && s.scope().market() == market);
        let original = matches.next().ok_or_else(Self::error)?;
        if matches.next().is_some() {
            return Err(Self::error());
        }
        Ok(original)
    }
}
#[cfg(target_os = "linux")]
impl crate::storage::RawOriginalRead for CapturedPublicationReader<'_> {
    fn read_committed_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<crate::storage::ManifestEntry>, crate::storage::StoreError> {
        self.revalidate()?;
        let body = self.scope(provider, market).map(|original| {
            original
                .batches()
                .iter()
                .map(|batch| batch.entry().clone())
                .collect()
        });
        let after = self.revalidate();
        after.and(body)
    }
    fn read_batch_bytes(
        &self,
        provider: &str,
        market: &str,
        entry: &crate::storage::ManifestEntry,
    ) -> Result<Vec<crate::contract::StoredFile>, crate::storage::StoreError> {
        self.revalidate()?;
        let body = (|| {
            let original = self.scope(provider, market)?;
            let batch = original
                .batches()
                .iter()
                .find(|b| b.batch_id() == entry.batch_id)
                .ok_or_else(Self::error)?;
            if batch.entry() != entry {
                return Err(Self::error());
            }
            let prefix = original
                .original_data_root()
                .join("raw")
                .join(format!("provider={provider}"))
                .join(format!("market={market}"))
                .join(format!("date={}", entry.date))
                .join(format!("batch={}", entry.batch_id));
            let mut bytes = 0usize;
            let mut files = Vec::new();
            files
                .try_reserve_exact(entry.files.len())
                .map_err(|_| Self::error())?;
            for file in batch.files() {
                bytes = bytes
                    .checked_add(file.bytes().len())
                    .ok_or_else(Self::error)?;
                if bytes > original.limits().values().max_retained_bytes {
                    return Err(Self::error());
                }
                // This path is descriptive provenance from the captured scope;
                // it is never opened. Bytes come only from its retained file.
                files.push(crate::contract::StoredFile {
                    file_name: file.file_name().into(),
                    storage_path: prefix.join(file.file_name()),
                    bytes: file.bytes().to_vec(),
                });
            }
            Ok(files)
        })();
        let after = self.revalidate();
        after.and(body)
    }
}

/// Independent reconstruction of the recorded price documents. Later conflicting
/// deliveries do not replace a recorded original: each exact copy chain and D4
/// evidence value is resolved from the retained full Raw inventory. This checks
/// original document content, not completeness of historical source discovery.
pub struct CheckedPriceDocuments<'a> {
    raw: &'a RetainedRawEvidence<'a>,
    documents: VerifiedPriceDocuments<'a>,
}

pub fn check_recorded_price_documents<'a>(
    raw: &'a RetainedRawEvidence<'a>,
    session: TradingDate,
    bars: &[DailyFileEvidenceV1Wire],
    reference: &[DailyFileEvidenceV1Wire],
    expected_bars: &DailyNormalizedDocumentV1Wire,
    expected_reference: &DailyNormalizedDocumentV1Wire,
) -> Result<CheckedPriceDocuments<'a>, ArchiveSourceError> {
    raw.revalidate()?;
    // reconstruct_prices requires exactly the fixed eleven instruments; bound
    // allocation before resolving a caller's untrusted declaration list.
    if bars.len() != crate::KR_ETF_CORE_SYMBOLS.len()
        || reference.len() != crate::KR_ETF_CORE_SYMBOLS.len()
    {
        return Err(ArchiveSourceError::Reconstruction);
    }
    let resolve = |claims: &[DailyFileEvidenceV1Wire]| {
        claims
            .iter()
            .map(|claim| {
                let source = SourceFileReference {
                    provider: claim.provider.clone(),
                    market: claim.market.clone(),
                    batch_id: claim.batch_id,
                    file_name: claim.file_name.clone(),
                    content_hash: claim.wire_sha256.clone(),
                    retrieved_at: claim.retrieved_at.as_timestamp(),
                };
                let resolved = resolve_original(&raw.evidence, &source)
                    .map_err(|_| ArchiveSourceError::Reconstruction)?;
                if resolved.evidence() != claim {
                    return Err(ArchiveSourceError::Pin);
                }
                Ok(resolved)
            })
            .collect::<Result<Vec<_>, ArchiveSourceError>>()
    };
    let bars = resolve(bars)?;
    let reference = resolve(reference)?;
    let documents = reconstruct_prices(session, &bars, &reference)
        .map_err(|_| ArchiveSourceError::Reconstruction)?;
    if documents.bars().descriptor() != expected_bars
        || documents.reference().descriptor() != expected_reference
    {
        return Err(ArchiveSourceError::Pin);
    }
    raw.revalidate()?;
    Ok(CheckedPriceDocuments { raw, documents })
}

impl CheckedPriceDocuments<'_> {
    /// Exact serde_json/reconstruct bytes; no Python float formatter or re-encode.
    pub fn consume<T>(
        self,
        use_documents: impl FnOnce(&[u8], &[u8]) -> Result<T, ArchiveSourceError>,
    ) -> Result<T, ArchiveSourceError> {
        self.raw.revalidate()?;
        let result = use_documents(
            self.documents.bars().bytes(),
            self.documents.reference().bytes(),
        );
        let custody = self.raw.revalidate();
        match (result, custody) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

/// One source-replayed admission/frontier pair. Constructors below independently
/// run the existing full first/continued reconstruction and compare exact sealed
/// admission bytes and observation preimages. This is not an all-history proof.
pub struct CheckedDailySources<'a, 'base> {
    raw: &'a RetainedRawEvidence<'a>,
    base: &'a RetainedApprovedV3<'a>,
    input: SealedDailyParent<'a, 'base>,
    frontier: &'a SealedSourceFrontier,
    predecessor: Option<&'a CheckedDailySources<'a, 'base>>,
}

pub fn check_first_daily_sources<'a, 'base>(
    raw: &'a RetainedRawEvidence<'a>,
    base: &'a RetainedApprovedV3<'a>,
    input: &'a SealedHistoricalDailyInput<'base>,
    frontier: &'a SealedSourceFrontier,
) -> Result<CheckedDailySources<'a, 'base>, ArchiveSourceError> {
    verify_first_daily_sources(raw, base, input, frontier)?;
    let value = CheckedDailySources {
        raw,
        base,
        input: SealedDailyParent::First(input),
        frontier,
        predecessor: None,
    };
    value.revalidate()?;
    Ok(value)
}

fn verify_first_daily_sources(
    raw: &RetainedRawEvidence<'_>,
    base: &RetainedApprovedV3<'_>,
    input: &SealedHistoricalDailyInput<'_>,
    frontier: &SealedSourceFrontier,
) -> Result<(), ArchiveSourceError> {
    raw.revalidate()?;
    if !std::ptr::eq(base.approved()?, input.base()) {
        return Err(ArchiveSourceError::Pin);
    }
    input
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    frontier
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    let parent = SealedDailyParent::First(input);
    let recorded =
        crate::daily_price_only::frontier::recorded_source_view(&raw.evidence, &parent, frontier)
            .map_err(archive_recorded_view_error)?;
    recorded.consume(archive_recorded_view_error, |snapshots| {
        let verified = verify_historical_daily_candidate(
            input.opened(),
            input.base(),
            snapshots,
            input.policy(),
            frontier.owner(),
            input.admission().verifier_code_commit.clone(),
        )
        .map_err(|_| ArchiveSourceError::Reconstruction)?;
        if verified.pins() != input.pins() || verified.admission_bytes() != input.admission_bytes()
        {
            return Err(ArchiveSourceError::Pin);
        }
        let prepared =
            prepare_first_source_frontier(&verified).map_err(|_| ArchiveSourceError::Frontier)?;
        if prepared.admission_sha256() != frontier.admission_sha256()
            || prepared.policy_sha256() != frontier.policy_sha256()
            || prepared.sha256() != frontier.sha256()
            || prepared.observation_bytes() != frontier.observation().preimage()
        {
            return Err(ArchiveSourceError::Frontier);
        }
        Ok(())
    })
}

fn archive_recorded_view_error(
    error: crate::daily_price_only::frontier::RecordedSourceViewError,
) -> ArchiveSourceError {
    use crate::daily_price_only::admission::DailyAdmissionVerificationError;
    use crate::daily_price_only::frontier::RecordedSourceViewError;
    match error {
        RecordedSourceViewError::Input(error) => match error {
            DailyAdmissionVerificationError::Artifact(_) => ArchiveSourceError::Custody,
            DailyAdmissionVerificationError::Evidence(_) => ArchiveSourceError::Reconstruction,
            DailyAdmissionVerificationError::InvalidPolicy
            | DailyAdmissionVerificationError::InvalidCandidate => ArchiveSourceError::Pin,
            DailyAdmissionVerificationError::LimitExceeded => ArchiveSourceError::Context,
        },
        RecordedSourceViewError::ExclusionUnproven => ArchiveSourceError::ExclusionUnproven,
    }
}

pub fn check_continued_daily_sources<'a, 'base>(
    previous: &'a CheckedDailySources<'a, 'base>,
    input: &'a SealedDailyContinuation<'base>,
    frontier: &'a SealedSourceFrontier,
) -> Result<CheckedDailySources<'a, 'base>, ArchiveSourceError> {
    previous.revalidate()?;
    verify_continued_daily_sources(
        previous.raw,
        previous.base,
        &previous.input,
        previous.frontier,
        input,
        frontier,
    )?;
    let next = SealedDailyParent::Continued(input);
    let value = CheckedDailySources {
        raw: previous.raw,
        base: previous.base,
        input: next,
        frontier,
        predecessor: Some(previous),
    };
    value.revalidate()?;
    Ok(value)
}

fn verify_continued_daily_sources(
    raw: &RetainedRawEvidence<'_>,
    base: &RetainedApprovedV3<'_>,
    previous_input: &SealedDailyParent<'_, '_>,
    previous_frontier: &SealedSourceFrontier,
    input: &SealedDailyContinuation<'_>,
    frontier: &SealedSourceFrontier,
) -> Result<(), ArchiveSourceError> {
    raw.revalidate()?;
    if !std::ptr::eq(base.approved()?, input.base()) {
        return Err(ArchiveSourceError::Pin);
    }
    previous_input
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    previous_frontier
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    input
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    frontier
        .revalidate()
        .map_err(|_| ArchiveSourceError::Custody)?;
    let next = SealedDailyParent::Continued(input);
    let recorded =
        crate::daily_price_only::frontier::recorded_source_view(&raw.evidence, &next, frontier)
            .map_err(archive_recorded_view_error)?;
    recorded.consume(archive_recorded_view_error, |snapshots| {
        let refreshed = refresh_source_frontier(previous_input, previous_frontier, snapshots)
            .map_err(|_| ArchiveSourceError::Frontier)?;
        let verified = verify_daily_continuation_candidate(
            next.opened(),
            &refreshed,
            input.policy(),
            input.admission().verifier_code_commit.clone(),
        )
        .map_err(|_| ArchiveSourceError::Reconstruction)?;
        if verified.pins() != input.pins() || verified.admission_bytes() != input.admission_bytes()
        {
            return Err(ArchiveSourceError::Pin);
        }
        let prepared = prepare_continued_source_frontier(&verified)
            .map_err(|_| ArchiveSourceError::Frontier)?;
        if prepared.owner() != frontier.owner()
            || prepared.admission_sha256() != frontier.admission_sha256()
            || prepared.policy_sha256() != frontier.policy_sha256()
            || prepared.sha256() != frontier.sha256()
            || prepared.observation_bytes() != frontier.observation().preimage()
        {
            return Err(ArchiveSourceError::Frontier);
        }
        raw.revalidate()?;
        Ok(())
    })
}

impl CheckedDailySources<'_, '_> {
    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        self.raw.revalidate()?;
        self.base.revalidate()?;
        // Iterative traversal avoids an attacker-controlled recursion stack.
        let mut current = Some(self);
        let mut visited = 0usize;
        while let Some(value) = current {
            visited = visited.checked_add(1).ok_or(ArchiveSourceError::Context)?;
            if visited > self.input.manifest().daily_sources.len() {
                return Err(ArchiveSourceError::Context);
            }
            value
                .input
                .revalidate()
                .map_err(|_| ArchiveSourceError::Custody)?;
            value
                .frontier
                .revalidate()
                .map_err(|_| ArchiveSourceError::Custody)?;
            current = value.predecessor;
        }
        if visited != self.input.manifest().daily_sources.len() {
            return Err(ArchiveSourceError::Context);
        }
        Ok(())
    }

    pub fn admission_sha256(&self) -> &ContentHash {
        &self.input.pins().admission_sha256
    }

    pub fn consume<T>(
        self,
        use_sources: impl FnOnce(&Self) -> Result<T, ArchiveSourceError>,
    ) -> Result<T, ArchiveSourceError> {
        self.revalidate()?;
        let result = use_sources(&self);
        let custody = self.revalidate();
        match (result, custody) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

/// V3 uses the existing retained checked approval, never the value-only reader.
/// V2 has no equivalent native capability and cannot enter this constructor.
pub struct RetainedApprovedV3<'a> {
    original: ApprovedV3Original<'a>,
    expected: &'a Attempt,
    captured: &'a Attempt,
}

enum ApprovedV3Original<'a> {
    Live(&'a CheckedHistoricalApproval<'a>),
    Archive {
        approved: &'a ApprovedHistoricalPriceOnlyArtifact,
        artifact: &'a RetainedFiles,
        registry: &'a RetainedFiles,
    },
}

pub fn retain_approved_v3<'a>(
    original: &'a CheckedHistoricalApproval<'a>,
    expected: &'a Attempt,
    captured: &'a Attempt,
) -> Result<RetainedApprovedV3<'a>, ArchiveSourceError> {
    let value = RetainedApprovedV3 {
        original: ApprovedV3Original::Live(original),
        expected,
        captured,
    };
    value.revalidate()?;
    if value.approved()?.v3_approval_summary().is_none() {
        return Err(ArchiveSourceError::Pin);
    }
    Ok(value)
}

impl RetainedApprovedV3<'_> {
    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        match &self.original {
            ApprovedV3Original::Live(original) => original
                .revalidate_in(self.expected, self.captured)
                .map_err(|_| ArchiveSourceError::Custody),
            ApprovedV3Original::Archive {
                artifact, registry, ..
            } => {
                let first = artifact.revalidate();
                let second = registry.revalidate();
                let same = self.expected.same_scope(self.captured);
                let expected = self.expected.check();
                let captured = self.captured.check();
                if !same || !expected || !captured {
                    return Err(ArchiveSourceError::Custody);
                }
                first.and(second)
            }
        }
    }
    pub fn approved(&self) -> Result<&ApprovedHistoricalPriceOnlyArtifact, ArchiveSourceError> {
        self.revalidate()?;
        match &self.original {
            ApprovedV3Original::Live(original) => original
                .approved_in(self.expected, self.captured)
                .map_err(|_| ArchiveSourceError::Custody),
            ApprovedV3Original::Archive { approved, .. } => Ok(approved),
        }
    }
}

/// Archive-only consuming bridge to the SAME first/continued/all-history
/// algorithms. The value-only approved artifact cannot enter this constructor:
/// its real registry/artifact descriptors and full historical Raw dependencies
/// are retained and checked here before the scoped typed history is created.
#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_arguments,
    reason = "Actual original roots and scope endpoints stay explicit."
)]
fn consume_checked_daily_history_originals(
    invocation: &crate::raw_custody::RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
    sources: &[crate::daily_price_only::source::CheckedRawSnapshot<'_, '_>],
    artifact_root: &std::path::Path,
    registry_root: &std::path::Path,
    admissions: &std::path::Path,
    frontiers: &std::path::Path,
    policy: &crate::daily_price_only::admission::PinnedDailyPolicy,
    entitlement: &str,
    observed_at: impl Fn(&ContentHash) -> Result<UtcTimestamp, ArchiveSourceError>,
    limits: crate::daily_price_only::replay::DailyHistoryLimits,
    keep: &dyn Fn() -> bool,
    mut consume: impl FnMut(
        &SealedDailyParent<'_, '_>,
        &SealedSourceFrontier,
    ) -> Result<(), ArchiveSourceError>,
    mut consume_material: impl FnMut(
        &crate::daily_price_only::input::AdmittedDailyPriceInputV1<'_, '_>,
    ) -> Result<(), ArchiveSourceError>,
) -> Result<(), ArchiveSourceError> {
    let mut guard = HistoricalUseGuard {
        attempt: expected,
        armed: true,
    };
    let mut files = None;
    let mut registry = None;
    let body = (|| {
        let first = invocation.settle_local_in(expected, Ok::<(), std::convert::Infallible>(()));
        let second = invocation.settle_local_in(captured, Ok::<(), std::convert::Infallible>(()));
        let native = crate::close_gate::require_native(invocation, expected);
        if first.is_err() || second.is_err() || native.is_err() {
            return Err(ArchiveSourceError::Custody);
        }
        if !keep() {
            return Err(ArchiveSourceError::Interrupted);
        }
        let pins = crate::historical_price_only_approval::embedded_v3_archive_pins()
            .map_err(|_| ArchiveSourceError::Pin)?;
        let leaf = artifact_root.join(
            crate::historical_price_only_v3_artifact::candidate_directory_name(
                pins.candidate_content_sha256(),
            )
            .map_err(|_| ArchiveSourceError::Pin)?,
        );
        registry = Some(RetainedFiles::open_with_attempt(
            registry_root,
            &["kis-historical-price-only-v3-approved-artifacts.json"],
            0,
            0o644,
            262_144,
            Some(expected),
        )?);
        files = Some(RetainedFiles::open_with_attempt(
            &leaf,
            &["manifest.json", "bars.ndjson"],
            10001,
            0o600,
            134_217_728,
            Some(expected),
        )?);
        let artifact = HistoricalArtifactView {
            files: files.as_ref().ok_or(ArchiveSourceError::Custody)?,
            path: &leaf,
        };
        let registry = registry.as_ref().ok_or(ArchiveSourceError::Custody)?;
        let approved =
            crate::historical_price_only_approval::approve_retained_historical_originals(
                HistoricalArchiveVersion::V3,
                &artifact,
                &registry.read_original(0, 262_144)?,
            )
            .map_err(|_| ArchiveSourceError::Pin)?;
        if approved.pins() != &pins {
            return Err(ArchiveSourceError::Pin);
        }
        let historical = HistoricalRawReader {
            sources,
            expected,
            captured,
            keep_reading: keep,
        };
        verify_historical_dependencies(
            HistoricalArchiveVersion::V3,
            &historical,
            &artifact,
            &approved,
        )?;
        let base = RetainedApprovedV3 {
            original: ApprovedV3Original::Archive {
                approved: &approved,
                artifact: artifact.files,
                registry,
            },
            expected,
            captured,
        };
        let mut original_error = None;
        let borrowed = crate::daily_price_only::source::with_checked_raw_originals(
            sources,
            expected,
            captured,
            |all| {
                let selected: Vec<_> = all
                    .iter()
                    .copied()
                    .filter(|s| ["kis", "kis-calendar"].contains(&s.scope().provider()))
                    .collect();
                let used = crate::daily_price_only::artifact::with_archive_daily_roots(
                    invocation,
                    expected,
                    captured,
                    admissions,
                    frontiers,
                    |admissions, frontiers| {
                        let result = (|| {
                            let history =
                                crate::daily_price_only::replay::read_captured_daily_history(
                                    admissions,
                                    frontiers,
                                    base.approved()?,
                                    policy,
                                    limits,
                                    keep,
                                )
                                .map_err(archive_history_error)?;
                            let mut domain_error = None;
                            let walked = history.visit(&mut || keep(), |node, previous| {
                            let checked = (|| {
                                let parent = node.as_parent();
                                // This actual SQL recorded time is an observation
                                // constraint, never a capture boundary or filter.
                                // FULL and selected replay still both must pass.
                                let raw = retain_raw_evidence(&selected, entitlement,
                                    observed_at(&parent.pins().admission_sha256)?)?;
                                match (&parent, previous) {
                                    (SealedDailyParent::First(input), None) =>
                                        verify_first_daily_sources(&raw, &base, input, node.frontier())?,
                                    (SealedDailyParent::Continued(input), Some(previous)) =>
                                        verify_continued_daily_sources(&raw, &base, &previous.as_parent(),
                                            previous.frontier(), input, node.frontier())?,
                                    _ => return Err(ArchiveSourceError::History),
                                }
                                let body = consume(&parent, node.frontier());
                                let after = raw.revalidate().and(base.revalidate());
                                after.and(body)?;
                                let admitted = crate::daily_price_only::input::AdmittedDailyPriceInputV1::from_sealed(node.as_parent())
                                    .map_err(|_| ArchiveSourceError::History)?;
                                historical_callback(expected, captured, &mut || keep(),
                                    || raw.revalidate().and(base.revalidate()),
                                    || consume_material(&admitted))
                            })();
                            if let Err(error) = checked {
                                domain_error = Some(error);
                                return Err(crate::daily_price_only::replay::DailyReplayError::InvalidChain);
                            }
                            Ok(())
                        });
                            let after = history.revalidate().map_err(archive_history_error);
                            if let Some(error) = domain_error {
                                return Err(error);
                            }
                            after.and(walked.map_err(archive_history_error))
                        })();
                        if let Err(error) = result {
                            original_error = Some(error);
                            return Err(crate::daily_price_only::artifact::DailyArtifactError::InvalidArtifact);
                        }
                        Ok(())
                    },
                );
                if let Some(error) = original_error {
                    return Err(error);
                }
                used.map_err(|_| ArchiveSourceError::Custody)
            },
        );
        match borrowed {
            Ok(()) => Ok(()),
            Err(failure) => Err(failure
                .callback
                .and_then(Result::err)
                .unwrap_or(ArchiveSourceError::Custody)),
        }
    })();
    let mut clean = true;
    if let Some(files) = &files {
        clean &= files.revalidate().is_ok();
    }
    if let Some(registry) = &registry {
        clean &= registry.revalidate().is_ok();
    }
    for source in sources {
        clean &= source.revalidate_originals_in(expected, captured).is_ok();
    }
    if let Some(files) = files {
        clean &= files.retire().is_ok();
    }
    if let Some(registry) = registry {
        clean &= registry.retire().is_ok();
    }
    let native_after = crate::close_gate::require_native(invocation, expected);
    let expected_after =
        invocation.settle_local_in(expected, Ok::<(), std::convert::Infallible>(()));
    let captured_after =
        invocation.settle_local_in(captured, Ok::<(), std::convert::Infallible>(()));
    clean &= native_after.is_ok();
    clean &= expected_after.is_ok();
    clean &= captured_after.is_ok();
    clean &= expected.check();
    clean &= captured.check();
    if !clean {
        return Err(body.err().unwrap_or(ArchiveSourceError::Custody));
    }
    body?;
    guard.armed = false;
    Ok(())
}

/// The existing V2 pinned reader surrounded by actual descriptor/name custody.
/// This is deliberately not named CheckedHistoricalApproval: that native checked
/// capability exists for V3 only. Consuming use rechecks all retained originals.
pub struct RetainedV2Historical {
    artifact: ApprovedHistoricalPriceOnlyArtifact,
    files: RetainedFiles,
    registry: RetainedFiles,
}

pub fn retain_historical_v2(
    artifact_root: &std::path::Path,
    registry_root: &std::path::Path,
) -> Result<RetainedV2Historical, ArchiveSourceError> {
    let pins = crate::historical_price_only_approval::embedded_v2_archive_pins()
        .map_err(|_| ArchiveSourceError::Pin)?;
    let registry = RetainedFiles::open(
        registry_root,
        &["kis-historical-price-only-beta-approved-artifacts.json"],
        0,
        0o644,
        262_144,
    )?;
    if registry.leaves[0].sha256 != *pins.approval_registry_sha256() {
        return Err(ArchiveSourceError::Pin);
    }
    let candidate = format!(
        "candidate-sha256={}",
        pins.candidate_content_sha256()
            .as_str()
            .strip_prefix("sha256:")
            .ok_or(ArchiveSourceError::Pin)?
    );
    let leaf = artifact_root
        .join("kis-historical-price-only-beta")
        .join("v2")
        .join(candidate);
    let owner = rustix::process::geteuid().as_raw();
    let files = RetainedFiles::open(
        &leaf,
        &["manifest.json", "bars.ndjson"],
        owner,
        0o600,
        134_217_728,
    )?;
    let artifact = crate::approve_historical_price_only_artifact_for_pin_values(
        artifact_root,
        pins.candidate_content_sha256(),
        pins.artifact_manifest_sha256(),
        pins.stage5_manifest_sha256(),
        pins.action_manifest_sha256(),
        pins.approval_registry_sha256(),
    )
    .map_err(|_| ArchiveSourceError::Reconstruction)?;
    if artifact.v3_approval_summary().is_some() || artifact.pins() != &pins {
        return Err(ArchiveSourceError::Pin);
    }
    let value = RetainedV2Historical {
        artifact,
        files,
        registry,
    };
    value.revalidate()?;
    Ok(value)
}

impl RetainedV2Historical {
    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        let files = self.files.revalidate();
        let registry = self.registry.revalidate();
        files.and(registry)
    }

    pub fn consume<T>(
        self,
        use_artifact: impl FnOnce(&ApprovedHistoricalPriceOnlyArtifact) -> Result<T, ArchiveSourceError>,
    ) -> Result<T, ArchiveSourceError> {
        self.revalidate()?;
        let result = use_artifact(&self.artifact);
        let custody = self.revalidate();
        match (result, custody) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

// Retained archive files use their actual caller ledger when opened for the
// historical consumer. Ordinary existing readers keep their legacy semantics.
enum ArchiveFile {
    Ordinary(std::fs::File),
    Checked(crate::checked_close::Owner<std::fs::File>),
}
impl ArchiveFile {
    fn adopt(file: std::fs::File, attempt: Option<&Attempt>) -> Self {
        match attempt {
            Some(attempt) => Self::Checked(attempt.adopt(file, crate::checked_close::Role::Source)),
            None => Self::Ordinary(file),
        }
    }
}
impl std::ops::Deref for ArchiveFile {
    type Target = std::fs::File;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Ordinary(file) => file,
            Self::Checked(owner) => owner.get(),
        }
    }
}
impl std::os::fd::AsFd for ArchiveFile {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&**self)
    }
}

// Only fixed class-bound leaves above may use this private descriptor reader.
// Hashing is bounded and streamed; cumulative historical bars are not duplicated.
struct RetainedFiles {
    attempt: Option<Attempt>,
    directories: Vec<(ArchiveFile, std::ffi::OsString, FileIdentity)>,
    leaves: Vec<RetainedLeaf>,
}
struct RetainedLeaf {
    file: ArchiveFile,
    name: std::ffi::OsString,
    identity: FileIdentity,
    sha256: ContentHash,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    regular: Option<(u64, i64, i64, i64, i64, i64)>,
}
impl FileIdentity {
    fn from_stat(stat: &rustix::fs::Stat, regular: bool) -> Self {
        Self {
            dev: stat.st_dev,
            ino: stat.st_ino,
            mode: stat.st_mode,
            uid: stat.st_uid,
            gid: stat.st_gid,
            regular: regular.then_some((
                stat.st_nlink,
                stat.st_size,
                stat.st_mtime,
                stat.st_mtime_nsec as i64,
                stat.st_ctime,
                stat.st_ctime_nsec as i64,
            )),
        }
    }
}
fn archive_no_acl(file: &std::fs::File) -> Result<(), ArchiveSourceError> {
    for name in ["system.posix_acl_access", "system.posix_acl_default"] {
        let mut byte = [0u8; 1];
        if rustix::fs::fgetxattr(file, name, byte.as_mut_slice()) != Err(rustix::io::Errno::NODATA)
        {
            return Err(ArchiveSourceError::Custody);
        }
    }
    Ok(())
}
fn archive_file_hash(file: &std::fs::File, size: u64) -> Result<ContentHash, ArchiveSourceError> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::FileExt;
    let mut hash = Sha256::new();
    let mut position = 0u64;
    let mut buffer = [0u8; 8192];
    while position < size {
        let wanted = usize::try_from((size - position).min(buffer.len() as u64))
            .map_err(|_| ArchiveSourceError::Custody)?;
        let count = file
            .read_at(&mut buffer[..wanted], position)
            .map_err(|_| ArchiveSourceError::Custody)?;
        if count == 0 {
            return Err(ArchiveSourceError::Custody);
        }
        hash.update(&buffer[..count]);
        position = position
            .checked_add(count as u64)
            .ok_or(ArchiveSourceError::Custody)?;
    }
    if file
        .read_at(&mut buffer[..1], size)
        .map_err(|_| ArchiveSourceError::Custody)?
        != 0
    {
        return Err(ArchiveSourceError::Custody);
    }
    ContentHash::parse(&format!("sha256:{:x}", hash.finalize()))
        .map_err(|_| ArchiveSourceError::Custody)
}
impl RetainedFiles {
    fn open(
        root: &std::path::Path,
        names: &[&str],
        owner: u32,
        mode: u32,
        maximum: u64,
    ) -> Result<Self, ArchiveSourceError> {
        Self::open_with_attempt(root, names, owner, mode, maximum, None)
    }

    fn open_with_attempt(
        root: &std::path::Path,
        names: &[&str],
        owner: u32,
        mode: u32,
        maximum: u64,
        attempt: Option<&Attempt>,
    ) -> Result<Self, ArchiveSourceError> {
        if attempt.is_some_and(|attempt| !attempt.check()) {
            return Err(ArchiveSourceError::Custody);
        }
        use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
        use std::os::unix::ffi::OsStrExt;
        let bytes = root.as_os_str().as_bytes();
        if bytes.len() > 4096
            || !bytes.starts_with(b"/")
            || bytes[1..]
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b"..")
        {
            return Err(ArchiveSourceError::Context);
        }
        let components = root.components().skip(1).collect::<Vec<_>>();
        if components.is_empty() || components.len() > 64 || names.len() > 2 {
            return Err(ArchiveSourceError::Context);
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let first = std::fs::File::from(
            open("/", flags, Mode::empty()).map_err(|_| ArchiveSourceError::Custody)?,
        );
        let first = ArchiveFile::adopt(first, attempt);
        let info = fstat(&first).map_err(|_| ArchiveSourceError::Custody)?;
        let mut directories = vec![(
            first,
            std::ffi::OsString::from("/"),
            FileIdentity::from_stat(&info, false),
        )];
        for component in components {
            let name = component.as_os_str();
            let file = std::fs::File::from(
                openat(&directories.last().unwrap().0, name, flags, Mode::empty())
                    .map_err(|_| ArchiveSourceError::Custody)?,
            );
            let file = ArchiveFile::adopt(file, attempt);
            let info = fstat(&file).map_err(|_| ArchiveSourceError::Custody)?;
            if FileType::from_raw_mode(info.st_mode) != FileType::Directory
                || info.st_mode & 0o7022 != 0
                || (info.st_uid != 0 && info.st_uid != owner)
            {
                return Err(ArchiveSourceError::Custody);
            }
            archive_no_acl(&file)?;
            directories.push((
                file,
                name.to_os_string(),
                FileIdentity::from_stat(&info, false),
            ));
        }
        let (parent, _, identity) = directories.last().unwrap();
        if identity.uid != owner
            || identity.gid != owner
            || (names.len() == 2 && identity.mode & 0o7777 != 0o700)
        {
            return Err(ArchiveSourceError::Custody);
        }
        let mut leaves = Vec::new();
        for name in names {
            let file = std::fs::File::from(
                openat(
                    parent,
                    *name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|_| ArchiveSourceError::Custody)?,
            );
            let file = ArchiveFile::adopt(file, attempt);
            let info = fstat(&file).map_err(|_| ArchiveSourceError::Custody)?;
            if FileType::from_raw_mode(info.st_mode) != FileType::RegularFile
                || info.st_uid != owner
                || info.st_gid != owner
                || info.st_mode & 0o7777 != mode
                || info.st_nlink != 1
                || info.st_size < 1
                || info.st_size as u64 > maximum
            {
                return Err(ArchiveSourceError::Custody);
            }
            archive_no_acl(&file)?;
            let sha256 = archive_file_hash(&file, info.st_size as u64)?;
            leaves.push(RetainedLeaf {
                file,
                name: (*name).into(),
                identity: FileIdentity::from_stat(&info, true),
                sha256,
            });
        }
        let value = Self {
            attempt: attempt.cloned(),
            directories,
            leaves,
        };
        value.revalidate()?;
        Ok(value)
    }

    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        if self
            .attempt
            .as_ref()
            .is_some_and(|attempt| !attempt.check())
        {
            return Err(ArchiveSourceError::Custody);
        }
        use rustix::fs::{AtFlags, fstat, statat};
        for (index, (file, name, identity)) in self.directories.iter().enumerate() {
            let current = fstat(file).map_err(|_| ArchiveSourceError::Custody)?;
            if FileIdentity::from_stat(&current, false) != *identity {
                return Err(ArchiveSourceError::Custody);
            }
            archive_no_acl(file)?;
            if index > 0 {
                let named = statat(
                    &self.directories[index - 1].0,
                    name,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(|_| ArchiveSourceError::Custody)?;
                if FileIdentity::from_stat(&named, false) != *identity {
                    return Err(ArchiveSourceError::Custody);
                }
            }
        }
        self.revalidate_membership()?;
        for leaf in &self.leaves {
            let validate = || {
                let current = fstat(&leaf.file).map_err(|_| ArchiveSourceError::Custody)?;
                let named = statat(
                    &self.directories.last().unwrap().0,
                    &leaf.name,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(|_| ArchiveSourceError::Custody)?;
                if FileIdentity::from_stat(&current, true) != leaf.identity
                    || FileIdentity::from_stat(&named, true) != leaf.identity
                {
                    return Err(ArchiveSourceError::Custody);
                }
                archive_no_acl(&leaf.file)
            };
            validate()?;
            let size = leaf.identity.regular.ok_or(ArchiveSourceError::Custody)?.1;
            if size < 1 || archive_file_hash(&leaf.file, size as u64)? != leaf.sha256 {
                return Err(ArchiveSourceError::Custody);
            }
            validate()?;
        }
        self.revalidate_membership()?;
        if self
            .attempt
            .as_ref()
            .is_some_and(|attempt| !attempt.check())
        {
            return Err(ArchiveSourceError::Custody);
        }
        Ok(())
    }

    fn revalidate_membership(&self) -> Result<(), ArchiveSourceError> {
        use std::os::unix::ffi::OsStrExt;
        // The sealed V2 candidate has exactly two members. The release's
        // registry directory contains unrelated adopted registries as well.
        if self.leaves.len() != 2 {
            return Ok(());
        }
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        let parent = &self
            .directories
            .last()
            .ok_or(ArchiveSourceError::Custody)?
            .0;
        let fd = rustix::fs::openat(
            parent,
            ".",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| ArchiveSourceError::Custody)?;
        let fd = ArchiveFile::adopt(std::fs::File::from(fd), self.attempt.as_ref());
        let mut entries = rustix::fs::RawDir::new(&fd, &mut buffer);
        let mut seen = [false; 2];
        while let Some(entry) = entries.next() {
            let entry = entry.map_err(|_| ArchiveSourceError::Custody)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let index = self
                .leaves
                .iter()
                .position(|leaf| leaf.name.as_bytes() == name)
                .ok_or(ArchiveSourceError::Custody)?;
            if seen[index] {
                return Err(ArchiveSourceError::Custody);
            }
            seen[index] = true;
        }
        if seen != [true; 2] {
            return Err(ArchiveSourceError::Custody);
        }
        Ok(())
    }
}

/// A single exact additional normalized document from the existing readers.
/// It borrows the full retained Raw inventory, including unselected history.
pub struct CheckedAdditionalDocument<'a> {
    raw: &'a RetainedRawEvidence<'a>,
    document: AdditionalDocument<'a>,
}
enum AdditionalDocument<'a> {
    Calendar(crate::daily_price_only::calendar::ReconstructedCalendarDocument),
    Actions(crate::daily_price_only::reconstruct::VerifiedActionDocument<'a>),
}

pub fn check_recorded_calendar_document<'a>(
    raw: &'a RetainedRawEvidence<'a>,
    owner: uuid::Uuid,
    policy: &ContentHash,
    original: &DailyFileEvidenceV1Wire,
    expected: &DailyNormalizedDocumentV1Wire,
) -> Result<CheckedAdditionalDocument<'a>, ArchiveSourceError> {
    raw.revalidate()?;
    let source = SourceFileReference {
        provider: original.provider.clone(),
        market: original.market.clone(),
        batch_id: original.batch_id,
        file_name: original.file_name.clone(),
        content_hash: original.wire_sha256.clone(),
        retrieved_at: original.retrieved_at.as_timestamp(),
    };
    let resolved =
        resolve_original(&raw.evidence, &source).map_err(|_| ArchiveSourceError::Reconstruction)?;
    if resolved.evidence() != original {
        return Err(ArchiveSourceError::Pin);
    }
    let calendar = crate::daily_price_only::calendar::project_calendar(&resolved, owner, policy)
        .map_err(|_| ArchiveSourceError::Reconstruction)?;
    let document = calendar
        .document(expected.session_date)
        .map_err(|_| ArchiveSourceError::Reconstruction)?;
    if document.descriptor() != expected {
        return Err(ArchiveSourceError::Pin);
    }
    raw.revalidate()?;
    Ok(CheckedAdditionalDocument {
        raw,
        document: AdditionalDocument::Calendar(document),
    })
}

pub fn check_recorded_action_document<'a>(
    raw: &'a RetainedRawEvidence<'a>,
    previous_frontier: TradingDate,
    session: TradingDate,
    witnesses: &[crate::daily_price_only::DailyActionChainV1Wire],
    expected: &DailyNormalizedDocumentV1Wire,
) -> Result<CheckedAdditionalDocument<'a>, ArchiveSourceError> {
    raw.revalidate()?;
    let document = crate::daily_price_only::reconstruct::reconstruct_actions(
        &raw.evidence,
        previous_frontier,
        session,
    )
    .map_err(|_| ArchiveSourceError::Reconstruction)?;
    if document.witnesses() != witnesses || document.document().descriptor() != expected {
        return Err(ArchiveSourceError::Pin);
    }
    raw.revalidate()?;
    Ok(CheckedAdditionalDocument {
        raw,
        document: AdditionalDocument::Actions(document),
    })
}

impl CheckedAdditionalDocument<'_> {
    pub fn consume<T>(
        self,
        use_document: impl FnOnce(&[u8]) -> Result<T, ArchiveSourceError>,
    ) -> Result<T, ArchiveSourceError> {
        self.raw.revalidate()?;
        let bytes = match &self.document {
            AdditionalDocument::Calendar(document) => document.bytes(),
            AdditionalDocument::Actions(document) => document.document().bytes(),
        };
        let result = use_document(bytes);
        let custody = self.raw.revalidate();
        match (result, custody) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }
}

#[cfg(test)]
mod tests;
/// Captured committed Daily file history for one pinned policy and base,
/// including standalone admissions. Every committed leaf/frontier is retained,
/// rather than a caller-selected tip or a hash list. Other policy generations
/// fail closed; this is not a multi-policy archive inventory. Recognized pending
/// names remain captured by the existing root reader but are never promoted.
/// This holder is unavailable when any existing independent source replay fails.
///
/// Historical exclusion is not inferred from maximum retrieval time or log
/// order. The source-owned recorded view requires independent complete and
/// selected replay. Relevant later conflicts therefore remain closed until an
/// actual retained historical boundary can justify excluding them.
pub struct CheckedDailyHistory<'a> {
    raw: &'a RetainedRawEvidence<'a>,
    base: &'a RetainedApprovedV3<'a>,
    history: crate::daily_price_only::replay::CapturedDailyHistory<'a>,
}

pub fn check_daily_history<'a>(
    raw: &'a RetainedRawEvidence<'a>,
    base: &'a RetainedApprovedV3<'a>,
    admissions: &crate::daily_price_only::artifact::DailyAdmissionRoot,
    frontiers: &crate::daily_price_only::artifact::DailySourceFrontierRoot,
    policy: &'a crate::daily_price_only::admission::PinnedDailyPolicy,
    limits: crate::daily_price_only::replay::DailyHistoryLimits,
    mut keep_reading: impl FnMut() -> bool,
) -> Result<CheckedDailyHistory<'a>, ArchiveSourceError> {
    raw.revalidate()?;
    base.revalidate()?;
    let history = crate::daily_price_only::replay::read_captured_daily_history(
        admissions,
        frontiers,
        base.approved()?,
        policy,
        limits,
        &mut keep_reading,
    )
    .map_err(archive_history_error)?;
    let result = CheckedDailyHistory { raw, base, history };
    result.check_sources(&mut keep_reading)?;
    result.revalidate()?;
    Ok(result)
}

fn archive_history_error(
    error: crate::daily_price_only::replay::DailyReplayError,
) -> ArchiveSourceError {
    use crate::daily_price_only::replay::DailyReplayError;
    match error {
        DailyReplayError::Interrupted => ArchiveSourceError::Interrupted,
        DailyReplayError::InvalidChain => ArchiveSourceError::History,
        DailyReplayError::InvalidLimits | DailyReplayError::LimitExceeded => {
            ArchiveSourceError::Context
        }
        DailyReplayError::Artifact(_) | DailyReplayError::Verification(_) => {
            ArchiveSourceError::Custody
        }
    }
}

impl CheckedDailyHistory<'_> {
    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        let raw = self.raw.revalidate();
        let base = self.base.revalidate();
        let history = self.history.revalidate().map_err(archive_history_error);
        raw.and(base).and(history)
    }

    fn check_sources(
        &self,
        keep_reading: &mut impl FnMut() -> bool,
    ) -> Result<(), ArchiveSourceError> {
        self.raw.revalidate()?;
        self.base.revalidate()?;
        let mut source_error = None;
        let result = self.history.visit(keep_reading, |node, previous| {
            let validation = match (node.as_parent(), previous) {
                (SealedDailyParent::First(input), None) => {
                    verify_first_daily_sources(self.raw, self.base, input, node.frontier())
                }
                (SealedDailyParent::Continued(input), Some(parent)) => {
                    verify_continued_daily_sources(
                        self.raw,
                        self.base,
                        &parent.as_parent(),
                        parent.frontier(),
                        input,
                        node.frontier(),
                    )
                }
                _ => Err(ArchiveSourceError::History),
            };
            if let Err(error) = validation {
                source_error = Some(error);
                return Err(crate::daily_price_only::replay::DailyReplayError::InvalidChain);
            }
            Ok(())
        });
        let raw = self.raw.revalidate();
        let base = self.base.revalidate();
        let custody = raw.and(base);
        if let Some(error) = source_error {
            return Err(error);
        }
        result.map_err(archive_history_error)?;
        custody
    }

    /// Consumes the non-clone owner; failure, cancellation or unwinding drops
    /// every retained leaf. Callback data are descriptive and cannot outlive the
    /// actual owners. No arbitrary callback result becomes archive authority.
    pub fn consume(
        self,
        mut keep_reading: impl FnMut() -> bool,
        mut consume: impl FnMut(
            &SealedDailyParent<'_, '_>,
            &SealedSourceFrontier,
        ) -> Result<(), ArchiveSourceError>,
    ) -> Result<(), ArchiveSourceError> {
        self.raw.revalidate()?;
        self.base.revalidate()?;
        let mut consumer_error = None;
        let result = self.history.visit(&mut keep_reading, |node, _| {
            let raw = self.raw.revalidate();
            let base = self.base.revalidate();
            let before = raw.and(base);
            let used = match before {
                Ok(()) => consume(&node.as_parent(), node.frontier()),
                Err(error) => Err(error),
            };
            let raw = self.raw.revalidate();
            let base = self.base.revalidate();
            let after = raw.and(base);
            if let Err(error) = used.and(after) {
                consumer_error = Some(error);
                return Err(crate::daily_price_only::replay::DailyReplayError::InvalidChain);
            }
            Ok(())
        });
        let raw = self.raw.revalidate();
        let base = self.base.revalidate();
        let custody = raw.and(base);
        if let Some(error) = consumer_error {
            return Err(error);
        }
        result.map_err(archive_history_error)?;
        custody
    }
}

/// The two already-adopted historical contracts. This selects no new approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoricalArchiveVersion {
    V2,
    V3,
}

/// Borrowed actual original files; construction is confined to the fixed-class
/// consumer below. A caller cannot manufacture this from hashes or metadata.
pub(crate) struct HistoricalArtifactView<'a> {
    files: &'a RetainedFiles,
    path: &'a std::path::Path,
}
impl HistoricalArtifactView<'_> {
    pub(crate) fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        self.files.revalidate()
    }
    pub(crate) fn path(&self) -> &std::path::Path {
        self.path
    }
    pub(crate) fn manifest_bytes(&self, maximum: usize) -> Result<Vec<u8>, ArchiveSourceError> {
        self.files.read_original(0, maximum)
    }
    pub(crate) fn bars_file(&self) -> Result<(&std::fs::File, u64), ArchiveSourceError> {
        self.revalidate()?;
        let leaf = self
            .files
            .leaves
            .get(1)
            .ok_or(ArchiveSourceError::Custody)?;
        let size = leaf.identity.regular.ok_or(ArchiveSourceError::Custody)?.1;
        Ok((
            &leaf.file,
            u64::try_from(size).map_err(|_| ArchiveSourceError::Custody)?,
        ))
    }
}
impl RetainedFiles {
    fn read_original(&self, index: usize, maximum: usize) -> Result<Vec<u8>, ArchiveSourceError> {
        use std::os::unix::fs::FileExt;
        self.revalidate()?;
        let result = (|| {
            let leaf = self.leaves.get(index).ok_or(ArchiveSourceError::Custody)?;
            let size = usize::try_from(leaf.identity.regular.ok_or(ArchiveSourceError::Custody)?.1)
                .map_err(|_| ArchiveSourceError::Custody)?;
            if size > maximum {
                return Err(ArchiveSourceError::Context);
            }
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size)
                .map_err(|_| ArchiveSourceError::Context)?;
            bytes.resize(size, 0);
            leaf.file
                .read_exact_at(&mut bytes, 0)
                .map_err(|_| ArchiveSourceError::Custody)?;
            if ContentHash::from_bytes(&bytes) != leaf.sha256 {
                return Err(ArchiveSourceError::Custody);
            }
            Ok(bytes)
        })();
        let custody = self.revalidate();
        custody.and(result)
    }
    fn retire(self) -> Result<(), ArchiveSourceError> {
        let attempt = self.attempt.clone();
        drop(self); // Every checked File owner performs exactly one native close.
        if attempt.as_ref().is_some_and(|attempt| !attempt.check()) {
            Err(ArchiveSourceError::Custody)
        } else {
            Ok(())
        }
    }
}

/// Fixed descriptive traversal. All Raw manifest members, including unselected
/// committed history, remain under custody and are visited. Control locks are
/// never file evidence. A callback must not publish provisional data as success.
pub enum HistoricalSourceMember<'a> {
    RawManifest {
        provider: &'static str,
        bytes: &'a [u8],
    },
    RawBatch {
        provider: &'static str,
        batch_id: domain::BatchId,
        bytes: &'a [u8],
    },
    RawFile {
        provider: &'static str,
        batch_id: domain::BatchId,
        name: &'a str,
        bytes: &'a [u8],
    },
    ArtifactManifest {
        file: &'a std::fs::File,
        size: u64,
        sha256: &'a ContentHash,
    },
    ArtifactBars {
        file: &'a std::fs::File,
        size: u64,
        sha256: &'a ContentHash,
    },
    ApprovalRegistry {
        file: &'a std::fs::File,
        size: u64,
        sha256: &'a ContentHash,
    },
}

struct HistoricalRawReader<'s, 'i, 'a> {
    sources: &'s [crate::daily_price_only::source::CheckedRawSnapshot<'i, 'a>],
    expected: &'s Attempt,
    captured: &'s Attempt,
    keep_reading: &'s dyn Fn() -> bool,
}
impl HistoricalRawReader<'_, '_, '_> {
    fn revalidate(&self) -> Result<(), ArchiveSourceError> {
        if !(self.keep_reading)() {
            return Err(ArchiveSourceError::Interrupted);
        }
        let mut clean = self.expected.same_scope(self.captured)
            && self.expected.check()
            && self.captured.check();
        for source in self.sources {
            clean &= source
                .revalidate_originals_in(self.expected, self.captured)
                .is_ok();
        }
        if clean {
            Ok(())
        } else {
            Err(ArchiveSourceError::Custody)
        }
    }
    fn scope(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<crate::daily_price_only::source::CheckedRawSnapshotView<'_>, ArchiveSourceError>
    {
        self.revalidate()?;
        if market != "kr" || historical_provider(provider).is_err() {
            return Err(ArchiveSourceError::Context);
        }
        let mut found = None;
        for source in self.sources {
            let view = source
                .view_in(self.expected, self.captured)
                .map_err(|_| ArchiveSourceError::Custody)?;
            let scope = view.scope().ok_or(ArchiveSourceError::Custody)?;
            if !view.is_complete() || scope.trusted_raw_owner_uid() != 10001 {
                return Err(ArchiveSourceError::Custody);
            }
            if scope.provider() == provider {
                if found.is_some() {
                    return Err(ArchiveSourceError::Context);
                }
                found = Some(view);
            }
        }
        found.ok_or(ArchiveSourceError::Context)
    }
    fn batch(
        &self,
        provider: &str,
        id: domain::BatchId,
    ) -> Result<crate::daily_price_only::source::CheckedRawBatchView<'_>, ArchiveSourceError> {
        let view = self.scope(provider, "kr")?;
        view.batches()
            .find(|batch| batch.batch_id() == id)
            .ok_or(ArchiveSourceError::Reconstruction)
    }
    fn stored(
        &self,
        provider: &str,
        entry: &crate::storage::ManifestEntry,
    ) -> Result<Vec<crate::contract::StoredFile>, ArchiveSourceError> {
        let view = self.scope(provider, &entry.market)?;
        let root = view.data_root().ok_or(ArchiveSourceError::Custody)?;
        let batch = view
            .batches()
            .find(|batch| batch.batch_id() == entry.batch_id)
            .ok_or(ArchiveSourceError::Reconstruction)?;
        if batch.entry() != entry {
            return Err(ArchiveSourceError::Pin);
        }
        let mut stored = Vec::new();
        stored
            .try_reserve_exact(entry.files.len())
            .map_err(|_| ArchiveSourceError::Context)?;
        for file in batch.files() {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(file.bytes().len())
                .map_err(|_| ArchiveSourceError::Context)?;
            bytes.extend_from_slice(file.bytes());
            // This descriptive path is derived solely from the actual captured
            // root and validated member. Algorithms never open it.
            let path = root
                .join("raw")
                .join(format!("provider={provider}"))
                .join("market=kr")
                .join(format!("date={}", entry.date))
                .join(format!("batch={}", entry.batch_id))
                .join(file.file_name());
            stored.push(crate::contract::StoredFile {
                file_name: file.file_name().to_owned(),
                bytes,
                storage_path: path,
            });
        }
        self.revalidate()?;
        Ok(stored)
    }
    fn v3_line_hash(
        &self,
        provider: &str,
        batch: &crate::daily_price_only::source::CheckedRawBatchView<'_>,
    ) -> Result<ContentHash, ArchiveSourceError> {
        let view = self.scope(provider, "kr")?;
        let manifest = view.manifest_bytes().ok_or(ArchiveSourceError::Custody)?;
        let pin = match provider {
            "kis" => crate::range_to_canonical_v3::HISTORICAL_PRICE_ONLY_V3_ACTION_MANIFEST_LINE_SHA256,
            "kis-daily-range" => crate::range_to_canonical_v3_price::HISTORICAL_PRICE_ONLY_V3_PRICE_MANIFEST_LINE_SHA256,
            _ => return Err(ArchiveSourceError::Context),
        };
        let pin = ContentHash::parse(pin).map_err(|_| ArchiveSourceError::Pin)?;
        pinned_terminated_line_hash(manifest, batch.entry(), &pin)
    }
}

// V3 pins include the actual terminating LF. Daily's line hash excludes it.
// Read the retained slice; never append a synthetic terminator or hash a JSON
// reserialization. Missing LF, wrong physical line or changed bytes is failure.
fn original_terminated_line_hash(
    manifest: &[u8],
    number: usize,
    expected: &[u8],
) -> Result<ContentHash, ArchiveSourceError> {
    let index = number.checked_sub(1).ok_or(ArchiveSourceError::Pin)?;
    let line = manifest
        .split_inclusive(|byte| *byte == b'\n')
        .nth(index)
        .ok_or(ArchiveSourceError::Pin)?;
    if !line.ends_with(b"\n") || &line[..line.len() - 1] != expected {
        return Err(ArchiveSourceError::Pin);
    }
    Ok(ContentHash::from_bytes(line))
}

fn pinned_terminated_line_hash(
    manifest: &[u8],
    entry: &crate::storage::ManifestEntry,
    expected: &ContentHash,
) -> Result<ContentHash, ArchiveSourceError> {
    // Equivalent repeats retain their distinct original line bytes. The adopted
    // V3 pin must identify an actually retained line of this exact typed batch;
    // choosing only the first equivalent record would hide valid later originals.
    if !manifest.ends_with(b"\n") {
        return Err(ArchiveSourceError::Pin);
    }
    for (index, line) in manifest.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !line.ends_with(b"\n") {
            return Err(ArchiveSourceError::Pin);
        }
        let hash = ContentHash::from_bytes(line);
        if &hash == expected {
            let original: crate::storage::ManifestEntry =
                serde_json::from_slice(&line[..line.len() - 1])
                    .map_err(|_| ArchiveSourceError::Pin)?;
            if &original != entry {
                return Err(ArchiveSourceError::Pin);
            }
            return original_terminated_line_hash(
                manifest,
                index.checked_add(1).ok_or(ArchiveSourceError::Pin)?,
                &line[..line.len() - 1],
            );
        }
    }
    Err(ArchiveSourceError::Pin)
}

impl crate::range_to_canonical::HistoricalRawRead for HistoricalRawReader<'_, '_, '_> {
    fn read_committed_manifest(
        &self,
        provider: &str,
        market: &str,
    ) -> Result<Vec<crate::storage::ManifestEntry>, crate::range_to_canonical::RangeCanonicalError>
    {
        let result = (|| {
            let view = self.scope(provider, market)?;
            let entries = view.parsed_entries().cloned().collect();
            self.revalidate()?;
            Ok(entries)
        })();
        result.map_err(historical_read_error)
    }
    fn read_batch_bytes(
        &self,
        provider: &str,
        market: &str,
        entry: &crate::storage::ManifestEntry,
    ) -> Result<Vec<crate::contract::StoredFile>, crate::range_to_canonical::RangeCanonicalError>
    {
        if market != "kr" || entry.market != market || entry.provider != provider {
            return Err(historical_read_error(ArchiveSourceError::Context));
        }
        self.stored(provider, entry).map_err(historical_read_error)
    }
}
fn historical_read_error(_: ArchiveSourceError) -> crate::range_to_canonical::RangeCanonicalError {
    crate::range_to_canonical::RangeCanonicalError::HistoricalBetaContract {
        reason: "HISTORICAL_ORIGINAL_CUSTODY".to_owned(),
    }
}

fn compare_historical_raw_bars(
    source: &[crate::range_to_canonical::RangeCanonicalBarCandidate],
    artifact: &[crate::HistoricalPriceOnlyBar],
) -> Result<(), ArchiveSourceError> {
    let mut remaining = std::collections::BTreeMap::new();
    for bar in source {
        if remaining
            .insert((&bar.instrument_id, bar.session_date), bar)
            .is_some()
        {
            return Err(ArchiveSourceError::Pin);
        }
    }
    if source.len() != artifact.len() {
        return Err(ArchiveSourceError::Pin);
    }
    for bar in artifact {
        let original = remaining
            .remove(&(&bar.instrument_id, bar.session_date))
            .ok_or(ArchiveSourceError::Pin)?;
        if original.open != bar.raw_open
            || original.high != bar.raw_high
            || original.low != bar.raw_low
            || original.close != bar.raw_close
            || original.volume != bar.raw_volume
            || original.trading_value != bar.raw_trading_value
        {
            return Err(ArchiveSourceError::Pin);
        }
    }
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(ArchiveSourceError::Pin)
    }
}

fn verify_historical_dependencies(
    version: HistoricalArchiveVersion,
    raw: &HistoricalRawReader<'_, '_, '_>,
    artifact: &HistoricalArtifactView<'_>,
    approved: &ApprovedHistoricalPriceOnlyArtifact,
) -> Result<(), ArchiveSourceError> {
    raw.revalidate()?;
    let result = (|| match version {
        HistoricalArchiveVersion::V2 => {
            let input =
                crate::range_to_canonical::verify_historical_price_only_beta_input_from_reader(
                    raw,
                    approved.pins().stage5_manifest_sha256(),
                    approved.pins().action_manifest_sha256(),
                )
                .map_err(|_| ArchiveSourceError::Reconstruction)?;
            crate::historical_price_only_artifact::verify_retained_historical_v2_dependencies(
                artifact,
                approved.pins().candidate_content_sha256(),
                &input,
            )
            .map_err(|_| ArchiveSourceError::Pin)?;
            compare_historical_raw_bars(input.bars(), approved.bars())
        }
        HistoricalArchiveVersion::V3 => {
            use crate::range_to_canonical_v3::*;
            use crate::range_to_canonical_v3_price::*;
            let price_id = HISTORICAL_PRICE_ONLY_V3_PRICE_BATCH_ID
                .parse()
                .map_err(|_| ArchiveSourceError::Pin)?;
            let action_id = HISTORICAL_PRICE_ONLY_V3_ACTION_BATCH_ID
                .parse()
                .map_err(|_| ArchiveSourceError::Pin)?;
            let price_batch = raw.batch("kis-daily-range", price_id)?;
            let price_files = raw.stored("kis-daily-range", price_batch.entry())?;
            let price_line = raw.v3_line_hash("kis-daily-range", &price_batch)?;
            let prices = verify_historical_price_only_v3_price_input(
                price_batch.entry(),
                &price_files,
                price_batch.batch_json_sha256(),
                &price_line,
            )
            .map_err(|_| ArchiveSourceError::Reconstruction)?;
            drop(price_files);
            let action_batch = raw.batch("kis", action_id)?;
            let action_files = raw.stored("kis", action_batch.entry())?;
            let action_line = raw.v3_line_hash("kis", &action_batch)?;
            let actions = verify_historical_price_only_v3_action_input(
                action_batch.entry(),
                &action_files,
                action_batch.batch_json_sha256(),
                &action_line,
            )
            .map_err(|_| ArchiveSourceError::Reconstruction)?;
            let summary = approved
                .v3_approval_summary()
                .ok_or(ArchiveSourceError::Pin)?;
            if summary.price_batch_id() != prices.source_batch_id()
                || summary.price_batch_json_sha256() != prices.source_batch_json_sha256()
                || summary.price_manifest_line_sha256() != prices.source_manifest_line_sha256()
                || summary.price_bars_sha256() != prices.bars_sha256()
                || summary.price_file_count() != prices.files().len()
                || summary.price_capture_contract_commit() != prices.capture_contract_commit()
                || summary.price_response_marker_evidence() != prices.response_marker_evidence()
                || summary.price_acquired_at() != prices.acquired_at()
                || summary.range_start() != prices.range_start()
                || summary.range_end() != prices.range_end()
                || summary.session_count() != prices.session_count()
                || summary.row_count() != prices.bar_count()
                || summary.action_batch_id() != actions.source_batch_id()
                || summary.action_batch_json_sha256() != actions.source_batch_json_sha256()
                || summary.action_manifest_line_sha256() != actions.source_manifest_line_sha256()
                || summary.action_file_count() != actions.files().len()
                || summary.action_count() != actions.action_count()
                || summary.cash_dividend_treatment_id() != actions.cash_dividends().treatment_id()
                || summary.cash_dividend_row_count() != actions.cash_dividends().row_count()
                || summary.cash_dividend_rows_sha256() != actions.cash_dividends().rows_sha256()
                || summary.action_acquired_at() != actions.cash_dividends().acquired_at()
            {
                return Err(ArchiveSourceError::Pin);
            }
            compare_historical_raw_bars(prices.bars(), approved.bars())
        }
    })();
    let custody = raw.revalidate().and(artifact.revalidate());
    custody.and(result)
}

fn historical_provider(
    provider: &str,
) -> Result<crate::daily_price_only::source::HistoricalRawProvider, ArchiveSourceError> {
    use crate::daily_price_only::source::HistoricalRawProvider as P;
    match provider {
        "kis" => Ok(P::Actions),
        "kis-calendar" => Ok(P::Calendar),
        "kis-daily-range" => Ok(P::Prices),
        "kis-daily-range-normalized" => Ok(P::Normalized),
        _ => Err(ArchiveSourceError::Context),
    }
}

// Replays storage.rs::validate_copied_sources against the actual retained pair.
// Storage admits only original one-hop copies, with the explicit calendar
// exception. No Daily-depth extension, re-encoding or invented retrieval time.
fn verify_historical_copy(
    destination: &crate::storage::ManifestEntry,
    file: &crate::storage::FileEntry,
    bytes: &[u8],
    source: &crate::storage::ManifestEntry,
    original: &crate::storage::FileEntry,
    original_bytes: &[u8],
) -> Result<(), ArchiveSourceError> {
    let reference = file.copied_from.as_ref().ok_or(ArchiveSourceError::Pin)?;
    let calendar = destination.provider == "kis"
        && destination.market == "kr"
        && source.provider == "kis-calendar"
        && source.market == destination.market
        && source.mode == crate::FetchMode::Credentialed
        && source.date < destination.date
        && file.kind == crate::ResponseKind::Calendar;
    if reference.provider != source.provider
        || reference.market != source.market
        || reference.batch_id != source.batch_id
        || reference.batch_id == destination.batch_id
        || reference.retrieved_at != source.retrieved_at
        || source.retrieved_at > destination.retrieved_at
        || (source.date != destination.date && !calendar)
        || source.mode != destination.mode
        || source.entitlement_reference != destination.entitlement_reference
        || reference.file_name != original.file_name
        || original.file_name != file.file_name
        || reference.content_hash != original.content_hash
        || original.content_hash != file.content_hash
        || original.kind != file.kind
        || original.size_bytes != file.size_bytes
        || original.request != file.request
        || original.response_continuation != file.response_continuation
        || original.copied_from.is_some()
        || original_bytes != bytes
    {
        return Err(ArchiveSourceError::Pin);
    }
    Ok(())
}

impl HistoricalRawReader<'_, '_, '_> {
    fn verify_copies(&self) -> Result<(), ArchiveSourceError> {
        self.revalidate()?;
        for source in self.sources {
            let view = source
                .view_in(self.expected, self.captured)
                .map_err(|_| ArchiveSourceError::Custody)?;
            let provider = view.scope().ok_or(ArchiveSourceError::Custody)?.provider();
            if ["krx", "kis-normalized", "kis-calendar-normalized"].contains(&provider) {
                // These are separate fixed archive publication scopes, not a
                // widening of the historical or live Daily provider sets.
                let checked = crate::daily_price_only::source::with_checked_raw_originals(
                    self.sources,
                    self.expected,
                    self.captured,
                    |all| {
                        let reader = CapturedPublicationReader {
                            originals: all,
                            keep_reading: self.keep_reading,
                        };
                        for batch in view.batches() {
                            if provider == "kis-normalized" {
                                // Include committed normalized originals even
                                // when no Curated generation selects this batch.
                                crate::normalize::verify_kis_normalized_originals(
                                    &reader,
                                    batch.entry(),
                                )
                                .map_err(|_| ArchiveSourceError::Reconstruction)?;
                            }
                            if provider == "kis-calendar-normalized" {
                                crate::publication::CalendarPublicationBundle::from_originals(
                                    &reader,
                                    batch.entry(),
                                )
                                .map_err(|_| ArchiveSourceError::Reconstruction)?;
                            } else {
                                crate::publication::PublicationBundle::from_originals(
                                    &reader,
                                    batch.entry(),
                                )
                                .map_err(|_| ArchiveSourceError::Reconstruction)?;
                            }
                            if provider == "krx"
                                && batch
                                    .entry()
                                    .files
                                    .iter()
                                    .any(|file| file.copied_from.is_some())
                            {
                                return Err(ArchiveSourceError::Context);
                            }
                        }
                        reader.revalidate().map_err(|_| ArchiveSourceError::Custody)
                    },
                );
                checked.map_err(|failure| {
                    failure
                        .callback
                        .and_then(Result::err)
                        .unwrap_or(ArchiveSourceError::Custody)
                })?;
                continue;
            }
            for batch in view.batches() {
                for file in batch.files() {
                    let Some(reference) = file.entry().copied_from.as_ref() else {
                        continue;
                    };
                    let original_scope = self.scope(&reference.provider, &reference.market)?;
                    let original = original_scope
                        .lookup_exact(reference)
                        .map_err(|_| ArchiveSourceError::Pin)?;
                    verify_historical_copy(
                        batch.entry(),
                        file.entry(),
                        file.bytes(),
                        original.batch().entry(),
                        original.entry(),
                        original.bytes(),
                    )?;
                }
            }
        }
        self.revalidate()
    }
}

// If a callback unwinds, no caller can reuse the shared attempt as clean.
struct HistoricalUseGuard<'a> {
    attempt: &'a Attempt,
    armed: bool,
}
impl Drop for HistoricalUseGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.attempt.gap(crate::checked_close::Gap::Abandoned);
        }
    }
}

fn historical_callback(
    expected: &Attempt,
    captured: &Attempt,
    keep_reading: &mut impl FnMut() -> bool,
    before_after: impl Fn() -> Result<(), ArchiveSourceError>,
    body: impl FnOnce() -> Result<(), ArchiveSourceError>,
) -> Result<(), ArchiveSourceError> {
    let mut guard = HistoricalUseGuard {
        attempt: expected,
        armed: true,
    };
    if !expected.same_scope(captured) || !expected.check() || !captured.check() {
        return Err(ArchiveSourceError::Custody);
    }
    if !keep_reading() {
        return Err(ArchiveSourceError::Interrupted);
    }
    before_after()?;
    let result = body();
    let custody = before_after();
    if !keep_reading() {
        return Err(ArchiveSourceError::Interrupted);
    }
    custody.and(result)?;
    if !expected.check() || !captured.check() {
        return Err(ArchiveSourceError::Custody);
    }
    guard.armed = false;
    Ok(())
}

/// Consumes real retained historical originals through the existing V2/V3
/// algorithms and a fixed descriptive traversal. The caller supplies its actual
/// native invocation/attempts and the installed Raw, artifact, registry and
/// external coordination roots; no credentials or lock is archive evidence.
///
/// Limits are per complete provider scope (at most four fixed scopes); checked
/// arithmetic bounds their total plus two artifact leaves and one registry.
/// All outputs remain provisional until this returns AND the caller completes
/// its generation, held database snapshot, transport and terminal close checks.
/// This function neither creates that authority nor completes a source_files audit.
#[expect(
    clippy::too_many_arguments,
    reason = "Keep actual custody inputs explicit; no serialized authority bundle."
)]
pub fn consume_historical_originals(
    invocation: &crate::raw_custody::RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
    version: HistoricalArchiveVersion,
    raw_root: &std::path::Path,
    coordination_root: &std::path::Path,
    artifact_root: &std::path::Path,
    registry_root: &std::path::Path,
    limits: crate::daily_price_only::source::ScanLimitValues,
    keep_reading: impl FnMut() -> bool,
    consume: impl FnMut(HistoricalSourceMember<'_>) -> Result<(), ArchiveSourceError>,
) -> Result<(), ArchiveSourceError> {
    consume_historical_originals_with_material(
        invocation,
        expected,
        captured,
        HistoricalOriginalSelection {
            version,
            raw_root,
            coordination_root,
            artifact_root,
            registry_root,
            limits,
        },
        keep_reading,
        consume,
        |_| Ok(()),
    )
}

/// Descriptive path/limit selection only; contains no approval or source authority.
pub struct HistoricalOriginalSelection<'a> {
    pub version: HistoricalArchiveVersion,
    pub raw_root: &'a std::path::Path,
    pub coordination_root: &'a std::path::Path,
    pub artifact_root: &'a std::path::Path,
    pub registry_root: &'a std::path::Path,
    pub limits: crate::daily_price_only::source::ScanLimitValues,
}

/// Borrows the actual approved artifact only inside the retained original pass.
/// The material cannot escape; callbacks remain provisional through final close.
pub fn consume_historical_originals_with_material(
    invocation: &crate::raw_custody::RawInvocation<'_>,
    expected: &Attempt,
    captured: &Attempt,
    selection: HistoricalOriginalSelection<'_>,
    keep_reading: impl FnMut() -> bool,
    mut consume: impl FnMut(HistoricalSourceMember<'_>) -> Result<(), ArchiveSourceError>,
    mut consume_material: impl FnMut(
        &ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<(), ArchiveSourceError>,
) -> Result<(), ArchiveSourceError> {
    use crate::daily_price_only::source::{CommittedRawReader, ScanLimits};
    let HistoricalOriginalSelection {
        version,
        raw_root,
        coordination_root,
        artifact_root,
        registry_root,
        limits,
    } = selection;
    let mut guard = HistoricalUseGuard {
        attempt: expected,
        armed: true,
    };
    let mut sources = Vec::new();
    let mut artifact_files = None;
    let mut registry_files = None;
    let keep_reading = std::cell::RefCell::new(keep_reading);
    let continuing = || (*keep_reading.borrow_mut())();
    let body = (|| {
        if !expected.same_scope(captured) || !expected.check() || !captured.check() {
            return Err(ArchiveSourceError::Custody);
        }
        // Reject aggregate arithmetic overflow before allocating or opening.
        for cap in [
            limits.max_retained_bytes,
            limits.max_open_descriptors,
            limits.max_manifest_records,
            limits.max_batches,
            limits.max_files,
        ] {
            cap.checked_mul(4)
                .and_then(|n| n.checked_add(134_217_728))
                .filter(|n| *n <= isize::MAX as usize)
                .ok_or(ArchiveSourceError::Context)?;
        }
        ScanLimits::new(limits).map_err(|_| ArchiveSourceError::Context)?;
        let mut required = std::collections::BTreeSet::from(["kis", "kis-daily-range"]);
        if version == HistoricalArchiveVersion::V2 {
            required.insert("kis-daily-range-normalized");
        }
        let mut visited = std::collections::BTreeSet::new();
        loop {
            let next = required
                .iter()
                .find(|name| !visited.contains(**name))
                .copied();
            let Some(provider) = next else {
                break;
            };
            if !continuing() {
                return Err(ArchiveSourceError::Interrupted);
            }
            let snapshot = CommittedRawReader::new(raw_root)
                .historical_snapshot_checked_in(
                    invocation,
                    expected,
                    captured,
                    coordination_root,
                    historical_provider(provider)?,
                    ScanLimits::new(limits).map_err(|_| ArchiveSourceError::Context)?,
                )
                .map_err(|_| ArchiveSourceError::Custody)?;
            sources.push(snapshot);
            let current = sources.last().ok_or(ArchiveSourceError::Custody)?;
            current
                .revalidate_originals_in(expected, captured)
                .map_err(|_| ArchiveSourceError::Custody)?;
            let view = current
                .view_in(expected, captured)
                .map_err(|_| ArchiveSourceError::Custody)?;
            for entry in view.parsed_entries() {
                for file in &entry.files {
                    if let Some(reference) = &file.copied_from {
                        if reference.market != "kr" {
                            return Err(ArchiveSourceError::Context);
                        }
                        required
                            .insert(historical_provider(&reference.provider)?.scope().provider());
                    }
                }
            }
            visited.insert(provider);
        }
        let raw = HistoricalRawReader {
            sources: &sources,
            expected,
            captured,
            keep_reading: &continuing,
        };
        raw.verify_copies()?;
        let pins = match version {
            HistoricalArchiveVersion::V2 => {
                crate::historical_price_only_approval::embedded_v2_archive_pins()
            }
            HistoricalArchiveVersion::V3 => {
                crate::historical_price_only_approval::embedded_v3_archive_pins()
            }
        }
        .map_err(|_| ArchiveSourceError::Pin)?;
        let candidate = format!(
            "candidate-sha256={}",
            pins.candidate_content_sha256()
                .as_str()
                .strip_prefix("sha256:")
                .ok_or(ArchiveSourceError::Pin)?
        );
        let (leaf, registry_name) = match version {
            HistoricalArchiveVersion::V2 => (
                artifact_root
                    .join("kis-historical-price-only-beta")
                    .join("v2")
                    .join(candidate),
                "kis-historical-price-only-beta-approved-artifacts.json",
            ),
            HistoricalArchiveVersion::V3 => (
                artifact_root.join(
                    crate::historical_price_only_v3_artifact::candidate_directory_name(
                        pins.candidate_content_sha256(),
                    )
                    .map_err(|_| ArchiveSourceError::Pin)?,
                ),
                "kis-historical-price-only-v3-approved-artifacts.json",
            ),
        };
        registry_files = Some(RetainedFiles::open_with_attempt(
            registry_root,
            &[registry_name],
            0,
            0o644,
            262_144,
            Some(expected),
        )?);
        artifact_files = Some(RetainedFiles::open_with_attempt(
            &leaf,
            &["manifest.json", "bars.ndjson"],
            10001,
            0o600,
            134_217_728,
            Some(expected),
        )?);
        let registry = registry_files.as_ref().ok_or(ArchiveSourceError::Custody)?;
        let files = artifact_files.as_ref().ok_or(ArchiveSourceError::Custody)?;
        let artifact = HistoricalArtifactView { files, path: &leaf };
        let registry_bytes = registry.read_original(0, 262_144)?;
        let approved =
            crate::historical_price_only_approval::approve_retained_historical_originals(
                version,
                &artifact,
                &registry_bytes,
            )
            .map_err(|_| ArchiveSourceError::Pin)?;
        if approved.pins() != &pins {
            return Err(ArchiveSourceError::Pin);
        }
        verify_historical_dependencies(version, &raw, &artifact, &approved)?;
        let revalidate = || {
            let raw = raw.revalidate();
            let artifact = artifact.revalidate();
            let registry = registry.revalidate();
            raw.and(artifact).and(registry)
        };
        // Lexical fixed provider order, then original committed batch order and
        // original file order. Full manifest preserves equivalent duplicate lines.
        for provider in visited {
            let view = raw.scope(provider, "kr")?;
            let bytes = view.manifest_bytes().ok_or(ArchiveSourceError::Custody)?;
            historical_callback(expected, captured, &mut || continuing(), revalidate, || {
                consume(HistoricalSourceMember::RawManifest { provider, bytes })
            })?;
            for batch in view.batches() {
                let batch_id = batch.batch_id();
                historical_callback(expected, captured, &mut || continuing(), revalidate, || {
                    consume(HistoricalSourceMember::RawBatch {
                        provider,
                        batch_id,
                        bytes: batch.batch_json_bytes(),
                    })
                })?;
                for file in batch.files() {
                    historical_callback(
                        expected,
                        captured,
                        &mut || continuing(),
                        revalidate,
                        || {
                            consume(HistoricalSourceMember::RawFile {
                                provider,
                                batch_id,
                                name: file.file_name(),
                                bytes: file.bytes(),
                            })
                        },
                    )?;
                }
            }
        }
        for (index, original) in files.leaves.iter().enumerate() {
            let size = original
                .identity
                .regular
                .ok_or(ArchiveSourceError::Custody)?
                .1 as u64;
            historical_callback(expected, captured, &mut || continuing(), revalidate, || {
                consume(if index == 0 {
                    HistoricalSourceMember::ArtifactManifest {
                        file: &original.file,
                        size,
                        sha256: &original.sha256,
                    }
                } else {
                    HistoricalSourceMember::ArtifactBars {
                        file: &original.file,
                        size,
                        sha256: &original.sha256,
                    }
                })
            })?;
        }
        let original = &registry.leaves[0];
        let size = original
            .identity
            .regular
            .ok_or(ArchiveSourceError::Custody)?
            .1 as u64;
        historical_callback(expected, captured, &mut || continuing(), revalidate, || {
            consume(HistoricalSourceMember::ApprovalRegistry {
                file: &original.file,
                size,
                sha256: &original.sha256,
            })
        })?;
        historical_callback(expected, captured, &mut || continuing(), revalidate, || {
            consume_material(&approved)
        })?;
        revalidate()
    })();
    // Always recheck every captured source, including error/cancellation paths,
    // before retiring all actual owners. Never return a cached checked bool.
    let mut clean = true;
    for source in &sources {
        clean &= source.revalidate_originals_in(expected, captured).is_ok();
    }
    for files in [&artifact_files, &registry_files].into_iter().flatten() {
        clean &= files.revalidate().is_ok();
    }
    if body.is_err() || !clean {
        expected.gap(crate::checked_close::Gap::Abandoned);
    }
    for source in sources {
        clean &= source.finish_local().is_clean();
    }
    if let Some(files) = artifact_files {
        clean &= files.retire().is_ok();
    }
    if let Some(files) = registry_files {
        clean &= files.retire().is_ok();
    }
    clean &= expected.check() && captured.check();
    if !clean {
        return Err(body.err().unwrap_or(ArchiveSourceError::Custody));
    }
    body?;
    guard.armed = false;
    Ok(())
}
