//! Protected v3 prefix and its first exact, unapproved price continuation.
//! No caller-provided bars, wire manifest or policy hash creates approval here.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use chrono::Datelike;
use domain::{ContentHash, FixedPoint, InstrumentId, TradingDate};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::contract::{fixed_limits, validate_file_request_for_symbol};
use super::discovery::DiscoveredSessionSources;
use super::verify::{
    DailyVerificationError as Error, RawEvidenceSnapshots, parse_provider_json, resolve_original,
};
use super::{
    DailyBasePinsV1Wire, DailyEvidenceKindV1, DailyFileEvidenceV1Wire, DailyPricePolicyV1Wire,
    hash_daily_price_policy_v1,
};
use crate::contract::{ResponseKind, SourceFileReference};
use crate::normalize::parse_kis_date;
use crate::providers::kis::KR_ETF_CORE_SYMBOLS;
use crate::range_normalize::ExpectedRangeSessions;
use crate::{ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar};

const V3_REGISTRY: &[u8] = include_bytes!(
    "../../../../configs/evidence/kis-historical-price-only-v3-approved-artifacts.json"
);
const BASE_START: &str = "2016-08-29";
const BASE_END: &str = "2026-08-28";

/// A borrow of the existing opaque v3 approval, bound to a structurally checked
/// policy. This does not attest policy activation, current authorization or DB state.
pub struct ProtectedHistoricalPrefix<'base> {
    base: &'base ApprovedHistoricalPriceOnlyArtifact,
    pins: DailyBasePinsV1Wire,
    policy_sha256: ContentHash,
    sessions: Vec<TradingDate>,
    index: BTreeMap<(InstrumentId, TradingDate), usize>,
}

impl<'base> ProtectedHistoricalPrefix<'base> {
    pub fn from_approved_v3(
        base: &'base ApprovedHistoricalPriceOnlyArtifact,
        policy: &DailyPricePolicyV1Wire,
    ) -> Result<Self, Error> {
        let approved = base.pins();
        if approved.approval_registry_sha256() != &ContentHash::from_bytes(V3_REGISTRY) {
            return Err(Error::InvalidContext);
        }
        let pins = DailyBasePinsV1Wire {
            candidate_content_sha256: approved.candidate_content_sha256().clone(),
            artifact_manifest_sha256: approved.artifact_manifest_sha256().clone(),
            stage5_manifest_sha256: approved.stage5_manifest_sha256().clone(),
            action_manifest_sha256: approved.action_manifest_sha256().clone(),
            approval_registry_sha256: approved.approval_registry_sha256().clone(),
        };
        let policy_sha256 = checked_policy(&pins, policy)?;
        let sessions = ExpectedRangeSessions::approved_xkrx(date(BASE_START)?, date(BASE_END)?)
            .map_err(|_| Error::InvalidContext)?
            .sessions;
        if sessions.len() != 2_452 || base.bars().len() != 26_972 {
            return Err(Error::InvalidContext);
        }
        let index = prefix_index(base.bars(), &sessions, date(BASE_START)?, date(BASE_END)?)?;
        Ok(Self {
            base,
            pins,
            policy_sha256,
            sessions,
            index,
        })
    }

    pub fn bars(&self) -> &[HistoricalPriceOnlyBar] {
        self.base.bars()
    }
    pub fn sessions(&self) -> &[TradingDate] {
        &self.sessions
    }
    pub fn pins(&self) -> &DailyBasePinsV1Wire {
        &self.pins
    }
    pub fn policy_sha256(&self) -> &ContentHash {
        &self.policy_sha256
    }
    pub fn frontier(&self) -> TradingDate {
        // The constructor checked the nonempty exact approved grid.
        self.sessions[self.sessions.len() - 1]
    }
}

/// First continuation only. The retained references prevent detached source/base
/// claims, but convey no seal, daily parent, artifact approval or admission.
pub struct ReconstructedHistoricalExtension<'proof, 'base, 'snapshot> {
    prefix: &'proof ProtectedHistoricalPrefix<'base>,
    sources: &'proof DiscoveredSessionSources<'snapshot>,
    added_bars: Vec<HistoricalPriceOnlyBar>,
    bars_ndjson: Vec<u8>,
    bars_sha256: ContentHash,
    inspected_rows: usize,
    compared_rows: usize,
}

impl ReconstructedHistoricalExtension<'_, '_, '_> {
    pub fn prefix(&self) -> &ProtectedHistoricalPrefix<'_> {
        self.prefix
    }
    pub fn sources(&self) -> &DiscoveredSessionSources<'_> {
        self.sources
    }
    pub fn added_bars(&self) -> &[HistoricalPriceOnlyBar] {
        &self.added_bars
    }
    pub fn bars_ndjson(&self) -> &[u8] {
        &self.bars_ndjson
    }
    pub fn bars_sha256(&self) -> &ContentHash {
        &self.bars_sha256
    }
    pub fn inspected_rows(&self) -> usize {
        self.inspected_rows
    }
    pub fn compared_rows(&self) -> usize {
        self.compared_rows
    }
}

pub fn reconstruct_historical_extension<'proof, 'base, 'snapshot>(
    prefix: &'proof ProtectedHistoricalPrefix<'base>,
    sources: &'proof DiscoveredSessionSources<'snapshot>,
    snapshots: &RawEvidenceSnapshots<'snapshot>,
) -> Result<ReconstructedHistoricalExtension<'proof, 'base, 'snapshot>, Error> {
    check_extension_context(
        prefix.frontier(),
        prefix.policy_sha256(),
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
            if compare_overlap(prefix.bars(), &prefix.index, prefix.sessions(), row)? {
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
        encode_cumulative_bars(prefix.bars(), &added_bars, fixed_limits().max_bars_bytes)?;
    let bars_sha256 = ContentHash::from_bytes(&bars_ndjson);
    Ok(ReconstructedHistoricalExtension {
        prefix,
        sources,
        added_bars,
        bars_ndjson,
        bars_sha256,
        inspected_rows,
        compared_rows,
    })
}

fn checked_policy(
    pins: &DailyBasePinsV1Wire,
    policy: &DailyPricePolicyV1Wire,
) -> Result<ContentHash, Error> {
    if policy.base.kind != "historical_price_only_v3" || &policy.base.pins != pins {
        return Err(Error::InvalidContext);
    }
    hash_daily_price_policy_v1(policy).map_err(|_| Error::InvalidContext)
}

fn check_extension_context(
    frontier: TradingDate,
    policy: &ContentHash,
    previous: TradingDate,
    target: TradingDate,
    source_policy: &ContentHash,
) -> Result<(), Error> {
    if previous != frontier || target <= frontier || source_policy != policy {
        return Err(Error::InvalidContext);
    }
    Ok(())
}

fn date(value: &str) -> Result<TradingDate, Error> {
    TradingDate::parse(value).map_err(|_| Error::InvalidContext)
}

fn instruments() -> Result<BTreeSet<InstrumentId>, Error> {
    KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|s| InstrumentId::parse(&format!("{s}.KRX")).map_err(|_| Error::InvalidIdentity))
        .collect()
}

