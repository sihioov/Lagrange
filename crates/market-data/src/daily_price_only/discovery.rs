//! Complete read-only session discovery over approved captured scopes.
//! Source reconstruction is not protected-prefix verification or admission.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;

use chrono::Datelike;
use domain::{ContentHash, InstrumentId, TradingDate};
use serde_json::Value;
use uuid::Uuid;

use super::calendar::{
    VerifiedCalendarProjection, eligible_calendar_date, project_calendar_candidates,
    prove_calendar_range,
};
use super::contract::{canonicalize_session, fixed_limits, validate_file_request_for_symbol};
use super::observation::{CanonicalSourceObservation, canonical_source_observation};
use super::reconstruct::{
    ActionSourceRanges, VerifiedActionDocument, VerifiedPriceDocuments, reconstruct_actions,
    reconstruct_indexed_actions, reconstruct_prices,
};
use super::verify::{
    ChargeSourceWork, DailyVerificationError as Error, RawEvidenceSnapshots, ResolvedOriginal,
    parse_provider_json, resolve_original_charged,
};
use super::{
    DailyCalendarBindingV1Wire, DailyEvidenceKindV1 as Kind, DailyFileEvidenceV1Wire,
    DailyNormalizedDocumentV1Wire, DailySessionEvidenceV1Wire, DailySessionTypeV1,
    daily_price_evidence_preimage_v1,
};
use crate::contract::{ResponseKind, SourceFileReference};
use crate::normalize::{canonical_bar, parse_kis_date};
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;

/// Requested facts, without provenance or a trusted-source-list field.
pub struct SessionSourceRequest {
    pub owner: Uuid,
    pub policy_sha256: ContentHash,
    pub previous_frontier: TradingDate,
    pub session_date: TradingDate,
}

/// Every validated row from captured daily bar originals, including surrounding
/// dates. The protected-prefix verifier must compare overlaps before admission.
pub struct ObservedBarRows {
    source: DailyFileEvidenceV1Wire,
    rows: Vec<Value>,
    coverage: BarRowCoverage,
}

impl ObservedBarRows {
    pub fn source(&self) -> &DailyFileEvidenceV1Wire {
        &self.source
    }
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }
    pub(super) fn coverage(&self) -> BarRowCoverage {
        self.coverage
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BarRowCoverage {
    RequestedDate,
    ClosedDayOverlap,
}

impl BarRowCoverage {
    pub(super) fn validate_dates(
        self,
        requested: TradingDate,
        mut dates: impl Iterator<Item = TradingDate>,
    ) -> Result<(), Error> {
        let valid = match self {
            Self::RequestedDate => dates.any(|date| date == requested),
            Self::ClosedDayOverlap => {
                dates.next().is_some_and(|date| date < requested)
                    && dates.all(|date| date < requested)
            }
        };
        valid.then_some(()).ok_or(Error::SourceCoverageGap)
    }
}

pub struct SessionSourceDocument {
    descriptor: DailyNormalizedDocumentV1Wire,
    bytes: Vec<u8>,
}

impl SessionSourceDocument {
    pub fn descriptor(&self) -> &DailyNormalizedDocumentV1Wire {
        &self.descriptor
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Private construction proves discovery and reconstruction only. The independent
/// admitter still reopens the protected v3/daily prefix and compares every observed
/// bar overlap, preserved frontier and candidate before installing any artifact.
/// No public constructor, Deserialize, approval, filesystem or DB capability.
pub struct DiscoveredSessionSources<'snapshot> {
    owner: Uuid,
    policy_sha256: ContentHash,
    session: DailySessionEvidenceV1Wire,
    evidence_preimage: Vec<u8>,
    documents: Vec<SessionSourceDocument>,
    observation: CanonicalSourceObservation,
    calendars: Vec<VerifiedCalendarProjection<'snapshot>>,
    observed_bars: Arc<[ObservedBarRows]>,
    prices: VerifiedPriceDocuments<'snapshot>,
    actions: VerifiedActionDocument<'snapshot>,
    observed_originals: Arc<[&'snapshot [u8]]>,
}

impl DiscoveredSessionSources<'_> {
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy_sha256
    }
    pub fn session(&self) -> &DailySessionEvidenceV1Wire {
        &self.session
    }
    pub fn evidence_preimage(&self) -> &[u8] {
        &self.evidence_preimage
    }
    pub fn documents(&self) -> &[SessionSourceDocument] {
        &self.documents
    }
    pub fn observation(&self) -> &CanonicalSourceObservation {
        &self.observation
    }
    pub fn calendars(&self) -> &[VerifiedCalendarProjection<'_>] {
        &self.calendars
    }
    pub fn observed_bars(&self) -> &[ObservedBarRows] {
        &self.observed_bars
    }
    pub fn prices(&self) -> &VerifiedPriceDocuments<'_> {
        &self.prices
    }
    pub fn actions(&self) -> &VerifiedActionDocument<'_> {
        &self.actions
    }
    /// Retained source owners also cover rows outside the requested extension.
    pub fn additional_original_count(&self) -> usize {
        self.observed_originals.len() - self.session.bars.len()
    }
}

