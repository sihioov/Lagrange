//! Read-only provider reconstruction over resolved, committed originals.
//! These documents are evidence, not an admitted artifact or a complete frontier.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Datelike;
use domain::{ContentHash, FixedPoint, TradingDate};
use serde_json::{Map, Value, json};

use super::contract::{
    canonical_cash_rows, canonicalize_action, fixed_limits, validate_action_chain,
    validate_file_request_for_symbol,
};
use super::verify::{
    DailyVerificationError as Error, RawEvidenceSnapshots, ResolvedOriginal, canonical_json,
    parse_provider_json, resolve_original,
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
use crate::providers::kis::{KIS_ACTION_SPECS, KR_ETF_CORE_SYMBOLS};

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
                let output = document
                    .get("output")
                    .and_then(Value::as_object)
                    .ok_or(Error::InvalidReference)?;
                let actual = output
                    .get("stck_shrn_iscd")
                    .and_then(Value::as_str)
                    .ok_or(Error::InvalidReference)?;
                if actual != symbol {
                    return Err(Error::InvalidReference);
                }
                json!({"instrument": instrument, "provider_symbol": actual})
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

struct ActionPage<'snapshot> {
    ordinal: usize,
    source: ResolvedOriginal<'snapshot>,
}

/// Verifies all complete overlapping committed bundles and every intervening civil
/// day. This performs no collection, normalization writes, sealing or admission.
pub fn reconstruct_actions<'snapshot>(
    snapshots: &RawEvidenceSnapshots<'snapshot>,
    previous_frontier: TradingDate,
    session: TradingDate,
) -> Result<VerifiedActionDocument<'snapshot>, Error> {
    let limits = fixed_limits();
    let dates = required_dates(previous_frontier, session)?;
    let snapshot = snapshots.kis_snapshot()?;
    let mut chains = Vec::new();
    let mut originals = Vec::new();
    let mut comparison_bytes = 0_u64;
    let mut evidence_bytes = 0_u64;
    for batch in snapshot.batches() {
        if !snapshots.matches_entitlement(batch) {
            continue;
        }
        let files: Vec<_> = batch
            .files()
            .filter(|file| file.kind() == ResponseKind::CorporateActions)
            .collect();
        if files.is_empty() {
            continue;
        }
        let mut range = None;
        let mut bundle_scope = None;
        // Request range, not the manifest partition, determines discovery. Every
        // action page of a bundle must describe the same interval and scope.
        for file in &files {
            let query = &file.entry().request.query;
            let identity = range_and_scope(query)?;
            if range.is_some_and(|value| value != (identity.0, identity.1))
                || bundle_scope.is_some_and(|value| value != identity.2)
            {
                return Err(Error::InvalidActions);
            }
            range = Some((identity.0, identity.1));
            bundle_scope = Some(identity.2);
        }
        let (start, end) = range.ok_or(Error::InvalidActions)?;
        if end <= previous_frontier || start > session {
            continue;
        }
        let scope = bundle_scope.ok_or(Error::InvalidActions)?;
        let mut groups: BTreeMap<(Class, Option<String>), Vec<ActionPage<'snapshot>>> =
            BTreeMap::new();
        for file in files {
            if originals.len() as u64 >= limits.max_file_evidence_per_session {
                return Err(Error::LimitExceeded);
            }
            let reference = SourceFileReference {
                provider: "kis".to_owned(),
                market: "kr".to_owned(),
                batch_id: batch.batch_id(),
                file_name: file.file_name().to_owned(),
                content_hash: file.content_hash().clone(),
                retrieved_at: batch.retrieved_at(),
            };
            let source = resolve_original(snapshots, &reference)?;
            add_budget(
                &mut evidence_bytes,
                source.canonical_evidence_bytes().len(),
                limits.max_manifest_bytes,
            )?;
            let (class, symbol, ordinal) = action_page_identity(source.evidence())?;
            originals.push(source.original_bytes());
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
                if chains.len() as u64 >= limits.max_action_chains_per_session {
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
                let chain = reconstruct_chain(
                    wire,
                    &pages,
                    previous_frontier,
                    session,
                    &mut comparison_bytes,
                )?;
                chains.push(chain);
            }
        }
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
            if parsed.record_date < wire.range_start || parsed.record_date > wire.range_end {
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
                || parsed.record_date <= previous_frontier
                || parsed.record_date > session
            {
                continue;
            }
            cells
                .entry((parsed.record_date, parsed.symbol.clone()))
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
