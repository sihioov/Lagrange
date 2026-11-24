//! Read-only provider reconstruction over resolved, committed originals.
//! These documents are evidence, not an admitted artifact or a complete frontier.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};

use chrono::Datelike;
use domain::{ContentHash, FixedPoint, TradingDate};
use serde_json::{Map, Value, json};

use super::contract::{
    canonical_cash_rows, canonicalize_action, fixed_limits, validate_action_chain,
    validate_file_request_for_symbol,
};
use super::source::{CommittedRawBatch, CommittedRawFile, CommittedRawSnapshot};
use super::verify::{
    ChargeSourceWork, DailyVerificationError as Error, RawEvidenceSnapshots, ResolvedOriginal,
    canonical_json, parse_provider_json, resolve_original_charged,
};
use super::{
    DailyActionChainV1Wire, DailyActionClassV1 as Class, DailyActionScopeV1 as Scope,
    DailyCashDividendRowV1Wire, daily_cash_dividend_rows_preimage_v1,
};
use super::{
    DailyEvidenceKindV1 as Kind, DailyFileEvidenceV1Wire, DailyFileKeyV1Wire,
    DailyNormalizedDocumentV1Wire, DailyNormalizedSourceCommitmentV1Wire,
    daily_normalized_source_preimage_v1,
};
use crate::contract::{ResponseKind, SourceFileReference};
use crate::normalize::{canonical_bar, parse_kis_date, validate_kis_action_row_fields};
use crate::providers::kis::{KIS_ACTION_SPECS, KR_ETF_CORE_SYMBOLS, ksd_final_page_truncated};

const NORMALIZER_VERSION: &str = "daily-raw-reconstruction-v1";

pub struct ReconstructedDocument {
    bytes: Vec<u8>,
    descriptor: DailyNormalizedDocumentV1Wire,
    rows: Vec<Value>,
}

impl ReconstructedDocument {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn descriptor(&self) -> &DailyNormalizedDocumentV1Wire {
        &self.descriptor
    }
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }
}

/// The retained borrows keep the captured originals and their shared locks alive.
pub struct VerifiedPriceDocuments<'snapshot> {
    bars: ReconstructedDocument,
    reference: ReconstructedDocument,
    bar_sources: Vec<DailyFileEvidenceV1Wire>,
    reference_sources: Vec<DailyFileEvidenceV1Wire>,
    _originals: Vec<&'snapshot [u8]>,
}

impl VerifiedPriceDocuments<'_> {
    pub fn bars(&self) -> &ReconstructedDocument {
        &self.bars
    }
    pub fn reference(&self) -> &ReconstructedDocument {
        &self.reference
    }
    pub fn bar_sources(&self) -> &[DailyFileEvidenceV1Wire] {
        &self.bar_sources
    }
    pub fn reference_sources(&self) -> &[DailyFileEvidenceV1Wire] {
        &self.reference_sources
    }
}

pub fn reconstruct_prices<'snapshot>(
    session: TradingDate,
    bars: &[ResolvedOriginal<'snapshot>],
    reference: &[ResolvedOriginal<'snapshot>],
) -> Result<VerifiedPriceDocuments<'snapshot>, Error> {
    if bars.len() != KR_ETF_CORE_SYMBOLS.len() || reference.len() != KR_ETF_CORE_SYMBOLS.len() {
        return Err(Error::SourceCoverageGap);
    }
    let (bar_rows, bar_sources) = reconstruct_price_kind(session, bars, Kind::Bars)?;
    let (reference_rows, reference_sources) =
        reconstruct_price_kind(session, reference, Kind::Reference)?;
    let bars_document = make_document(Kind::Bars, session, &bar_sources, bar_rows)?;
    let reference_document =
        make_document(Kind::Reference, session, &reference_sources, reference_rows)?;
    Ok(VerifiedPriceDocuments {
        bars: bars_document,
        reference: reference_document,
        bar_sources,
        reference_sources,
        _originals: bars
            .iter()
            .chain(reference)
            .map(ResolvedOriginal::original_bytes)
            .collect(),
    })
}