fn file_reference(file: super::source::CommittedRawFile<'_>) -> SourceFileReference {
    let entry = file.batch().entry();
    SourceFileReference {
        provider: entry.provider.clone(),
        market: entry.market.clone(),
        batch_id: entry.batch_id,
        file_name: file.file_name().to_owned(),
        content_hash: file.content_hash().clone(),
        retrieved_at: entry.retrieved_at,
    }
}

fn original_date(source: &ResolvedOriginal<'_>) -> TradingDate {
    terminal_evidence_date(source.evidence())
}

fn terminal_evidence_date(evidence: &DailyFileEvidenceV1Wire) -> TradingDate {
    evidence
        .copied_from
        .last()
        .map_or(evidence.batch_date, |hop| hop.batch_date)
}

#[cfg(all(test, target_os = "linux"))]
#[path = "discovery_compat_tests.rs"]
mod compat_tests;

pub(super) type SealedCalendarDeliveries<'a> =
    BTreeMap<domain::BatchId, Vec<&'a DailyFileEvidenceV1Wire>>;

pub(super) fn sealed_calendar_deliveries(
    files: &[DailyFileEvidenceV1Wire],
) -> SealedCalendarDeliveries<'_> {
    let mut deliveries = SealedCalendarDeliveries::new();
    for file in files.iter().filter(|file| file.kind == Kind::Calendar) {
        deliveries.entry(file.batch_id).or_default().push(file);
    }
    deliveries
}

#[derive(Clone, Copy)]
pub(super) enum CalendarSelection<'a> {
    CurrentTargetPreferred,
    #[cfg(test)]
    LegacyHash,
    Sealed {
        bindings: &'a [DailyCalendarBindingV1Wire],
        deliveries: &'a SealedCalendarDeliveries<'a>,
    },
}

/// Enumerates the snapshots itself. Duplicate target bar/reference deliveries,
/// even numerically equal ones, fail closed: D4 v1 cannot represent multiple
/// files for one symbol, and no competing source is silently discarded.
pub fn discover_session_sources<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    request: &SessionSourceRequest,
) -> Result<DiscoveredSessionSources<'snapshot>, Error> {
    discover_session_sources_with_selection(
        snapshots,
        request,
        CalendarSelection::CurrentTargetPreferred,
    )
}

pub(super) fn discover_session_sources_with_selection<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    request: &SessionSourceRequest,
    selection: CalendarSelection<'_>,
) -> Result<DiscoveredSessionSources<'snapshot>, Error> {
    let start = validate_source_request(snapshots, request)?;
    let raw = collect_price_sources(snapshots, Some(request.session_date), &mut |_, _| Ok(()))?;
    let calendars =
        project_calendar_candidates(&raw.calendars, request.owner, &request.policy_sha256)?;
    let observed_bars = observe_bar_rows(&raw.bars, Some(request.session_date), &calendars)?;
    let observed_originals = raw
        .bars
        .iter()
        .map(ResolvedOriginal::original_bytes)
        .collect();
    let bars: Vec<_> = raw
        .bars
        .into_iter()
        .filter(|source| original_date(source) == request.session_date)
        .collect();
    let calendars = calendars
        .into_iter()
        .filter(|projection| calendar_touches(projection, start, request.session_date))
        .collect();
    assemble_session_sources(
        request,
        start,
        selection,
        SessionPriceSources {
            bars: &bars,
            reference: &raw.reference,
            calendars,
            observed_bars: observed_bars.into(),
            observed_originals,
        },
        || reconstruct_actions(snapshots, request.previous_frontier, request.session_date),
    )
}

