//! Complete read-only session discovery over approved captured scopes.
//! Source reconstruction is not protected-prefix verification or admission.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

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
    VerifiedActionDocument, VerifiedPriceDocuments, reconstruct_actions, reconstruct_prices,
};
use super::verify::{
    DailyVerificationError as Error, RawEvidenceSnapshots, ResolvedOriginal, parse_provider_json,
    resolve_original,
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
}

impl ObservedBarRows {
    pub fn source(&self) -> &DailyFileEvidenceV1Wire {
        &self.source
    }
    pub fn rows(&self) -> &[Value] {
        &self.rows
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
    observed_bars: Vec<ObservedBarRows>,
    prices: VerifiedPriceDocuments<'snapshot>,
    actions: VerifiedActionDocument<'snapshot>,
    other_originals: Vec<&'snapshot [u8]>,
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
        self.other_originals.len()
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
    source
        .evidence()
        .copied_from
        .last()
        .map_or(source.evidence().batch_date, |hop| hop.batch_date)
}

/// Enumerates the snapshots itself. Duplicate target bar/reference deliveries,
/// even numerically equal ones, fail closed: D4 v1 cannot represent multiple
/// files for one symbol, and no competing source is silently discarded.
pub fn discover_session_sources<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    request: &SessionSourceRequest,
) -> Result<DiscoveredSessionSources<'snapshot>, Error> {
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
    let limits = fixed_limits();
    let mut bars = Vec::new();
    let mut reference = Vec::new();
    let mut calendar_originals = Vec::new();
    let mut observed_bars = Vec::new();
    let mut other_originals = Vec::new();
    let mut source_count = 0u64;
    let mut parse_bytes = 0u64;
    for &snapshot in snapshots.scopes() {
        for batch in snapshot.batches() {
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
                let source = resolve_original(snapshots, &file_reference(file))?;
                if matches!(kind, Kind::Bars | Kind::Reference) && !is_etf_price_source(&source)? {
                    continue;
                }
                // A valid unrelated reference delivery cannot affect this date.
                if kind == Kind::Reference && original_date(&source) != request.session_date {
                    continue;
                }
                parse_bytes = parse_bytes
                    .checked_add(source.original_bytes().len() as u64)
                    .ok_or(Error::LimitExceeded)?;
                if parse_bytes > limits.max_bars_bytes {
                    return Err(Error::LimitExceeded);
                }
                match kind {
                    Kind::Calendar => {
                        calendar_originals.push(source);
                    }
                    Kind::Bars => {
                        observed_bars.push(inspect_bar_rows(&source)?);
                        if original_date(&source) == request.session_date {
                            bars.push(source);
                        } else {
                            other_originals.push(source.original_bytes());
                        }
                    }
                    Kind::Reference => reference.push(source),
                    _ => return Err(Error::InvalidEvidence),
                }
            }
        }
    }
    let mut calendars: Vec<_> =
        project_calendar_candidates(&calendar_originals, request.owner, &request.policy_sha256)?
            .into_iter()
            .filter(|projection| {
                projection.days().iter().any(|day| {
                    day.session_date() >= start && day.session_date() <= request.session_date
                })
            })
            .collect();
    // Scope/identity ambiguity is diagnosed before the exact-eleven constructor.
    for files in [&bars, &reference] {
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
    let prices = reconstruct_prices(request.session_date, &bars, &reference)?;
    let actions = reconstruct_actions(snapshots, request.previous_frontier, request.session_date)?;
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
        let selected = by_hash
            .get(day.selected_evidence().as_str())
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
    observed_bars.sort_by(|a, b| {
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
        other_originals,
    })
}

fn inspect_bar_rows(original: &ResolvedOriginal<'_>) -> Result<ObservedBarRows, Error> {
    let evidence = original.evidence();
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
    validate_file_request_for_symbol(evidence, Kind::Bars, symbol, original_date(original))
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
    if !by_date.contains_key(&original_date(original)) {
        return Err(Error::SourceCoverageGap);
    }
    Ok(ObservedBarRows {
        source: evidence.clone(),
        rows: by_date.into_values().collect(),
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