fn reconstruct_price_kind(
    session: TradingDate,
    files: &[ResolvedOriginal<'_>],
    kind: Kind,
) -> Result<(Vec<Value>, Vec<DailyFileEvidenceV1Wire>), Error> {
    let mut by_symbol = BTreeMap::new();
    for original in files {
        let evidence = original.evidence();
        let symbol = query_value(evidence, "FID_INPUT_ISCD").ok_or(Error::InvalidEvidence)?;
        if !KR_ETF_CORE_SYMBOLS.contains(&symbol) {
            return Err(Error::InvalidIdentity);
        }
        validate_file_request_for_symbol(evidence, kind, symbol, session)
            .map_err(|_| Error::InvalidEvidence)?;
        let document = provider_object(original.original_bytes())?;
        let instrument = format!("{symbol}.KRX");
        let row = match kind {
            Kind::Bars => reconstruct_bar(&document, &evidence.file_name, &instrument, session)?,
            Kind::Reference => {
                validate_reference_symbol(&document, symbol)?;
                json!({"instrument": instrument, "provider_symbol": symbol})
            }
            _ => return Err(Error::InvalidEvidence),
        };
        if by_symbol
            .insert(symbol.to_owned(), (row, evidence.clone()))
            .is_some()
        {
            return Err(Error::SourceConflict);
        }
    }
    if by_symbol.len() != KR_ETF_CORE_SYMBOLS.len() {
        return Err(Error::SourceCoverageGap);
    }
    let (rows, mut evidence): (Vec<_>, Vec<_>) = by_symbol.into_values().unzip();
    evidence.sort_by_key(file_identity);
    Ok((rows, evidence))
}

pub(crate) fn validate_recovery_price(
    kind: ResponseKind,
    bytes: &[u8],
    file_name: &str,
    symbol: &str,
    session: TradingDate,
) -> Result<(), Error> {
    let document = provider_object(bytes)?;
    match kind {
        ResponseKind::Bars => {
            let reported_symbol = document
                .get("output1")
                .and_then(|output| output.get("stck_shrn_iscd"));
            if reported_symbol.is_some_and(|actual| actual.as_str() != Some(symbol)) {
                return Err(Error::InvalidIdentity);
            }
            reconstruct_bar(&document, file_name, &format!("{symbol}.KRX"), session)?;
            Ok(())
        }
        ResponseKind::Reference => validate_reference_symbol(&document, symbol),
        _ => Err(Error::InvalidEvidence),
    }
}

fn validate_reference_symbol(document: &Map<String, Value>, symbol: &str) -> Result<(), Error> {
    if document
        .get("output")
        .and_then(|output| output.get("stck_shrn_iscd"))
        .and_then(Value::as_str)
        != Some(symbol)
    {
        return Err(Error::InvalidReference);
    }
    Ok(())
}

fn reconstruct_bar(
    document: &Map<String, Value>,
    file_name: &str,
    instrument: &str,
    session: TradingDate,
) -> Result<Value, Error> {
    if !document.get("output1").is_some_and(Value::is_object) {
        return Err(Error::InvalidBars);
    }
    let rows = document
        .get("output2")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidBars)?;
    let mut dates = BTreeSet::new();
    let mut target = None;
    for row in rows {
        let row = row.as_object().ok_or(Error::InvalidBars)?;
        let date_text = row
            .get("stck_bsop_date")
            .and_then(Value::as_str)
            .ok_or(Error::InvalidBars)?;
        let date = parse_kis_date(ResponseKind::Bars, file_name, "stck_bsop_date", date_text)
            .map_err(|_| Error::InvalidBars)?;
        if !(1..=9999).contains(&date.as_naive_date().year()) {
            return Err(Error::InvalidBars);
        }
        let canonical =
            canonical_bar(file_name, row, instrument, date).map_err(|_| Error::InvalidBars)?;
        if !dates.insert(date) {
            return Err(Error::SourceConflict);
        }
        if date == session {
            target = Some(canonical);
        }
    }
    target.ok_or(Error::SourceCoverageGap)
}

fn provider_object(bytes: &[u8]) -> Result<Map<String, Value>, Error> {
    let Value::Object(document) = parse_provider_json(bytes)? else {
        return Err(Error::InvalidProviderJson);
    };
    if document.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(Error::ProviderResponse);
    }
    Ok(document)
}