fn validate_source_request(
    snapshots: &RawEvidenceSnapshots<'_>,
    request: &SessionSourceRequest,
) -> Result<TradingDate, Error> {
    let start = request
        .previous_frontier
        .checked_add_days(1)
        .map_err(|_| Error::InvalidContext)?;
    let span = request
        .session_date
        .as_naive_date()
        .signed_duration_since(start.as_naive_date())
        .num_days();
    if request.owner.is_nil()
        || !(0..370).contains(&span)
        || request.previous_frontier
            < TradingDate::parse("2026-08-28").map_err(|_| Error::InvalidContext)?
        || request.session_date > eligible_calendar_date(snapshots.observed_at())?
    {
        return Err(Error::InvalidContext);
    }
    Ok(start)
}

struct RawPriceSources<'snapshot> {
    bars: Vec<ResolvedOriginal<'snapshot>>,
    reference: Vec<ResolvedOriginal<'snapshot>>,
    calendars: Vec<ResolvedOriginal<'snapshot>>,
    price_bytes: u64,
}

fn collect_price_sources<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    reference_date: Option<TradingDate>,
    charge: ChargeSourceWork<'_>,
) -> Result<RawPriceSources<'snapshot>, Error> {
    let limits = fixed_limits();
    let mut captured = RawPriceSources {
        bars: Vec::new(),
        reference: Vec::new(),
        calendars: Vec::new(),
        price_bytes: 0,
    };
    let mut source_count = 0u64;
    let mut selected_reference_bytes = 0u64;
    for &snapshot in snapshots.scopes() {
        for batch in snapshots.batches(snapshot)? {
            if !snapshots.matches_entitlement(batch) {
                continue;
            }
            for file in batch.files() {
                let kind = match file.kind() {
                    ResponseKind::Bars => Kind::Bars,
                    ResponseKind::Reference => Kind::Reference,
                    ResponseKind::Calendar => Kind::Calendar,
                    _ => continue,
                };
                source_count = source_count.checked_add(1).ok_or(Error::LimitExceeded)?;
                if source_count > limits.max_file_evidence_per_session {
                    return Err(Error::LimitExceeded);
                }
                let source = resolve_original_charged(snapshots, &file_reference(file), charge)?;
                if matches!(kind, Kind::Bars | Kind::Reference) && !is_etf_price_source(&source)? {
                    continue;
                }
                if kind == Kind::Reference {
                    // An unrelated reference is not positive evidence. The
                    // prepared path stores its verified original for its own date.
                    if reference_date.is_none_or(|date| original_date(&source) == date) {
                        if reference_date.is_some() {
                            selected_reference_bytes = add_price_bytes(
                                selected_reference_bytes,
                                source.original_bytes().len() as u64,
                            )?;
                            add_price_bytes(captured.price_bytes, selected_reference_bytes)?;
                        }
                        captured.reference.push(source);
                    }
                    continue;
                }
                captured.price_bytes =
                    add_price_bytes(captured.price_bytes, source.original_bytes().len() as u64)?;
                add_price_bytes(captured.price_bytes, selected_reference_bytes)?;
                match kind {
                    Kind::Calendar => captured.calendars.push(source),
                    Kind::Bars => captured.bars.push(source),
                    _ => return Err(Error::InvalidEvidence),
                }
            }
        }
    }
    Ok(captured)
}

fn add_price_bytes(before: u64, added: u64) -> Result<u64, Error> {
    let total = before.checked_add(added).ok_or(Error::LimitExceeded)?;
    (total <= fixed_limits().max_bars_bytes)
        .then_some(total)
        .ok_or(Error::LimitExceeded)
}

