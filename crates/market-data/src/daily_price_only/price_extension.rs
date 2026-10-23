//! Shared exact price reconstruction for already-protected historical/daily prefixes.
//! Module-private views do not create an approved capability or storage authority.

use super::contract::fixed_limits;
use super::discovery::DiscoveredSessionSources;
use super::history::{
    check_extension_context, checked_addition, compare_overlap, compare_target_observations,
    encode_cumulative_bars, exact_original_rows, prefix_index, same_raw_prices,
};
use super::verify::{DailyVerificationError as Error, RawEvidenceSnapshots, resolve_original};
use crate::HistoricalPriceOnlyBar;
use crate::contract::SourceFileReference;
use domain::{ContentHash, TradingDate};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct PricePrefixView<'a> {
    pub bars: &'a [HistoricalPriceOnlyBar],
    pub sessions: &'a [TradingDate],
    pub policy_sha256: &'a ContentHash,
}

pub(super) struct ReconstructedPrices {
    pub added_bars: Vec<HistoricalPriceOnlyBar>,
    pub bars_ndjson: Vec<u8>,
    pub bars_sha256: ContentHash,
    pub inspected_rows: usize,
    pub compared_rows: usize,
}

pub(super) fn reconstruct_prefix_prices(
    prefix: PricePrefixView<'_>,
    sources: &DiscoveredSessionSources<'_>,
    snapshots: &RawEvidenceSnapshots<'_>,
) -> Result<ReconstructedPrices, Error> {
    let start = *prefix.sessions.first().ok_or(Error::InvalidContext)?;
    let frontier = *prefix.sessions.last().ok_or(Error::InvalidContext)?;
    let index = prefix_index(prefix.bars, prefix.sessions, start, frontier)?;
    check_extension_context(
        frontier,
        prefix.policy_sha256,
        sources.session().previous_frontier,
        sources.session().session_date,
        sources.policy_sha256(),
    )?;
    let target = sources.session().session_date;
    let mut added = BTreeMap::new();
    let mut target_observations = BTreeMap::new();
    let mut inspected_rows = 0usize;
    let mut compared_rows = 0usize;
    let mut bytes = 0u64;
    for observed in sources.observed_bars() {
        let evidence = observed.source();
        let reference = SourceFileReference {
            provider: evidence.provider.clone(),
            market: evidence.market.clone(),
            batch_id: evidence.batch_id,
            file_name: evidence.file_name.clone(),
            content_hash: evidence.wire_sha256.clone(),
            retrieved_at: evidence.retrieved_at.as_timestamp(),
        };
        let original = resolve_original(snapshots, &reference)?;
        if original.evidence() != evidence {
            return Err(Error::SourceConflict);
        }
        bytes = bytes
            .checked_add(original.original_bytes().len() as u64)
            .ok_or(Error::LimitExceeded)?;
        if bytes > fixed_limits().max_bars_bytes {
            return Err(Error::LimitExceeded);
        }
        let rows = exact_original_rows(original.original_bytes(), evidence)?;
        if rows.len() != observed.rows().len() {
            return Err(Error::SourceConflict);
        }
        for (row, canonical) in rows.iter().zip(observed.rows()) {
            let expected_date = row.session_date.to_iso();
            let expected_instrument = row.instrument_id.to_string();
            if canonical.get("date").and_then(Value::as_str) != Some(expected_date.as_str())
                || canonical.get("instrument").and_then(Value::as_str)
                    != Some(expected_instrument.as_str())
            {
                return Err(Error::SourceConflict);
            }
            inspected_rows = inspected_rows.checked_add(1).ok_or(Error::LimitExceeded)?;
            if compare_overlap(prefix.bars, &index, prefix.sessions, row)? {
                compared_rows = compared_rows.checked_add(1).ok_or(Error::LimitExceeded)?;
            }
            if row.session_date == target {
                if let Some(previous) = target_observations.get(&row.instrument_id) {
                    if !same_raw_prices(previous, row) {
                        return Err(Error::SourceConflict);
                    }
                } else {
                    target_observations.insert(row.instrument_id.clone(), row.clone());
                }
            }
        }
        if sources.session().bars.contains(evidence) {
            let row = rows
                .into_iter()
                .find(|row| row.session_date == target)
                .ok_or(Error::SourceCoverageGap)?;
            if added.insert(row.instrument_id.clone(), row).is_some() {
                return Err(Error::SourceConflict);
            }
        }
    }
    let added_bars = checked_addition(added.into_values().collect(), target)?;
    compare_target_observations(
        &added_bars,
        &target_observations.into_values().collect::<Vec<_>>(),
    )?;
    if prefix
        .sessions
        .len()
        .checked_add(1)
        .ok_or(Error::LimitExceeded)? as u64
        > fixed_limits().max_sessions
    {
        return Err(Error::LimitExceeded);
    }
    let bars_ndjson =
        encode_cumulative_bars(prefix.bars, &added_bars, fixed_limits().max_bars_bytes)?;
    let bars_sha256 = ContentHash::from_bytes(&bars_ndjson);
    Ok(ReconstructedPrices {
        added_bars,
        bars_ndjson,
        bars_sha256,
        inspected_rows,
        compared_rows,
    })
}

/// Recheck all surrounding Raw rows against an already sealed full prefix. This
/// also runs on a no-new-session refresh, so out-of-range deliveries cannot hide
/// a changed historical/daily overlap merely by not selecting today's file.
pub(super) fn compare_observed_prefix(
    prefix: PricePrefixView<'_>,
    sources: &DiscoveredSessionSources<'_>,
    snapshots: &RawEvidenceSnapshots<'_>,
) -> Result<Option<super::DailyFileEvidenceV1Wire>, Error> {
    if prefix.policy_sha256 != sources.policy_sha256() {
        return Err(Error::InvalidContext);
    }
    let start = *prefix.sessions.first().ok_or(Error::InvalidContext)?;
    let end = *prefix.sessions.last().ok_or(Error::InvalidContext)?;
    let index = prefix_index(prefix.bars, prefix.sessions, start, end)?;
    let mut bytes = 0u64;
    for observed in sources.observed_bars() {
        let evidence = observed.source();
        let reference = SourceFileReference {
            provider: evidence.provider.clone(),
            market: evidence.market.clone(),
            batch_id: evidence.batch_id,
            file_name: evidence.file_name.clone(),
            content_hash: evidence.wire_sha256.clone(),
            retrieved_at: evidence.retrieved_at.as_timestamp(),
        };
        let original = resolve_original(snapshots, &reference)?;
        if original.evidence() != evidence {
            return Err(Error::SourceConflict);
        }
        bytes = bytes
            .checked_add(original.original_bytes().len() as u64)
            .ok_or(Error::LimitExceeded)?;
        if bytes > fixed_limits().max_bars_bytes {
            return Err(Error::LimitExceeded);
        }
        let rows = exact_original_rows(original.original_bytes(), evidence)?;
        if rows.len() != observed.rows().len() {
            return Err(Error::SourceConflict);
        }
        for (row, canonical) in rows.iter().zip(observed.rows()) {
            if canonical.get("date").and_then(Value::as_str)
                != Some(row.session_date.to_iso().as_str())
                || canonical.get("instrument").and_then(Value::as_str)
                    != Some(row.instrument_id.to_string().as_str())
            {
                return Err(Error::SourceConflict);
            }
            match compare_overlap(prefix.bars, &index, prefix.sessions, row) {
                Err(Error::SourceConflict) => return Ok(Some(evidence.clone())),
                Err(error) => return Err(error),
                Ok(_) => {}
            }
        }
    }
    Ok(None)
}