fn query_value<'a>(file: &'a DailyFileEvidenceV1Wire, key: &str) -> Option<&'a str> {
    file.request
        .query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn file_identity(file: &DailyFileEvidenceV1Wire) -> (String, String, String, String) {
    (
        file.provider.clone(),
        file.market.clone(),
        file.batch_id.to_string(),
        file.file_name.clone(),
    )
}

fn file_key(file: &DailyFileEvidenceV1Wire) -> DailyFileKeyV1Wire {
    DailyFileKeyV1Wire {
        provider: file.provider.clone(),
        market: file.market.clone(),
        batch_id: file.batch_id,
        file_name: file.file_name.clone(),
        wire_sha256: file.wire_sha256.clone(),
    }
}

fn make_document(
    kind: Kind,
    session: TradingDate,
    files: &[DailyFileEvidenceV1Wire],
    rows: Vec<Value>,
) -> Result<ReconstructedDocument, Error> {
    let mut source_files: Vec<_> = files.iter().map(file_key).collect();
    source_files.sort_by_key(|key| {
        (
            key.provider.clone(),
            key.market.clone(),
            key.batch_id.to_string(),
            key.file_name.clone(),
            key.wire_sha256.to_string(),
        )
    });
    let source = DailyNormalizedSourceCommitmentV1Wire {
        schema: "owner-beta-daily-normalized-source-v1".to_owned(),
        kind,
        session_date: session,
        source_files,
    };
    let source_hash = ContentHash::from_bytes(
        &daily_normalized_source_preimage_v1(&source).map_err(|_| Error::InvalidEvidence)?,
    );
    let document = json!({"schema": "owner-beta-daily-reconstructed-document-v1", "normalizer_version": NORMALIZER_VERSION, "kind": kind, "session_date": session, "source_evidence_sha256": source_hash, "rows": rows});
    let bytes = canonical_json(&document)?;
    let descriptor = DailyNormalizedDocumentV1Wire {
        kind,
        session_date: session,
        normalizer_version: NORMALIZER_VERSION.to_owned(),
        sha256: ContentHash::from_bytes(&bytes),
        size_bytes: u64::try_from(bytes.len()).map_err(|_| Error::LimitExceeded)?,
        row_count: u64::try_from(rows.len()).map_err(|_| Error::LimitExceeded)?,
        source_files: source.source_files,
        source_evidence_sha256: source_hash,
    };
    Ok(ReconstructedDocument {
        bytes,
        descriptor,
        rows,
    })
}

const ACTION_CLASSES: [Class; 7] = [
    Class::PaidinSubscription,
    Class::PaidinRecord,
    Class::BonusIssue,
    Class::Dividend,
    Class::MergerSplit,
    Class::ReverseSplit,
    Class::CapitalDecrease,
];

/// A cell selects a complete witness chain; it never selects individual pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSelection {
    pub civil_date: TradingDate,
    pub logical_class: Class,
    pub symbol: String,
    pub witness_index: usize,
}

pub struct VerifiedActionDocument<'snapshot> {
    document: ReconstructedDocument,
    witnesses: Vec<DailyActionChainV1Wire>,
    selections: Vec<ActionSelection>,
    _originals: Vec<&'snapshot [u8]>,
}

impl VerifiedActionDocument<'_> {
    pub fn document(&self) -> &ReconstructedDocument {
        &self.document
    }
    pub fn witnesses(&self) -> &[DailyActionChainV1Wire] {
        &self.witnesses
    }
    pub fn selections(&self) -> &[ActionSelection] {
        &self.selections
    }
}

type Cell = (TradingDate, String);
type ChainOrder = (TradingDate, TradingDate, &'static str, String, String);

struct ActionChain {
    wire: DailyActionChainV1Wire,
    rows: BTreeMap<Cell, Vec<Vec<u8>>>,
}

pub(super) struct ActionPage<'snapshot> {
    ordinal: usize,
    source: ResolvedOriginal<'snapshot>,
}

/// Verifies all complete overlapping committed bundles and every intervening civil
/// day. This performs no collection, normalization writes, sealing or admission.
/// A chain that is truncated (see `ksd_final_page_truncated`) is excluded: it
/// contributes no absence, no rows and no comparison, and never covers a cell.
pub fn reconstruct_actions<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    previous_frontier: TradingDate,
    session: TradingDate,
) -> Result<VerifiedActionDocument<'snapshot>, Error> {
    reconstruct_actions_inner(snapshots, previous_frontier, session, None)
}