fn check_price_bytes(bytes: u64, reference: &[ResolvedOriginal<'_>]) -> Result<(), Error> {
    reference.iter().try_fold(bytes, |size, file| {
        add_price_bytes(size, file.original_bytes().len() as u64)
    })?;
    Ok(())
}

fn calendar_touches(
    projection: &VerifiedCalendarProjection<'_>,
    start: TradingDate,
    end: TradingDate,
) -> bool {
    projection
        .days()
        .iter()
        .any(|day| (start..=end).contains(&day.session_date()))
}

fn observe_bar_rows(
    originals: &[ResolvedOriginal<'_>],
    session: Option<TradingDate>,
    calendars: &[VerifiedCalendarProjection<'_>],
) -> Result<Vec<ObservedBarRows>, Error> {
    let mut proven_closed_days = BTreeSet::new();
    let mut rows = originals
        .iter()
        .map(|source| inspect_bar_rows(source, session, calendars, &mut proven_closed_days))
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_by(|a, b| {
        a.source
            .provider
            .cmp(&b.source.provider)
            .then_with(|| {
                a.source
                    .batch_id
                    .to_string()
                    .cmp(&b.source.batch_id.to_string())
            })
            .then_with(|| a.source.file_name.cmp(&b.source.file_name))
    });
    Ok(rows)
}

/// An index owned by a single refresh of a single held Raw snapshot. The
/// common overlap rows are canonicalized once; selected positives still pass
/// the regular reconstructors, including provider identity and byte checks.
pub(super) struct IndexedSessionSources<'view, 'snapshot> {
    snapshots: &'view RawEvidenceSnapshots<'snapshot>,
    owner: Uuid,
    policy_sha256: ContentHash,
    bars: BTreeMap<TradingDate, Vec<ResolvedOriginal<'snapshot>>>,
    reference: BTreeMap<TradingDate, Vec<ResolvedOriginal<'snapshot>>>,
    calendars: Vec<VerifiedCalendarProjection<'snapshot>>,
    calendar_by_date: BTreeMap<TradingDate, Vec<usize>>,
    observed_bars: Arc<[ObservedBarRows]>,
    observed_originals: Arc<[&'snapshot [u8]]>,
    latest_overlap_only_date: Option<TradingDate>,
    price_bytes: u64,
    actions: ActionSourceRanges<'snapshot>,
}

impl<'view, 'snapshot> IndexedSessionSources<'view, 'snapshot> {
    pub(super) fn new(
        snapshots: &'view RawEvidenceSnapshots<'snapshot>,
        owner: Uuid,
        policy_sha256: &ContentHash,
        charge: ChargeSourceWork<'_>,
    ) -> Result<Self, Error> {
        let raw = collect_price_sources(snapshots, None, charge)?;
        for source in raw.bars.iter().chain(&raw.calendars) {
            // Reserve the actual body traversal before decoding or normalizing.
            charge(source.original_bytes().len() as u64, 1)?;
        }
        let calendars = project_calendar_candidates(&raw.calendars, owner, policy_sha256)?;
        let observed_bars = observe_bar_rows(&raw.bars, None, &calendars)?;
        let observed_originals = raw
            .bars
            .iter()
            .map(ResolvedOriginal::original_bytes)
            .collect();
        let latest_overlap_only_date = observed_bars
            .iter()
            .filter(|bars| bars.coverage() == BarRowCoverage::ClosedDayOverlap)
            .map(|bars| terminal_evidence_date(bars.source()))
            .max();
        let mut calendar_by_date = BTreeMap::<_, Vec<_>>::new();
        for (index, projection) in calendars.iter().enumerate() {
            for day in projection.days() {
                calendar_by_date
                    .entry(day.session_date())
                    .or_default()
                    .push(index);
            }
        }
        fn by_date(
            originals: Vec<ResolvedOriginal<'_>>,
        ) -> BTreeMap<TradingDate, Vec<ResolvedOriginal<'_>>> {
            let mut index = BTreeMap::<_, Vec<_>>::new();
            for source in originals {
                index
                    .entry(original_date(&source))
                    .or_default()
                    .push(source);
            }
            index
        }
        Ok(Self {
            snapshots,
            owner,
            policy_sha256: policy_sha256.clone(),
            bars: by_date(raw.bars),
            reference: by_date(raw.reference),
            calendars,
            calendar_by_date,
            observed_bars: observed_bars.into(),
            observed_originals,
            latest_overlap_only_date,
            price_bytes: raw.price_bytes,
            actions: ActionSourceRanges::new(snapshots)?,
        })
    }

    pub(super) fn discover(
        &self,
        request: &SessionSourceRequest,
        selection: CalendarSelection<'_>,
        mut charge: impl FnMut(u64, u64) -> Result<(), Error>,
    ) -> Result<DiscoveredSessionSources<'snapshot>, Error> {
        let start = validate_source_request(self.snapshots, request)?;
        if request.owner != self.owner || request.policy_sha256 != self.policy_sha256 {
            return Err(Error::InvalidContext);
        }
        if self
            .latest_overlap_only_date
            .is_some_and(|date| request.session_date <= date)
        {
            return Err(Error::SourceCoverageGap);
        }
        let bars = self
            .bars
            .get(&request.session_date)
            .map_or(&[][..], Vec::as_slice);
        let reference = self
            .reference
            .get(&request.session_date)
            .map_or(&[][..], Vec::as_slice);
        check_price_bytes(self.price_bytes, reference)?;
        for file in bars.iter().chain(reference) {
            charge(file.original_bytes().len() as u64, 1)?;
        }
        let selected = self
            .calendar_by_date
            .range(start..=request.session_date)
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut calendars = Vec::with_capacity(selected.len());
        for index in selected {
            let projection = &self.calendars[index];
            charge(projection.source_file().size_bytes, 1)?;
            if let CalendarSelection::Sealed { deliveries, .. } = selection
                && !deliveries
                    .get(&projection.source_file().batch_id)
                    .is_some_and(|known| known.contains(&projection.source_file()))
            {
                continue;
            }
            calendars.push(projection.clone());
        }
        assemble_session_sources(
            request,
            start,
            selection,
            SessionPriceSources {
                bars,
                reference,
                calendars,
                observed_bars: Arc::clone(&self.observed_bars),
                observed_originals: Arc::clone(&self.observed_originals),
            },
            || {
                reconstruct_indexed_actions(
                    self.snapshots,
                    &self.actions,
                    request.previous_frontier,
                    request.session_date,
                    &mut charge,
                )
            },
        )
    }
}

struct SessionPriceSources<'a, 'snapshot> {
    bars: &'a [ResolvedOriginal<'snapshot>],
    reference: &'a [ResolvedOriginal<'snapshot>],
    calendars: Vec<VerifiedCalendarProjection<'snapshot>>,
    observed_bars: Arc<[ObservedBarRows]>,
    observed_originals: Arc<[&'snapshot [u8]]>,
}

fn assemble_session_sources<'snapshot>(
    request: &SessionSourceRequest,
    start: TradingDate,
    selection: CalendarSelection<'_>,
    sources: SessionPriceSources<'_, 'snapshot>,
    reconstruct_actions: impl FnOnce() -> Result<VerifiedActionDocument<'snapshot>, Error>,
) -> Result<DiscoveredSessionSources<'snapshot>, Error> {
    let SessionPriceSources {
        bars,
        reference,
        mut calendars,
        observed_bars,
        observed_originals,
    } = sources;
    // Scope/identity ambiguity is diagnosed before the exact-eleven constructor.
    for files in [bars, reference] {
        let mut symbols = BTreeSet::new();
        for file in files {
            let symbol = file
                .evidence()
                .request
                .query
                .iter()
                .find(|(key, _)| key == "FID_INPUT_ISCD")
                .map(|(_, value)| value.as_str())
                .ok_or(Error::InvalidIdentity)?;
            if !symbols.insert(symbol) {
                return Err(Error::SourceConflict);
            }
        }
    }
    let prices = reconstruct_prices(request.session_date, bars, reference)?;
    let actions = reconstruct_actions()?;
    calendars.sort_by(|a, b| {
        a.evidence_sha256()
            .as_str()
            .cmp(b.evidence_sha256().as_str())
    });
    let projections: Vec<_> = calendars.iter().collect();
    let proof = prove_calendar_range(&projections, start, request.session_date)?;
    let by_hash: BTreeMap<_, _> = calendars
        .iter()
        .map(|projection| (projection.evidence_sha256().as_str(), projection))
        .collect();
    let mut bindings = Vec::new();
    let mut documents = Vec::new();
    for day in proof.days() {
        let expected = if day.date() == request.session_date {
            DailySessionTypeV1::Trading
        } else {
            DailySessionTypeV1::Closed
        };
        if day.session_type() != expected {
            return Err(Error::CalendarConflict);
        }
        let preferred = match selection {
            CalendarSelection::Sealed { bindings, .. } => {
                // A subsequent admission may include a new delivery of a known
                // original. Earlier documents must keep their exact witness.
                let index = bindings
                    .binary_search_by_key(&day.date(), |binding| binding.civil_date)
                    .map_err(|_| Error::InvalidEvidence)?;
                let binding = &bindings[index];
                Some(
                    day.witnesses()
                        .iter()
                        .find(|hash| {
                            by_hash.get(hash.as_str()).is_some_and(|projection| {
                                projection.source_file() == &binding.source_file
                            })
                        })
                        .ok_or(Error::InvalidEvidence)?,
                )
            }
            // Fresh discovery prefers a current requested original. The proof
            // still includes every valid surrounding witness.
            CalendarSelection::CurrentTargetPreferred if day.date() == request.session_date => {
                day.witnesses().iter().find(|hash| {
                    by_hash.get(hash.as_str()).is_some_and(|projection| {
                        let file = projection.source_file();
                        file.copied_from
                            .last()
                            .map_or(file.batch_date, |origin| origin.batch_date)
                            == request.session_date
                    })
                })
            }
            _ => None,
        };
        let selected = by_hash
            .get(preferred.unwrap_or(day.selected_evidence()).as_str())
            .ok_or(Error::InvalidEvidence)?;
        let document = selected.document(day.date())?;
        bindings.push(DailyCalendarBindingV1Wire {
            civil_date: day.date(),
            session_type: day.session_type(),
            exchange: "KRX".to_owned(),
            timezone: "Asia/Seoul".to_owned(),
            source_file: selected.source_file().clone(),
            normalized_document_sha256: document.descriptor().sha256.clone(),
            legacy_projection: None,
        });
        documents.push(SessionSourceDocument {
            descriptor: document.descriptor().clone(),
            bytes: document.bytes().to_vec(),
        });
    }
    for document in [prices.bars(), prices.reference(), actions.document()] {
        documents.push(SessionSourceDocument {
            descriptor: document.descriptor().clone(),
            bytes: document.bytes().to_vec(),
        });
    }
    documents.sort_by_key(|doc| (doc.descriptor.session_date, kind_label(doc.descriptor.kind)));
    let mut session = DailySessionEvidenceV1Wire {
        session_date: request.session_date,
        previous_frontier: request.previous_frontier,
        instruments: KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|s| {
                InstrumentId::from_str(&format!("{s}.KRX")).map_err(|_| Error::InvalidIdentity)
            })
            .collect::<Result<_, _>>()?,
        bars: prices.bar_sources().to_vec(),
        reference: prices.reference_sources().to_vec(),
        calendar: bindings,
        actions: actions.witnesses().to_vec(),
        normalized_documents: documents.iter().map(|doc| doc.descriptor.clone()).collect(),
    };
    session.instruments.sort_by_key(InstrumentId::as_str);
    // A refreshed discovery is compared structurally with the sealed manifest.
    // Use the same wire order here: reconstruction's action enum order is not
    // the canonical literal order used when the first admission was sealed.
    canonicalize_session(&mut session).map_err(|_| Error::InvalidEvidence)?;
    let evidence_preimage = daily_price_evidence_preimage_v1(std::slice::from_ref(&session))
        .map_err(|_| Error::InvalidEvidence)?;
    let files: Vec<_> = session
        .bars
        .iter()
        .chain(&session.reference)
        .chain(
            calendars
                .iter()
                .map(VerifiedCalendarProjection::source_file),
        )
        .chain(session.actions.iter().flat_map(|chain| &chain.pages))
        .cloned()
        .collect();
    let observation = canonical_source_observation(&files)?;
    Ok(DiscoveredSessionSources {
        owner: request.owner,
        policy_sha256: request.policy_sha256.clone(),
        session,
        evidence_preimage,
        documents,
        observation,
        calendars,
        observed_bars,
        prices,
        actions,
        observed_originals,
    })
}