fn prefix_index(
    bars: &[HistoricalPriceOnlyBar],
    sessions: &[TradingDate],
    start: TradingDate,
    end: TradingDate,
) -> Result<BTreeMap<(InstrumentId, TradingDate), usize>, Error> {
    let limits = fixed_limits();
    if sessions.len() as u64 > limits.max_sessions
        || bars.len() as u64
            > limits
                .max_sessions
                .checked_mul(11)
                .ok_or(Error::LimitExceeded)?
    {
        return Err(Error::LimitExceeded);
    }
    if sessions.is_empty()
        || sessions.first() != Some(&start)
        || sessions.last() != Some(&end)
        || sessions.windows(2).any(|dates| dates[0] >= dates[1])
        || bars.len() != sessions.len().checked_mul(11).ok_or(Error::LimitExceeded)?
    {
        return Err(Error::InvalidContext);
    }
    let expected = instruments()?;
    let mut index = BTreeMap::new();
    let mut previous = None;
    for (position, bar) in bars.iter().enumerate() {
        let key = (bar.instrument_id.clone(), bar.session_date);
        if !expected.contains(&bar.instrument_id)
            || sessions.binary_search(&bar.session_date).is_err()
            || previous.as_ref().is_some_and(|old| old >= &key)
        {
            return Err(Error::InvalidContext);
        }
        validate_bar(bar)?;
        previous = Some(key.clone());
        if index.insert(key, position).is_some() {
            return Err(Error::InvalidContext);
        }
    }
    for instrument in expected {
        for session in sessions {
            if !index.contains_key(&(instrument.clone(), *session)) {
                return Err(Error::InvalidContext);
            }
        }
    }
    Ok(index)
}

fn compare_overlap(
    bars: &[HistoricalPriceOnlyBar],
    index: &BTreeMap<(InstrumentId, TradingDate), usize>,
    sessions: &[TradingDate],
    observed: &HistoricalPriceOnlyBar,
) -> Result<bool, Error> {
    let start = sessions.first().ok_or(Error::InvalidContext)?;
    let end = sessions.last().ok_or(Error::InvalidContext)?;
    if observed.session_date < *start || observed.session_date > *end {
        return Ok(false);
    }
    let position = index
        .get(&(observed.instrument_id.clone(), observed.session_date))
        .ok_or(Error::SourceConflict)?;
    let old = &bars[*position];
    if !same_raw_prices(old, observed) {
        return Err(Error::SourceConflict);
    }
    Ok(true)
}

fn same_raw_prices(old: &HistoricalPriceOnlyBar, observed: &HistoricalPriceOnlyBar) -> bool {
    let old_prices = [old.raw_open, old.raw_high, old.raw_low, old.raw_close];
    let new_prices = [
        observed.raw_open,
        observed.raw_high,
        observed.raw_low,
        observed.raw_close,
    ];
    !old_prices
        .into_iter()
        .zip(new_prices)
        .any(|(old, new)| exact_parts(old) != exact_parts(new))
        && old.raw_volume == observed.raw_volume
        && old.raw_trading_value.map(exact_parts) == observed.raw_trading_value.map(exact_parts)
}

fn compare_target_observations(
    added: &[HistoricalPriceOnlyBar],
    observed: &[HistoricalPriceOnlyBar],
) -> Result<(), Error> {
    for row in observed {
        let expected = added
            .iter()
            .find(|expected| {
                expected.instrument_id == row.instrument_id
                    && expected.session_date == row.session_date
            })
            .ok_or(Error::SourceConflict)?;
        if !same_raw_prices(expected, row) {
            return Err(Error::SourceConflict);
        }
    }
    Ok(())
}

fn exact_original_rows(
    bytes: &[u8],
    evidence: &DailyFileEvidenceV1Wire,
) -> Result<Vec<HistoricalPriceOnlyBar>, Error> {
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
    let target = evidence
        .copied_from
        .last()
        .map_or(evidence.batch_date, |origin| origin.batch_date);
    validate_file_request_for_symbol(evidence, DailyEvidenceKindV1::Bars, symbol, target)
        .map_err(|_| Error::InvalidEvidence)?;
    let document = parse_provider_json(bytes)?;
    if document.get("rt_cd").and_then(Value::as_str) != Some("0")
        || !document.get("output1").is_some_and(Value::is_object)
    {
        return Err(Error::InvalidBars);
    }
    let rows = document
        .get("output2")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidBars)?;
    let instrument =
        InstrumentId::parse(&format!("{symbol}.KRX")).map_err(|_| Error::InvalidIdentity)?;
    let mut parsed = BTreeMap::new();
    for row in rows {
        let row = exact_provider_bar(row, &instrument)?;
        if parsed.insert(row.session_date, row).is_some() {
            return Err(Error::SourceConflict);
        }
    }
    if !parsed.contains_key(&target) {
        return Err(Error::SourceCoverageGap);
    }
    Ok(parsed.into_values().collect())
}

fn exact_provider_bar(
    value: &Value,
    instrument: &InstrumentId,
) -> Result<HistoricalPriceOnlyBar, Error> {
    let row = value.as_object().ok_or(Error::InvalidBars)?;
    let text = row
        .get("stck_bsop_date")
        .and_then(Value::as_str)
        .ok_or(Error::InvalidBars)?;
    let session_date = parse_kis_date(ResponseKind::Bars, "<bar>", "stck_bsop_date", text)
        .map_err(|_| Error::InvalidBars)?;
    if !(1..=9999).contains(&session_date.as_naive_date().year()) {
        return Err(Error::InvalidBars);
    }
    let raw_open = decimal(row, "stck_oprc")?;
    let raw_high = decimal(row, "stck_hgpr")?;
    let raw_low = decimal(row, "stck_lwpr")?;
    let raw_close = decimal(row, "stck_clpr")?;
    let raw_volume = match row.get("acml_vol") {
        Some(Value::String(value))
            if !value.is_empty()
                && value.len() <= 20
                && value.bytes().all(|c| c.is_ascii_digit()) =>
        {
            value.parse::<u64>().map_err(|_| Error::InvalidBars)?
        }
        Some(Value::Number(value)) => value.as_u64().ok_or(Error::InvalidBars)?,
        _ => return Err(Error::InvalidBars),
    };
    let raw_trading_value = row
        .contains_key("acml_tr_pbmn")
        .then(|| decimal(row, "acml_tr_pbmn"))
        .transpose()?;
    let bar = HistoricalPriceOnlyBar {
        instrument_id: instrument.clone(),
        session_date,
        raw_open,
        raw_high,
        raw_low,
        raw_close,
        raw_volume,
        raw_trading_value,
        adjusted_open: raw_open,
        adjusted_high: raw_high,
        adjusted_low: raw_low,
        adjusted_close: raw_close,
    };
    validate_bar(&bar)?;
    Ok(bar)
}