pub(super) fn reconstruct_indexed_actions<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    ranges: &ActionSourceRanges<'snapshot>,
    previous_frontier: TradingDate,
    session: TradingDate,
    charge: ChargeSourceWork<'_>,
) -> Result<VerifiedActionDocument<'snapshot>, Error> {
    reconstruct_actions_inner(
        snapshots,
        previous_frontier,
        session,
        Some((ranges, charge)),
    )
}

fn reconstruct_actions_inner<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    previous_frontier: TradingDate,
    session: TradingDate,
    indexed: Option<(&ActionSourceRanges<'snapshot>, ChargeSourceWork<'_>)>,
) -> Result<VerifiedActionDocument<'snapshot>, Error> {
    let limits = fixed_limits();
    let dates = required_dates(previous_frontier, session)?;
    let mut chains = Vec::new();
    let mut originals = Vec::new();
    let mut comparison_bytes = 0u64;
    let mut collect = |wire, pages: &[ActionPage<'snapshot>], charge: ChargeSourceWork<'_>| {
        for page in pages {
            charge(page.source.original_bytes().len() as u64, 1)?;
        }
        originals.extend(pages.iter().map(|page| page.source.original_bytes()));
        chains.push(reconstruct_chain(
            wire,
            pages,
            previous_frontier,
            session,
            &mut comparison_bytes,
        )?);
        Ok(())
    };
    if let Some((ranges, charge)) = indexed {
        visit_action_bundles(
            snapshots,
            ranges.bundles.iter().map(Ok),
            previous_frontier,
            session,
            charge,
            &mut collect,
        )?;
    } else {
        visit_complete_action_chains(
            snapshots,
            snapshots.kis_snapshot()?,
            previous_frontier,
            session,
            &mut |_, _| Ok(()),
            collect,
        )?;
    }
    // Stable witness order also makes public selection indices deterministic.
    chains.sort_by_key(|chain| {
        (
            chain.wire.logical_class,
            chain.wire.symbol.clone(),
            chain_order(&chain.wire),
        )
    });
    let mut selections = Vec::new();
    let mut cash = Vec::new();
    for day in dates {
        for class in ACTION_CLASSES {
            let mut symbols = KR_ETF_CORE_SYMBOLS;
            symbols.sort_unstable();
            for symbol in symbols {
                let cell = (day, symbol.to_owned());
                let mut selected: Option<usize> = None;
                for (index, chain) in chains.iter().enumerate() {
                    let wire = &chain.wire;
                    if wire.logical_class != class
                        || wire.range_start > day
                        || wire.range_end < day
                        || (wire.scope == Scope::Etf11Symbol
                            && wire.symbol.as_deref() != Some(symbol))
                    {
                        continue;
                    }
                    if let Some(prior) = selected {
                        if chain.rows.get(&cell).map(Vec::as_slice).unwrap_or(&[])
                            != chains[prior]
                                .rows
                                .get(&cell)
                                .map(Vec::as_slice)
                                .unwrap_or(&[])
                        {
                            return Err(Error::SourceConflict);
                        }
                        if chain_order(wire) < chain_order(&chains[prior].wire) {
                            selected = Some(index);
                        }
                    } else {
                        selected = Some(index);
                    }
                }
                let selected = selected.ok_or(Error::SourceCoverageGap)?;
                selections.push(ActionSelection {
                    civil_date: day,
                    logical_class: class,
                    symbol: symbol.to_owned(),
                    witness_index: selected,
                });
                if class == Class::Dividend {
                    for row in &chains[selected].wire.cash_dividends {
                        if row.symbol == symbol && row.record_date == day {
                            if cash.len() as u64 >= limits.max_cash_rows_per_chain {
                                return Err(Error::LimitExceeded);
                            }
                            cash.push(row.clone());
                        }
                    }
                }
            }
        }
    }
    let cash = canonical_cash_rows(&cash).map_err(|_| Error::InvalidActions)?;
    let rows = cash
        .iter()
        .map(|row| serde_json::to_value(row).map_err(|_| Error::InvalidActions))
        .collect::<Result<Vec<_>, _>>()?;
    let sources: Vec<_> = chains
        .iter()
        .flat_map(|chain| chain.wire.pages.iter().cloned())
        .collect();
    let document = make_document(Kind::CorporateActions, session, &sources, rows)?;
    Ok(VerifiedActionDocument {
        document,
        witnesses: chains.into_iter().map(|chain| chain.wire).collect(),
        selections,
        _originals: originals,
    })
}