fn inspect_bar_rows(
    original: &ResolvedOriginal<'_>,
    session: Option<TradingDate>,
    calendar_candidates: &[VerifiedCalendarProjection<'_>],
    proven_closed_days: &mut BTreeSet<TradingDate>,
) -> Result<ObservedBarRows, Error> {
    let evidence = original.evidence();
    let requested = original_date(original);
    let symbol = evidence
        .request
        .query
        .iter()
        .find(|(key, _)| key == "FID_INPUT_ISCD")
        .map(|(_, value)| value.as_str())
        .ok_or(Error::InvalidIdentity)?;
    if !KR_ETF_CORE_SYMBOLS.contains(&symbol) {
        return Err(Error::InvalidIdentity);
    }
    validate_file_request_for_symbol(evidence, Kind::Bars, symbol, requested)
        .map_err(|_| Error::InvalidEvidence)?;
    let document = parse_provider_json(original.original_bytes())?;
    if document.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(Error::ProviderResponse);
    }
    if !document.get("output1").is_some_and(Value::is_object) {
        return Err(Error::InvalidBars);
    }
    let rows = document
        .get("output2")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidBars)?;
    let instrument = format!("{symbol}.KRX");
    let mut by_date = BTreeMap::new();
    for row in rows {
        let row = row.as_object().ok_or(Error::InvalidBars)?;
        let text = row
            .get("stck_bsop_date")
            .and_then(Value::as_str)
            .ok_or(Error::InvalidBars)?;
        let date = parse_kis_date(
            ResponseKind::Bars,
            &evidence.file_name,
            "stck_bsop_date",
            text,
        )
        .map_err(|_| Error::InvalidBars)?;
        if !(1..=9999).contains(&date.as_naive_date().year()) {
            return Err(Error::InvalidBars);
        }
        let canonical = canonical_bar(&evidence.file_name, row, &instrument, date)
            .map_err(|_| Error::InvalidBars)?;
        if by_date.insert(date, canonical).is_some() {
            return Err(Error::SourceConflict);
        }
    }
    let coverage = if by_date.contains_key(&requested) {
        BarRowCoverage::RequestedDate
    } else {
        // A closed request cannot supply the session's own bar. Any earlier rows
        // still have to agree with the protected prefix in exact reconstruction.
        if session.is_some_and(|session| requested >= session) {
            return Err(Error::SourceCoverageGap);
        }
        if !proven_closed_days.contains(&requested) {
            let covering: Vec<_> = calendar_candidates
                .iter()
                .filter(|projection| {
                    projection
                        .days()
                        .iter()
                        .any(|day| day.session_date() == requested)
                })
                .collect();
            if covering.is_empty()
                || prove_calendar_range(&covering, requested, requested)?.days()[0].session_type()
                    != DailySessionTypeV1::Closed
            {
                return Err(Error::SourceCoverageGap);
            }
            proven_closed_days.insert(requested);
        }
        BarRowCoverage::ClosedDayOverlap
    };
    coverage.validate_dates(requested, by_date.keys().copied())?;
    Ok(ObservedBarRows {
        source: evidence.clone(),
        rows: by_date.into_values().collect(),
        coverage,
    })
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Bars => "bars",
        Kind::Calendar => "calendar",
        Kind::CorporateActions => "corporate_actions",
        Kind::Reference => "reference",
    }
}

fn is_etf_price_source(source: &ResolvedOriginal<'_>) -> Result<bool, Error> {
    let symbol = source
        .evidence()
        .request
        .query
        .iter()
        .find(|(key, _)| key == "FID_INPUT_ISCD")
        .map(|(_, value)| value.as_str())
        .ok_or(Error::InvalidIdentity)?;
    if symbol.len() != 6 || !symbol.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::InvalidIdentity);
    }
    Ok(KR_ETF_CORE_SYMBOLS.contains(&symbol))
}