fn decimal(row: &Map<String, Value>, field: &str) -> Result<FixedPoint, Error> {
    let text = match row.get(field) {
        Some(Value::String(value)) if value.len() <= 80 => value.clone(),
        Some(Value::Number(value)) if value.is_i64() || value.is_u64() => value.to_string(),
        _ => return Err(Error::InvalidBars),
    };
    if text.is_empty() || text.len() > 80 {
        return Err(Error::InvalidBars);
    }
    let unsigned = text.strip_prefix('-').unwrap_or(&text);
    let mut parts = unsigned.split('.');
    let whole = parts.next().ok_or(Error::InvalidBars)?;
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::InvalidBars);
    }
    FixedPoint::parse(&text).map_err(|_| Error::InvalidBars)
}

fn exact_parts(value: FixedPoint) -> (i128, u8) {
    let (mut bits, mut scale) = (value.bits(), value.scale());
    while scale > 0 && bits % 10 == 0 {
        bits /= 10;
        scale -= 1;
    }
    (bits, scale)
}

fn exact_cmp(left: FixedPoint, right: FixedPoint) -> Result<Ordering, Error> {
    let (left, left_scale) = exact_parts(left);
    let (right, right_scale) = exact_parts(right);
    let scale = left_scale.max(right_scale);
    let left = left
        .checked_mul(10i128.pow(u32::from(scale - left_scale)))
        .ok_or(Error::InvalidBars)?;
    let right = right
        .checked_mul(10i128.pow(u32::from(scale - right_scale)))
        .ok_or(Error::InvalidBars)?;
    Ok(left.cmp(&right))
}

fn validate_bar(bar: &HistoricalPriceOnlyBar) -> Result<(), Error> {
    for [open, high, low, close] in [
        [bar.raw_open, bar.raw_high, bar.raw_low, bar.raw_close],
        [
            bar.adjusted_open,
            bar.adjusted_high,
            bar.adjusted_low,
            bar.adjusted_close,
        ],
    ] {
        if [open, high, low, close]
            .iter()
            .any(|price| !price.is_positive())
            || exact_cmp(high, open)? == Ordering::Less
            || exact_cmp(high, close)? == Ordering::Less
            || exact_cmp(low, open)? == Ordering::Greater
            || exact_cmp(low, close)? == Ordering::Greater
            || exact_cmp(low, high)? == Ordering::Greater
        {
            return Err(Error::InvalidBars);
        }
    }
    if bar
        .raw_trading_value
        .is_some_and(|value| value.is_negative())
    {
        return Err(Error::InvalidBars);
    }
    Ok(())
}