/// The request range and scope of each eligible KSD bundle are read once.
/// Replays still resolve and verify the real pages of each overlapping bundle.
pub(super) struct ActionSourceRanges<'snapshot> {
    bundles: Vec<ActionBundle<'snapshot>>,
}

impl<'snapshot> ActionSourceRanges<'snapshot> {
    pub(super) fn new(snapshots: &RawEvidenceSnapshots<'snapshot>) -> Result<Self, Error> {
        let bundles = snapshots
            .batches(snapshots.kis_snapshot()?)?
            .filter_map(|batch| action_bundle(snapshots, batch).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { bundles })
    }
}

struct ActionBundle<'snapshot> {
    batch: &'snapshot CommittedRawBatch,
    start: TradingDate,
    end: TradingDate,
    scope: Scope,
    files: Vec<CommittedRawFile<'snapshot>>,
}

fn action_bundle<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    batch: &'snapshot CommittedRawBatch,
) -> Result<Option<ActionBundle<'snapshot>>, Error> {
    if !snapshots.matches_entitlement(batch) {
        return Ok(None);
    }
    let files: Vec<_> = batch
        .files()
        .filter(|file| file.kind() == ResponseKind::CorporateActions)
        .collect();
    let mut files_iter = files.iter();
    let Some(first) = files_iter.next() else {
        return Ok(None);
    };
    // Request range, not the manifest partition, determines discovery. An
    // out-of-window bundle still has to prove a single range and scope.
    let identity = range_and_scope(&first.entry().request.query)?;
    for file in files_iter {
        if range_and_scope(&file.entry().request.query)? != identity {
            return Err(Error::InvalidActions);
        }
    }
    let (start, end, scope) = identity;
    Ok(Some(ActionBundle {
        batch,
        start,
        end,
        scope,
        files,
    }))
}

/// Read the page identities and terminal status without normalizing action rows.
/// A refresh uses the same completeness rule before attributing added evidence.
pub(super) fn visit_complete_action_chains<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    snapshot: &'snapshot CommittedRawSnapshot,
    previous_frontier: TradingDate,
    session: TradingDate,
    charge: ChargeSourceWork<'_>,
    visit: impl FnMut(
        DailyActionChainV1Wire,
        &[ActionPage<'snapshot>],
        ChargeSourceWork<'_>,
    ) -> Result<(), Error>,
) -> Result<(), Error> {
    let bundles = snapshots
        .batches(snapshot)?
        .filter_map(|batch| action_bundle(snapshots, batch).transpose());
    visit_action_bundles(
        snapshots,
        bundles,
        previous_frontier,
        session,
        charge,
        visit,
    )
}

fn visit_action_bundles<'snapshot, B: Borrow<ActionBundle<'snapshot>>>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    bundles: impl Iterator<Item = Result<B, Error>>,
    previous_frontier: TradingDate,
    session: TradingDate,
    charge: ChargeSourceWork<'_>,
    mut visit: impl FnMut(
        DailyActionChainV1Wire,
        &[ActionPage<'snapshot>],
        ChargeSourceWork<'_>,
    ) -> Result<(), Error>,
) -> Result<(), Error> {
    let limits = fixed_limits();
    let mut evidence_bytes = 0u64;
    let mut file_count = 0u64;
    let mut chain_count = 0u64;
    for bundle in bundles {
        let bundle = bundle?;
        let ActionBundle {
            batch,
            start,
            end,
            scope,
            files,
        } = bundle.borrow();
        let (start, end, scope) = (*start, *end, *scope);
        if end <= previous_frontier || start > session {
            continue;
        }
        let mut groups: BTreeMap<(Class, Option<String>), Vec<ActionPage<'snapshot>>> =
            BTreeMap::new();
        for file in files {
            if file_count >= limits.max_file_evidence_per_session {
                return Err(Error::LimitExceeded);
            }
            let reference = SourceFileReference {
                provider: batch.entry().provider.clone(),
                market: batch.entry().market.clone(),
                batch_id: batch.batch_id(),
                file_name: file.file_name().to_owned(),
                content_hash: file.content_hash().clone(),
                retrieved_at: batch.retrieved_at(),
            };
            let source = resolve_original_charged(snapshots, &reference, charge)?;
            add_budget(
                &mut evidence_bytes,
                source.canonical_evidence_bytes().len(),
                limits.max_manifest_bytes,
            )?;
            let (class, symbol, ordinal) = action_page_identity(source.evidence())?;
            file_count += 1;
            groups
                .entry((class, symbol))
                .or_default()
                .push(ActionPage { ordinal, source });
        }
        let symbols: Vec<Option<String>> = match scope {
            Scope::WholeMarket => vec![None],
            Scope::Etf11Symbol => KR_ETF_CORE_SYMBOLS
                .iter()
                .map(|symbol| Some((*symbol).to_owned()))
                .collect(),
        };
        if groups.len() != ACTION_CLASSES.len() * symbols.len() {
            return Err(Error::SourceCoverageGap);
        }
        for class in ACTION_CLASSES {
            for symbol in &symbols {
                let mut pages = groups
                    .remove(&(class, symbol.clone()))
                    .ok_or(Error::SourceCoverageGap)?;
                if chain_count >= limits.max_action_chains_per_session {
                    return Err(Error::LimitExceeded);
                }
                pages.sort_by_key(|page| page.ordinal);
                if pages.len() as u64 > limits.max_pages_per_action_chain
                    || pages
                        .iter()
                        .enumerate()
                        .any(|(index, page)| page.ordinal != index + 1)
                {
                    return Err(Error::InvalidActions);
                }
                let wire = DailyActionChainV1Wire {
                    logical_class: class,
                    scope,
                    symbol: symbol.clone(),
                    range_start: start,
                    range_end: end,
                    pages: pages
                        .iter()
                        .map(|page| page.source.evidence().clone())
                        .collect(),
                    cash_dividends: Vec::new(),
                    cash_dividend_rows_sha256: ContentHash::from_bytes(b"[]"),
                };
                // Validate request/continuation/original-bundle identity before
                // interpreting any provider body as negative or positive evidence.
                validate_action_chain(&wire, previous_frontier, session)
                    .map_err(|_| Error::InvalidActions)?;
                // A KSD response holds at most 100 rows. A chain whose final
                // page carries marker `F`, or is full (including legacy pages
                // with no recorded marker), may have been cut off by the
                // provider, so it is incomplete evidence: it proves neither
                // absence nor presence. Exclude it entirely, before any row is
                // interpreted; its cells then fall to `SourceCoverageGap`
                // unless a complete chain covers them. The structural checks
                // above still apply to it, and the other classes of the same
                // bundle remain independent, usable chains.
                let last = pages.last().ok_or(Error::InvalidActions)?;
                charge(last.source.original_bytes().len() as u64, 1)?;
                let final_document = provider_object(last.source.original_bytes())?;
                if ksd_final_page_truncated(
                    last.source.evidence().response_continuation.as_deref(),
                    final_document.get("output1"),
                ) {
                    continue;
                }
                visit(wire, &pages, charge)?;
                chain_count += 1;
            }
        }
    }
    Ok(())
}

fn required_dates(previous: TradingDate, session: TradingDate) -> Result<Vec<TradingDate>, Error> {
    if previous >= session
        || !(1..=9999).contains(&previous.as_naive_date().year())
        || !(1..=9999).contains(&session.as_naive_date().year())
    {
        return Err(Error::InvalidContext);
    }
    let mut dates = Vec::new();
    let mut day = previous;
    while day < session {
        if dates.len() as u64 >= fixed_limits().max_civil_dates_per_addition {
            return Err(Error::LimitExceeded);
        }
        day = day.checked_add_days(1).map_err(|_| Error::InvalidContext)?;
        dates.push(day);
    }
    Ok(dates)
}