fn checked_addition(
    mut bars: Vec<HistoricalPriceOnlyBar>,
    target: TradingDate,
) -> Result<Vec<HistoricalPriceOnlyBar>, Error> {
    bars.sort_by_key(|bar| bar.instrument_id.clone());
    let actual = bars
        .iter()
        .map(|bar| bar.instrument_id.clone())
        .collect::<BTreeSet<_>>();
    if bars.len() != 11
        || actual != instruments()?
        || bars.iter().any(|bar| bar.session_date != target)
    {
        return Err(Error::SourceCoverageGap);
    }
    for bar in &bars {
        validate_bar(bar)?;
        if [bar.raw_open, bar.raw_high, bar.raw_low, bar.raw_close]
            .into_iter()
            .zip([
                bar.adjusted_open,
                bar.adjusted_high,
                bar.adjusted_low,
                bar.adjusted_close,
            ])
            .any(|(raw, adjusted)| raw.bits() != adjusted.bits() || raw.scale() != adjusted.scale())
        {
            return Err(Error::InvalidBars);
        }
    }
    Ok(bars)
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BarRow {
    instrument_id: String,
    session_date: TradingDate,
    raw_open: String,
    raw_high: String,
    raw_low: String,
    raw_close: String,
    raw_volume: u64,
    raw_trading_value: Option<String>,
    adjusted_open: String,
    adjusted_high: String,
    adjusted_low: String,
    adjusted_close: String,
}

impl From<&HistoricalPriceOnlyBar> for BarRow {
    fn from(bar: &HistoricalPriceOnlyBar) -> Self {
        Self {
            instrument_id: bar.instrument_id.to_string(),
            session_date: bar.session_date,
            raw_open: bar.raw_open.to_string(),
            raw_high: bar.raw_high.to_string(),
            raw_low: bar.raw_low.to_string(),
            raw_close: bar.raw_close.to_string(),
            raw_volume: bar.raw_volume,
            raw_trading_value: bar.raw_trading_value.map(|value| value.to_string()),
            adjusted_open: bar.adjusted_open.to_string(),
            adjusted_high: bar.adjusted_high.to_string(),
            adjusted_low: bar.adjusted_low.to_string(),
            adjusted_close: bar.adjusted_close.to_string(),
        }
    }
}

struct BoundedBytes {
    bytes: Vec<u8>,
    maximum: u64,
    exceeded: bool,
}
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(size) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("LIMIT"));
        };
        if size as u64 > self.maximum || self.bytes.try_reserve(bytes.len()).is_err() {
            self.exceeded = true;
            return Err(io::Error::other("LIMIT"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_cumulative_bars(
    base: &[HistoricalPriceOnlyBar],
    added: &[HistoricalPriceOnlyBar],
    maximum: u64,
) -> Result<Vec<u8>, Error> {
    let count = base
        .len()
        .checked_add(added.len())
        .ok_or(Error::LimitExceeded)?;
    if count as u64
        > fixed_limits()
            .max_sessions
            .checked_mul(11)
            .ok_or(Error::LimitExceeded)?
    {
        return Err(Error::LimitExceeded);
    }
    let mut bars = Vec::new();
    bars.try_reserve(count).map_err(|_| Error::LimitExceeded)?;
    bars.extend(base.iter().chain(added));
    bars.sort_by(|a, b| {
        a.instrument_id
            .cmp(&b.instrument_id)
            .then(a.session_date.cmp(&b.session_date))
    });
    if bars.windows(2).any(|pair| {
        pair[0].instrument_id == pair[1].instrument_id
            && pair[0].session_date == pair[1].session_date
    }) {
        return Err(Error::SourceConflict);
    }
    let mut writer = BoundedBytes {
        bytes: Vec::new(),
        maximum,
        exceeded: false,
    };
    for bar in bars {
        if serde_json::to_writer(&mut writer, &BarRow::from(bar)).is_err()
            || writer.write_all(b"\n").is_err()
        {
            return Err(if writer.exceeded {
                Error::LimitExceeded
            } else {
                Error::Encoding
            });
        }
    }
    Ok(writer.bytes)
}

/// First-admission readback only. The caller separately checks the four sealed
/// files, pinned policy, admission and expected hashes before exposing a handle.
pub(super) fn read_first_admission_bars(
    prefix: &ProtectedHistoricalPrefix<'_>,
    manifest: &super::DailyPriceManifestV1Wire,
    bytes: &[u8],
) -> Result<Vec<HistoricalPriceOnlyBar>, Error> {
    let mut sessions = prefix.sessions().to_vec();
    sessions.push(manifest.range_end);
    if manifest.parent_admission_sha256.is_some()
        || manifest.daily_sources.len() != 1
        || manifest.sessions != sessions
        || manifest.range_start != date(BASE_START)?
        || manifest.range_end <= prefix.frontier()
        || manifest.daily_sources[0].previous_frontier != prefix.frontier()
        || manifest.daily_sources[0].session_date != manifest.range_end
        || &manifest.base.pins != prefix.pins()
        || &manifest.policy_sha256 != prefix.policy_sha256()
        || manifest.bars.size_bytes != bytes.len() as u64
        || manifest.bars.sha256 != ContentHash::from_bytes(bytes)
    {
        return Err(Error::InvalidContext);
    }
    let maximum = sessions.len().checked_mul(11).ok_or(Error::LimitExceeded)?;
    let bars = decode_stored_bars(bytes, maximum)?;
    if bars.len() != maximum || manifest.bars.row_count != maximum as u64 {
        return Err(Error::InvalidBars);
    }
    let index = prefix_index(&bars, &sessions, manifest.range_start, manifest.range_end)?;
    compare_preserved_rows(prefix.bars(), &bars, &index)?;
    let additions = bars
        .iter()
        .filter(|bar| bar.session_date == manifest.range_end)
        .cloned()
        .collect();
    checked_addition(additions, manifest.range_end)?;
    Ok(bars)
}

fn compare_preserved_rows(
    expected: &[HistoricalPriceOnlyBar],
    actual: &[HistoricalPriceOnlyBar],
    index: &BTreeMap<(InstrumentId, TradingDate), usize>,
) -> Result<(), Error> {
    for prior in expected {
        let position = index
            .get(&(prior.instrument_id.clone(), prior.session_date))
            .ok_or(Error::SourceConflict)?;
        let current = actual.get(*position).ok_or(Error::SourceConflict)?;
        // Includes raw/adjusted representation, optional value and scale, not
        // only rounded equality. Canonical readback must preserve all base bytes.
        if BarRow::from(prior) != BarRow::from(current) {
            return Err(Error::SourceConflict);
        }
    }
    Ok(())
}

fn decode_stored_bars(
    bytes: &[u8],
    maximum_rows: usize,
) -> Result<Vec<HistoricalPriceOnlyBar>, Error> {
    if bytes.len() as u64 > fixed_limits().max_bars_bytes
        || maximum_rows as u64
            > fixed_limits()
                .max_sessions
                .checked_mul(11)
                .ok_or(Error::LimitExceeded)?
    {
        return Err(Error::LimitExceeded);
    }
    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err(Error::InvalidBars);
    }
    let mut bars = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|byte| *byte == b'\n') {
        if bars.len() == maximum_rows || line.len() > 16 * 1024 {
            return Err(Error::LimitExceeded);
        }
        let row: BarRow = serde_json::from_slice(line).map_err(|_| Error::InvalidBars)?;
        if serde_json::to_vec(&row).map_err(|_| Error::Encoding)? != line {
            return Err(Error::InvalidBars);
        }
        let instrument =
            InstrumentId::parse(&row.instrument_id).map_err(|_| Error::InvalidIdentity)?;
        if instrument.to_string() != row.instrument_id {
            return Err(Error::InvalidIdentity);
        }
        let bar = HistoricalPriceOnlyBar {
            instrument_id: instrument,
            session_date: row.session_date,
            raw_open: stored_decimal(&row.raw_open)?,
            raw_high: stored_decimal(&row.raw_high)?,
            raw_low: stored_decimal(&row.raw_low)?,
            raw_close: stored_decimal(&row.raw_close)?,
            raw_volume: row.raw_volume,
            raw_trading_value: row
                .raw_trading_value
                .as_deref()
                .map(stored_decimal)
                .transpose()?,
            adjusted_open: stored_decimal(&row.adjusted_open)?,
            adjusted_high: stored_decimal(&row.adjusted_high)?,
            adjusted_low: stored_decimal(&row.adjusted_low)?,
            adjusted_close: stored_decimal(&row.adjusted_close)?,
        };
        validate_bar(&bar)?;
        bars.try_reserve(1).map_err(|_| Error::LimitExceeded)?;
        bars.push(bar);
    }
    Ok(bars)
}

fn stored_decimal(text: &str) -> Result<FixedPoint, Error> {
    if text.len() > 80 {
        return Err(Error::InvalidBars);
    }
    let value = FixedPoint::parse(text).map_err(|_| Error::InvalidBars)?;
    if value.to_string() != text {
        return Err(Error::InvalidBars);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::super::{
        DailyBaseV1Wire, DailyRequestMetadataV1Wire, DailySourceKindV1Wire, DailyWireTimestampV1,
    };
    use super::*;
    use domain::BatchId;
    use serde_json::json;
    use uuid::Uuid;

    fn day(s: &str) -> TradingDate {
        TradingDate::parse(s).unwrap()
    }
    fn fp(s: &str) -> FixedPoint {
        FixedPoint::parse(s).unwrap()
    }
    fn id() -> InstrumentId {
        InstrumentId::parse("069500.KRX").unwrap()
    }
    fn pin(s: &str) -> ContentHash {
        ContentHash::from_bytes(s.as_bytes())
    }
    fn row(day: &str) -> Value {
        json!({"stck_bsop_date":day,"stck_oprc":"10.00","stck_hgpr":"12.00",
            "stck_lwpr":"9.00","stck_clpr":"11.00","acml_vol":"7","acml_tr_pbmn":"70.00"})
    }
    fn bar(day: &str) -> HistoricalPriceOnlyBar {
        exact_provider_bar(&row(day), &id()).unwrap()
    }
    fn grid(days: &[TradingDate]) -> Vec<HistoricalPriceOnlyBar> {
        instruments()
            .unwrap()
            .into_iter()
            .flat_map(|instrument| {
                days.iter().map(move |date| {
                    let mut bar = bar(&date.to_iso());
                    bar.instrument_id = instrument.clone();
                    bar
                })
            })
            .collect()
    }
    fn evidence() -> DailyFileEvidenceV1Wire {
        let timestamp = DailyWireTimestampV1::parse("2026-08-31T07:31:00.000000000Z").unwrap();
        DailyFileEvidenceV1Wire {
            provider: "kis".into(),
            market: "kr".into(),
            batch_id: BatchId::from_uuid(Uuid::from_u128(1)),
            batch_date: day("2026-08-31"),
            batch_json_sha256: pin("batch"),
            manifest_line_sha256: pin("line"),
            file_name: "069500-bars.json".into(),
            kind: DailyEvidenceKindV1::Bars,
            wire_sha256: pin("body"),
            size_bytes: 1,
            request: DailyRequestMetadataV1Wire {
                endpoint: "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice".into(),
                query: vec![
                    ("FID_COND_MRKT_DIV_CODE".into(), "J".into()),
                    ("FID_INPUT_DATE_1".into(), "20260831".into()),
                    ("FID_INPUT_DATE_2".into(), "20260831".into()),
                    ("FID_INPUT_ISCD".into(), "069500".into()),
                    ("FID_ORG_ADJ_PRC".into(), "1".into()),
                    ("FID_PERIOD_DIV_CODE".into(), "D".into()),
                ],
                headers: vec![
                    ("appkey".into(), "[REDACTED]".into()),
                    ("appsecret".into(), "[REDACTED]".into()),
                    ("authorization".into(), "[REDACTED]".into()),
                    ("tr_cont".into(), "".into()),
                    ("tr_id".into(), "FHKST03010100".into()),
                ],
                mode: "credentialed".into(),
            },
            response_continuation: None,
            retrieved_at: timestamp,
            retrieval_precision: "batch".into(),
            original_retrieved_at: timestamp,
            original_retrieval_precision: "batch".into(),
            copied_from: vec![],
        }
    }
    fn original(rows: Vec<Value>) -> Vec<u8> {
        serde_json::to_vec(&json!({"rt_cd":"0","output1":{},"output2":rows})).unwrap()
    }
    fn policy() -> DailyPricePolicyV1Wire {
        DailyPricePolicyV1Wire {
            schema: "owner-beta-daily-etf11-policy-v1".into(),
            capability: "PRICE_RETURN_ONLY".into(),
            audience: "OWNER_ONLY".into(),
            vendor_snapshot: true,
            strict_pit: false,
            base: DailyBaseV1Wire {
                kind: "historical_price_only_v3".into(),
                pins: DailyBasePinsV1Wire {
                    candidate_content_sha256: pin("content"),
                    artifact_manifest_sha256: pin("manifest"),
                    stage5_manifest_sha256: pin("price"),
                    action_manifest_sha256: pin("action"),
                    approval_registry_sha256: ContentHash::from_bytes(V3_REGISTRY),
                },
            },
            instruments: instruments().unwrap().into_iter().collect(),
            price_semantics: "KIS_FID_ORG_ADJ_PRC_1_RAW_OHLC".into(),
            cash_action_policy: "CASH_DIVIDENDS_EXCLUDED_NONCASH_BLOCKED_V1".into(),
            normalizer_version: "daily-raw-reconstruction-v1".into(),
            materializer_version: "daily-price-only-v1".into(),
            source_kinds: vec![
                DailyEvidenceKindV1::Bars,
                DailyEvidenceKindV1::Calendar,
                DailyEvidenceKindV1::CorporateActions,
                DailyEvidenceKindV1::Reference,
            ]
            .into_iter()
            .map(|kind| DailySourceKindV1Wire {
                provider: "kis".into(),
                market: "kr".into(),
                kind,
            })
            .chain([DailySourceKindV1Wire {
                provider: "kis-calendar".into(),
                market: "kr".into(),
                kind: DailyEvidenceKindV1::Calendar,
            }])
            .collect(),
            limits: fixed_limits(),
        }
    }

    #[test]
    fn s5b_policy_binds_all_five_pins_and_fixed_contract() {
        let valid = policy();
        assert_eq!(
            checked_policy(&valid.base.pins, &valid).unwrap(),
            hash_daily_price_policy_v1(&valid).unwrap()
        );
        for field in 0..5 {
            let mut other = valid.clone();
            let changed = match field {
                0 => &mut other.base.pins.candidate_content_sha256,
                1 => &mut other.base.pins.artifact_manifest_sha256,
                2 => &mut other.base.pins.stage5_manifest_sha256,
                3 => &mut other.base.pins.action_manifest_sha256,
                _ => &mut other.base.pins.approval_registry_sha256,
            };
            *changed = pin("changed");
            assert_eq!(
                checked_policy(&valid.base.pins, &other),
                Err(Error::InvalidContext)
            );
        }
        for field in 0..3 {
            let mut other = valid.clone();
            match field {
                0 => other.base.kind = "historical_price_only_v2".into(),
                1 => other.strict_pit = true,
                _ => other.limits.max_sessions = 20_001,
            };
            assert_eq!(
                checked_policy(&valid.base.pins, &other),
                Err(Error::InvalidContext)
            );
        }
    }

    #[test]
    fn s5b_context_cannot_skip_frontier_or_substitute_policy() {
        let end = day(BASE_END);
        let target = day("2026-08-31");
        let hash = pin("policy");
        assert_eq!(
            check_extension_context(end, &hash, end, target, &hash),
            Ok(())
        );
        for (previous, next, policy) in [
            (day("2026-08-29"), target, hash.clone()),
            (end, end, hash.clone()),
            (end, day("2026-08-27"), hash.clone()),
            (end, target, pin("different")),
        ] {
            assert_eq!(
                check_extension_context(end, &hash, previous, next, &policy),
                Err(Error::InvalidContext)
            );
        }
    }

    #[test]
    fn s5b_prefix_is_a_complete_ordered_instrument_session_grid() {
        let days = [day("2026-08-27"), day(BASE_END)];
        let bars = grid(&days);
        assert_eq!(
            prefix_index(&bars, &days, days[0], days[1]).unwrap().len(),
            22
        );
        for case in 0..5 {
            let mut changed = bars.clone();
            match case {
                0 => {
                    changed.pop();
                }
                1 => changed.push(changed[0].clone()),
                2 => changed.swap(0, 1),
                3 => changed[0].instrument_id = InstrumentId::parse("000020.KRX").unwrap(),
                _ => changed[0].session_date = day("2026-08-26"),
            };
            assert_eq!(
                prefix_index(&changed, &days, days[0], days[1]).err(),
                Some(Error::InvalidContext)
            );
        }
        assert_eq!(
            prefix_index(&bars, &[days[1], days[0]], days[0], days[1]).err(),
            Some(Error::InvalidContext)
        );
        assert_eq!(
            prefix_index(&[], &[], days[0], days[1]).err(),
            Some(Error::InvalidContext)
        );
        let excessive = vec![days[0]; 20_001];
        assert_eq!(
            prefix_index(&[], &excessive, days[0], days[0]).err(),
            Some(Error::LimitExceeded)
        );
    }

    #[test]
    fn s5b_every_raw_overlap_field_and_missing_value_are_protected() {
        let days = [day(BASE_END)];
        let bars = grid(&days);
        let index = prefix_index(&bars, &days, days[0], days[0]).unwrap();
        assert_eq!(compare_overlap(&bars, &index, &days, &bars[0]), Ok(true));
        for case in 0..7 {
            let mut changed = bars[0].clone();
            match case {
                0 => changed.raw_open = fp("10.01"),
                1 => changed.raw_high = fp("12.01"),
                2 => changed.raw_low = fp("8.99"),
                3 => changed.raw_close = fp("11.01"),
                4 => changed.raw_volume = 8,
                5 => changed.raw_trading_value = Some(fp("70.01")),
                _ => changed.raw_trading_value = None,
            };
            assert_eq!(
                compare_overlap(&bars, &index, &days, &changed),
                Err(Error::SourceConflict)
            );
        }
        let mut same = bars[0].clone();
        same.raw_open = fp("10.0000");
        same.adjusted_open = fp("1");
        assert_eq!(compare_overlap(&bars, &index, &days, &same), Ok(true));
        // Compare provider raw values with raw values, never adjusted historical prices.
        let mut absent = bars.clone();
        absent[0].raw_trading_value = None;
        let mut zero = absent[0].clone();
        zero.raw_trading_value = Some(FixedPoint::ZERO);
        assert_eq!(
            compare_overlap(&absent, &index, &days, &zero),
            Err(Error::SourceConflict)
        );
    }

    #[test]
    fn s5b_protected_holiday_row_conflicts_and_outside_rows_are_not_added() {
        let days = [day("2026-08-24"), day(BASE_END)];
        let bars = grid(&days);
        let index = prefix_index(&bars, &days, days[0], days[1]).unwrap();
        assert_eq!(
            compare_overlap(&bars, &index, &days, &bar("2026-08-25")),
            Err(Error::SourceConflict)
        );
        assert_eq!(
            compare_overlap(&bars, &index, &days, &bar("2026-08-23")),
            Ok(false)
        );
        assert_eq!(
            compare_overlap(&bars, &index, &days, &bar("2026-08-31")),
            Ok(false)
        );
    }

    #[test]
    fn s5b_target_row_in_another_requested_day_must_agree() {
        let added = grid(&[day("2026-08-31")]);
        let mut observed = added.clone();
        observed.reverse();
        assert_eq!(compare_target_observations(&added, &observed), Ok(()));
        observed[0].raw_close = fp("10.5");
        assert_eq!(
            compare_target_observations(&added, &observed),
            Err(Error::SourceConflict)
        );
        observed = added.clone();
        observed[0].raw_trading_value = None;
        assert_eq!(
            compare_target_observations(&added, &observed),
            Err(Error::SourceConflict)
        );
    }

    #[test]
    fn s5b_exact_comparison_never_rounds_large_distinct_values() {
        let a = fp("17014118346046923173168730371588410572.1");
        let b = fp("17014118346046923173168730371588410572");
        assert_eq!(exact_cmp(a, b), Ok(Ordering::Greater));
        assert_ne!(exact_parts(a), exact_parts(b));
        assert_eq!(exact_parts(fp("1.00")), exact_parts(fp("1")));
        assert_eq!(
            exact_cmp(fp("170141183460469231731687303715884105727"), fp("0.1")),
            Err(Error::InvalidBars)
        );
        let days = [day(BASE_END)];
        let mut bars = grid(&days);
        bars[0].raw_close = a;
        let index = BTreeMap::from([((bars[0].instrument_id.clone(), days[0]), 0)]);
        let mut changed = bars[0].clone();
        changed.raw_close = b;
        assert_eq!(
            compare_overlap(&bars, &index, &days, &changed),
            Err(Error::SourceConflict)
        );
    }

    #[test]
    fn s5b_original_decimal_strings_keep_precision_and_new_adjusted_fields() {
        let mut value = row("20260831");
        for field in ["stck_oprc", "stck_hgpr", "stck_lwpr", "stck_clpr"] {
            value[field] = json!("9007199254740993.1250");
        }
        value["acml_tr_pbmn"] = json!("12345678901234567890.12345678");
        let result = exact_provider_bar(&value, &id()).unwrap();
        assert_eq!(result.raw_open.bits(), 90_071_992_547_409_931_250i128);
        assert_eq!(result.raw_open.scale(), 4);
        assert_eq!(result.raw_open.bits(), result.adjusted_open.bits());
        assert_eq!(result.raw_open.scale(), result.adjusted_open.scale());
        assert_eq!(
            result.raw_trading_value.unwrap().to_string(),
            "12345678901234567890.12345678"
        );
    }

    #[test]
    fn s5b_fractional_json_numbers_and_exponents_do_not_enter_arithmetic() {
        for field in [
            "stck_oprc",
            "stck_hgpr",
            "stck_lwpr",
            "stck_clpr",
            "acml_tr_pbmn",
        ] {
            for bad in [
                json!(1.25),
                json!("1e2"),
                json!(" 10"),
                json!("+10"),
                json!(".5"),
                json!("5."),
                Value::Null,
                json!("0.00000000000000000000000000001"),
                json!("170141183460469231731687303715884105728"),
            ] {
                let mut value = row("20260831");
                value[field] = bad;
                assert_eq!(
                    exact_provider_bar(&value, &id()).err(),
                    Some(Error::InvalidBars)
                );
            }
        }
        let bytes=br#"{"stck_bsop_date":"20260831","stck_oprc":9007199254740993.1,"stck_hgpr":"12","stck_lwpr":"9","stck_clpr":"11","acml_vol":1}"#;
        let value = parse_provider_json(bytes).unwrap();
        assert_eq!(
            exact_provider_bar(&value, &id()).err(),
            Some(Error::InvalidBars)
        );
    }

    #[test]
    fn s5b_integer_tokens_volume_and_optional_value_boundaries() {
        let mut value = row("20260831");
        for field in ["stck_oprc", "stck_hgpr", "stck_lwpr", "stck_clpr"] {
            value[field] = json!(10);
        }
        value["acml_vol"] = json!(u64::MAX);
        value.as_object_mut().unwrap().remove("acml_tr_pbmn");
        let result = exact_provider_bar(&value, &id()).unwrap();
        assert_eq!(result.raw_volume, u64::MAX);
        assert_eq!(result.raw_trading_value, None);
        for bad in [
            json!("18446744073709551616"),
            json!("1.0"),
            json!(1.0),
            json!(-1),
            json!("-1"),
            json!(" 1"),
            Value::Null,
        ] {
            value["acml_vol"] = bad;
            assert_eq!(
                exact_provider_bar(&value, &id()).err(),
                Some(Error::InvalidBars)
            );
        }
        value["acml_vol"] = json!(0);
        value["acml_tr_pbmn"] = json!(0);
        let result = exact_provider_bar(&value, &id()).unwrap();
        assert_eq!(result.raw_volume, 0);
        assert_eq!(result.raw_trading_value, Some(FixedPoint::ZERO));
        value["acml_tr_pbmn"] = json!("-0.1");
        assert_eq!(
            exact_provider_bar(&value, &id()).err(),
            Some(Error::InvalidBars)
        );
    }

    #[test]
    fn s5b_price_ohlc_and_date_failures_are_not_target_only() {
        for (field, bad) in [
            ("stck_oprc", json!("0")),
            ("stck_clpr", json!("-1")),
            ("stck_hgpr", json!("10.99")),
            ("stck_lwpr", json!("10.01")),
            ("stck_bsop_date", json!("20260230")),
            ("stck_bsop_date", json!("00000101")),
        ] {
            let mut value = row("20260828");
            value[field] = bad;
            let bytes = original(vec![value, row("20260831")]);
            assert_eq!(
                exact_original_rows(&bytes, &evidence()).err(),
                Some(Error::InvalidBars)
            );
        }
    }

    #[test]
    fn s5b_original_scan_preserves_all_dates_and_rejects_duplicate_or_missing_target() {
        let mut rows = vec![row("20260831"), row("20260828"), row("20260901")];
        let forward = exact_original_rows(&original(rows.clone()), &evidence()).unwrap();
        assert_eq!(
            forward.iter().map(|r| r.session_date).collect::<Vec<_>>(),
            [day("2026-08-28"), day("2026-08-31"), day("2026-09-01")]
        );
        rows.reverse();
        assert_eq!(
            exact_original_rows(&original(rows), &evidence()).unwrap(),
            forward
        );
        assert_eq!(
            exact_original_rows(
                &original(vec![row("20260831"), row("20260831")]),
                &evidence()
            )
            .err(),
            Some(Error::SourceConflict)
        );
        assert_eq!(
            exact_original_rows(&original(vec![row("20260828")]), &evidence()).err(),
            Some(Error::SourceCoverageGap)
        );
    }

    #[test]
    fn s5b_original_shape_duplicate_json_and_request_binding_fail_closed() {
        for bytes in [
            br#"{"rt_cd":"0","rt_cd":"0","output1":{},"output2":[]}"#.as_slice(),
            br#"{"rt_cd":"0","output1":null,"output2":[]}"#.as_slice(),
            br#"{"rt_cd":"1","output1":{},"output2":[]}"#.as_slice(),
        ] {
            assert!(exact_original_rows(bytes, &evidence()).is_err());
        }
        let body = original(vec![row("20260831")]);
        let mut file = evidence();
        file.request
            .query
            .iter_mut()
            .find(|(key, _)| key == "FID_ORG_ADJ_PRC")
            .unwrap()
            .1 = "0".into();
        assert_eq!(
            exact_original_rows(&body, &file).err(),
            Some(Error::InvalidEvidence)
        );
        file = evidence();
        file.request
            .query
            .iter_mut()
            .find(|(key, _)| key == "FID_INPUT_ISCD")
            .unwrap()
            .1 = "000020".into();
        assert_eq!(
            exact_original_rows(&body, &file).err(),
            Some(Error::InvalidIdentity)
        );
    }

    #[test]
    fn s5b_addition_requires_all_eleven_unique_target_rows_and_raw_equals_adjusted() {
        let target = day("2026-08-31");
        let mut rows = grid(&[target]);
        rows.reverse();
        assert_eq!(
            checked_addition(rows.clone(), target).unwrap(),
            grid(&[target])
        );
        for case in 0..4 {
            let mut changed = rows.clone();
            match case {
                0 => {
                    changed.pop();
                }
                1 => changed[1] = changed[0].clone(),
                2 => changed[0].session_date = day(BASE_END),
                _ => changed[0].adjusted_open = fp("10.000"),
            };
            assert!(checked_addition(changed, target).is_err());
        }
    }

    const GOLDEN_ROW: &str = concat!(
        "{\"instrument_id\":\"069500.KRX\",\"session_date\":\"2026-08-28\",",
        "\"raw_open\":\"10.00\",\"raw_high\":\"12.00\",\"raw_low\":\"9.00\",\"raw_close\":\"11.00\",",
        "\"raw_volume\":7,\"raw_trading_value\":\"70.00\",",
        "\"adjusted_open\":\"5.00\",\"adjusted_high\":\"6.00\",\"adjusted_low\":\"4.50\",\"adjusted_close\":\"5.50\"}\n"
    );

    fn adjusted_base() -> HistoricalPriceOnlyBar {
        let mut base = bar(BASE_END);
        base.adjusted_open = fp("5.00");
        base.adjusted_high = fp("6.00");
        base.adjusted_low = fp("4.50");
        base.adjusted_close = fp("5.50");
        base
    }

    #[test]
    fn s5b_ndjson_literal_preserves_historical_raw_adjusted_representation() {
        let base = adjusted_base();
        validate_bar(&base).unwrap();
        let bytes = encode_cumulative_bars(std::slice::from_ref(&base), &[], 1000).unwrap();
        assert_eq!(bytes, GOLDEN_ROW.as_bytes());
        assert_eq!(
            encode_cumulative_bars(&[], &[bar("2026-08-31")], 1000).unwrap(),
            GOLDEN_ROW
                .replace("2026-08-28", "2026-08-31")
                .replace("\"adjusted_open\":\"5.00\"", "\"adjusted_open\":\"10.00\"")
                .replace("\"adjusted_high\":\"6.00\"", "\"adjusted_high\":\"12.00\"")
                .replace("\"adjusted_low\":\"4.50\"", "\"adjusted_low\":\"9.00\"")
                .replace(
                    "\"adjusted_close\":\"5.50\"",
                    "\"adjusted_close\":\"11.00\""
                )
                .as_bytes()
        );
        assert_ne!(base.raw_open.bits(), base.adjusted_open.bits());
    }

    #[test]
    fn s5b_ndjson_exact_byte_ceiling_counts_terminal_lf() {
        let base = adjusted_base();
        let length = GOLDEN_ROW.len() as u64;
        assert_eq!(
            encode_cumulative_bars(std::slice::from_ref(&base), &[], length).unwrap(),
            GOLDEN_ROW.as_bytes()
        );
        assert_eq!(
            encode_cumulative_bars(&[base], &[], length - 1),
            Err(Error::LimitExceeded)
        );
        let mut writer = BoundedBytes {
            bytes: vec![],
            maximum: 0,
            exceeded: false,
        };
        assert!(writer.write_all(b"x").is_err());
        assert!(writer.exceeded);
        assert!(writer.bytes.is_empty());
    }

    #[test]
    fn s5b_cumulative_encoding_orders_instrument_then_date_and_rejects_duplicates() {
        let base_days = [day("2026-08-27"), day(BASE_END)];
        let mut base = grid(&base_days);
        base.reverse();
        let target = day("2026-08-31");
        let mut added = grid(&[target]);
        added.reverse();
        let bytes = encode_cumulative_bars(&base, &added, 100_000).unwrap();
        let rows = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 33);
        let expected = instruments()
            .unwrap()
            .into_iter()
            .flat_map(|instrument| {
                [base_days[0], base_days[1], target]
                    .into_iter()
                    .map(move |date| (instrument.to_string(), date.to_iso()))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter()
                .map(|r| (
                    r["instrument_id"].as_str().unwrap().to_owned(),
                    r["session_date"].as_str().unwrap().to_owned()
                ))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            encode_cumulative_bars(&base, &base, 100_000),
            Err(Error::SourceConflict)
        );
    }

    #[test]
    fn s5c_readback_canonical_rows_preserve_raw_and_adjusted_scales() {
        let rows = decode_stored_bars(GOLDEN_ROW.as_bytes(), 1).unwrap();
        assert_eq!(rows, [adjusted_base()]);
        assert_eq!(rows[0].raw_open.scale(), 2);
        assert_eq!(rows[0].adjusted_open.bits(), 500);
        assert_eq!(
            encode_cumulative_bars(&rows, &[], 1000).unwrap(),
            GOLDEN_ROW.as_bytes()
        );
    }

    #[test]
    fn s5c_readback_rejects_noncanonical_or_ambiguous_rows() {
        let cases = [
            GOLDEN_ROW.trim_end().to_owned(),
            format!("{GOLDEN_ROW}\n"),
            GOLDEN_ROW.replace("{\"instrument_id\"", "{ \"instrument_id\""),
            GOLDEN_ROW.replace("{\"instrument_id\"", "{\"unknown\":0,\"instrument_id\""),
            GOLDEN_ROW.replace(
                "\"raw_open\":\"10.00\"",
                "\"raw_open\":\"10.00\",\"raw_open\":\"10.00\"",
            ),
            GOLDEN_ROW.replace("\"raw_open\":\"10.00\"", "\"raw_open\":10.0"),
            GOLDEN_ROW.replace("\"raw_open\":\"10.00\"", "\"raw_open\":\"010.00\""),
            GOLDEN_ROW.replace("\"raw_low\":\"9.00\"", "\"raw_low\":\"19.00\""),
        ];
        for bytes in cases {
            assert!(decode_stored_bars(bytes.as_bytes(), 2).is_err());
        }
    }

    #[test]
    fn s5c_readback_row_budget_rejects_one_more_and_oversize_line() {
        assert!(decode_stored_bars(GOLDEN_ROW.as_bytes(), 1).is_ok());
        assert_eq!(
            decode_stored_bars(format!("{GOLDEN_ROW}{GOLDEN_ROW}").as_bytes(), 1),
            Err(Error::LimitExceeded)
        );
        assert_eq!(
            decode_stored_bars(GOLDEN_ROW.as_bytes(), 0),
            Err(Error::LimitExceeded)
        );
        let bytes = format!("{}\n", " ".repeat(16 * 1024 + 1));
        assert_eq!(
            decode_stored_bars(bytes.as_bytes(), 1),
            Err(Error::LimitExceeded)
        );
    }

    #[test]
    fn s5c_readback_rejects_any_changed_historical_representation() {
        let old = adjusted_base();
        let index = BTreeMap::from([((old.instrument_id.clone(), old.session_date), 0)]);
        assert_eq!(
            compare_preserved_rows(
                std::slice::from_ref(&old),
                std::slice::from_ref(&old),
                &index
            ),
            Ok(())
        );
        for field in 0..7 {
            let mut changed = old.clone();
            match field {
                0 => changed.raw_open = fp("10.0"),
                1 => changed.raw_volume += 1,
                2 => changed.raw_trading_value = None,
                3 => changed.adjusted_open = fp("5.0"),
                4 => changed.adjusted_high = fp("7.00"),
                5 => changed.adjusted_low = fp("4.00"),
                _ => changed.adjusted_close = fp("5.00"),
            }
            assert_eq!(
                compare_preserved_rows(std::slice::from_ref(&old), &[changed], &index),
                Err(Error::SourceConflict)
            );
        }
        assert_eq!(
            compare_preserved_rows(&[old], &[], &BTreeMap::new()),
            Err(Error::SourceConflict)
        );
    }
}