pub(super) fn range_and_scope(
    query: &[(String, String)],
) -> Result<(TradingDate, TradingDate, Scope), Error> {
    let get = |key: &str| -> Result<&str, Error> {
        let mut values = query.iter().filter(|(name, _)| name == key);
        let value = values.next().ok_or(Error::InvalidActions)?;
        if values.next().is_some() {
            return Err(Error::InvalidActions);
        }
        Ok(value.1.as_str())
    };
    let parse = |text: &str| -> Result<TradingDate, Error> {
        if text.len() != 8 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidActions);
        }
        let day = parse_kis_date(ResponseKind::CorporateActions, "<committed>", "range", text)
            .map_err(|_| Error::InvalidActions)?;
        if !(1..=9999).contains(&day.as_naive_date().year()) {
            return Err(Error::InvalidActions);
        }
        Ok(day)
    };
    let start = parse(get("F_DT")?)?;
    let end = parse(get("T_DT")?)?;
    if start > end {
        return Err(Error::InvalidActions);
    }
    let symbol = get("SHT_CD")?;
    let scope = if symbol.is_empty() {
        Scope::WholeMarket
    } else if KR_ETF_CORE_SYMBOLS.contains(&symbol) {
        Scope::Etf11Symbol
    } else {
        return Err(Error::InvalidActions);
    };
    Ok((start, end, scope))
}

pub(super) fn action_page_identity(
    file: &DailyFileEvidenceV1Wire,
) -> Result<(Class, Option<String>, usize), Error> {
    let symbol = query_value(file, "SHT_CD").ok_or(Error::InvalidActions)?;
    let symbol = (!symbol.is_empty()).then(|| symbol.to_owned());
    // The existing provider encodes actual page ordinals in these exact names.
    // Copies can rename their files; use the resolved terminal original's name.
    let name = file
        .copied_from
        .last()
        .map_or(file.file_name.as_str(), |origin| origin.file_name.as_str());
    for (class, spec) in ACTION_CLASSES.into_iter().zip(KIS_ACTION_SPECS) {
        if file.request.endpoint != spec.path
            || spec
                .extra
                .iter()
                .any(|(key, value)| query_value(file, key) != Some(*value))
        {
            continue;
        }
        let prefix = match &symbol {
            Some(symbol) => format!("{}-{symbol}-page-", spec.label),
            None => format!("{}-page-", spec.label),
        };
        let Some(page) = name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if page.len() != 2 || !page.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::InvalidActions);
        }
        let ordinal = page.parse::<usize>().map_err(|_| Error::InvalidActions)?;
        if ordinal == 0 || ordinal as u64 > fixed_limits().max_pages_per_action_chain {
            return Err(Error::InvalidActions);
        }
        return Ok((class, symbol, ordinal));
    }
    Err(Error::InvalidActions)
}

fn chain_order(chain: &DailyActionChainV1Wire) -> ChainOrder {
    let first = &chain.pages[0];
    let original_hash = first
        .copied_from
        .last()
        .map_or(&first.manifest_line_sha256, |origin| {
            &origin.manifest_line_sha256
        });
    let scope = match chain.scope {
        Scope::WholeMarket => "whole_market",
        Scope::Etf11Symbol => "etf11_symbol",
    };
    (
        chain.range_start,
        chain.range_end,
        scope,
        original_hash.to_string(),
        first.batch_id.to_string(),
    )
}

fn add_budget(total: &mut u64, next: usize, maximum: u64) -> Result<(), Error> {
    *total = total
        .checked_add(u64::try_from(next).map_err(|_| Error::LimitExceeded)?)
        .ok_or(Error::LimitExceeded)?;
    if *total > maximum {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}

fn reconstruct_chain(
    mut wire: DailyActionChainV1Wire,
    pages: &[ActionPage<'_>],
    previous_frontier: TradingDate,
    session: TradingDate,
    comparison_bytes: &mut u64,
) -> Result<ActionChain, Error> {
    let mut cells: BTreeMap<Cell, Vec<Vec<u8>>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for page in pages {
        let file = page.source.evidence();
        let document = provider_object(page.source.original_bytes())?;
        let rows: &[Value] = match document.get("output1") {
            Some(Value::Array(rows)) => rows,
            Some(value @ Value::Object(_)) => std::slice::from_ref(value),
            _ => return Err(Error::InvalidActions),
        };
        for value in rows {
            let row = value.as_object().ok_or(Error::InvalidActions)?;
            let parsed =
                validate_kis_action_row_fields(&file.request.endpoint, &file.file_name, row)
                    .map_err(|_| Error::InvalidActions)?;
            // KIS GB1=1 selects the subscription date, while GB1=2 selects
            // record_date. Preserve both sources; never reinterpret the
            // subscription delivery as record-date coverage. The full matrix
            // separately requires the PaidinRecord chain.
            let coverage_date = if wire.logical_class == Class::PaidinSubscription {
                let text = row
                    .get("sub_term_ft")
                    .and_then(Value::as_str)
                    .filter(|text| text.len() == 8 && text.bytes().all(|b| b.is_ascii_digit()))
                    .ok_or(Error::InvalidActions)?;
                parse_kis_date(
                    ResponseKind::CorporateActions,
                    &file.file_name,
                    "sub_term_ft",
                    text,
                )
                .map_err(|_| Error::InvalidActions)?
            } else {
                parsed.record_date
            };
            if coverage_date < wire.range_start || coverage_date > wire.range_end {
                return Err(Error::InvalidActions);
            }
            if let Some(expected) = &wire.symbol {
                if &parsed.symbol != expected {
                    return Err(Error::InvalidActions);
                }
                // Preserve the provider's independent aliases check, including
                // case variants; never rely solely on the primary sht_cd field.
                for (name, value) in row {
                    if ["sht_cd", "stck_shrn_iscd", "isu_srt_cd"]
                        .iter()
                        .any(|alias| name.eq_ignore_ascii_case(alias))
                    {
                        let value = value.as_str().ok_or(Error::InvalidActions)?;
                        if !value.is_empty() && value != expected {
                            return Err(Error::InvalidActions);
                        }
                    }
                }
            }
            let canonical = canonical_json(value)?;
            add_budget(
                comparison_bytes,
                canonical.len(),
                fixed_limits().max_normalized_document_bytes,
            )?;
            if !seen.insert(canonical.clone()) {
                return Err(Error::SourceConflict);
            }
            if !KR_ETF_CORE_SYMBOLS.contains(&parsed.symbol.as_str())
                || coverage_date <= previous_frontier
                || coverage_date > session
            {
                continue;
            }
            cells
                .entry((coverage_date, parsed.symbol.clone()))
                .or_default()
                .push(canonical);
            let Some(dividend) = parsed.dividend else {
                return Err(Error::NonCashAction);
            };
            if dividend.stock_dividend_rate != FixedPoint::ZERO {
                return Err(Error::NonCashAction);
            }
            if wire.cash_dividends.len() as u64 >= fixed_limits().max_cash_rows_per_chain {
                return Err(Error::LimitExceeded);
            }
            wire.cash_dividends.push(DailyCashDividendRowV1Wire {
                symbol: parsed.symbol,
                record_date: dividend.record_date,
                dividend_kind: dividend.dividend_kind,
                face_value: decimal(dividend.face_value),
                cash_dividend_amount: decimal(dividend.cash_dividend_amount),
                cash_dividend_rate: decimal(dividend.cash_dividend_rate),
                stock_dividend_rate: decimal(dividend.stock_dividend_rate),
                cash_payment_date: dividend.cash_payment_date,
                stock_payment_date: dividend.stock_payment_date,
                odd_lot_payment_date: dividend.odd_lot_payment_date,
                stock_kind: dividend.stock_kind,
                high_dividend_flag: dividend.high_dividend_flag,
                source_file: file_key(file),
            });
        }
    }
    for rows in cells.values_mut() {
        rows.sort_unstable();
    }
    canonicalize_action(&mut wire).map_err(|_| Error::InvalidActions)?;
    wire.cash_dividend_rows_sha256 = ContentHash::from_bytes(
        &daily_cash_dividend_rows_preimage_v1(&wire.cash_dividends)
            .map_err(|_| Error::InvalidActions)?,
    );
    validate_action_chain(&wire, previous_frontier, session).map_err(|_| Error::InvalidActions)?;
    Ok(ActionChain { wire, rows: cells })
}

fn decimal(value: FixedPoint) -> String {
    let text = value.to_string();
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}
