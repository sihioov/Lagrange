//! Sealed v3 approval and complete ETF11 input validation for simulation v2.
//!
//! The prepared value owns the one approved artifact object so later
//! admission and signal evaluation can reuse those exact bytes without
//! reopening the artifact directory.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use domain::{ContentHash, FixedPoint, TradingDate};
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyBar,
    range_normalize::ExpectedRangeSessions,
};

use super::{V2ContractError, V2MarketSession, contract::approved_instruments};
use crate::owner_beta::simulation::{
    APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256, APPROVED_BARS_SHA256,
    APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_END_DATE, APPROVED_REGISTRY_SHA256,
    APPROVED_SESSION_COUNT, APPROVED_STAGE5_MANIFEST_SHA256, APPROVED_START_DATE,
};

const BAR_PRICE_SCALE: u8 = 4;
const ETF11_INSTRUMENT_COUNT: usize = 11;
// Source sequence commitment from market-data's fixed v3 approval contract.
// APPROVED_BARS_SHA256 instead commits to the serialized artifact bars.
const APPROVED_SOURCE_BARS_SHA256: &str =
    "sha256:20c750f0ca415073da37650ae2bb0c942a181b4c86f167defe95895e4499dcf2";

type SessionPrices = Vec<(String, String, String, u64)>;
type PricesBySession = BTreeMap<TradingDate, SessionPrices>;

/// Sanitized failures while preparing the one approved v3 artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum V2InputError {
    #[error("approved historical artifact could not be verified")]
    ApprovalRejected,
    #[error("approved historical artifact metadata is inconsistent")]
    MetadataInvalid,
    #[error("approved historical bars are incomplete or invalid")]
    BarsInvalid,
    #[error("simulation evaluation window is outside the ready approved range")]
    InvalidWindow,
    #[error("the approved prefix has no common-ready evaluation window")]
    InsufficientReadiness,
    #[error("v2 simulation contract validation failed")]
    ContractInvalid,
}

impl From<V2ContractError> for V2InputError {
    fn from(_: V2ContractError) -> Self {
        Self::ContractInvalid
    }
}

/// One approved v3 object after its complete fixed envelope, calendar,
/// inventory, and price contract has been checked.
///
/// Fields are private. A downstream admission can retain this value and call
/// the prepared-input run entrypoint without reopening the artifact root.
pub struct V2PreparedInput {
    pub(crate) approved: ApprovedHistoricalPriceOnlyArtifact,
    sessions: Vec<TradingDate>,
    prices: PricesBySession,
}

impl V2PreparedInput {
    /// The immutable calendar derived from the approved XKRX range.
    pub fn sessions(&self) -> &[TradingDate] {
        &self.sessions
    }

    /// Calls the fixed production v3 approval entrypoint exactly once.
    pub fn approve(artifact_root: &Path) -> Result<Self, V2InputError> {
        let approved = market_data::approve_historical_price_only_artifact(artifact_root)
            .map_err(|_| V2InputError::ApprovalRejected)?;
        Self::from_approved(approved)
    }

    pub(crate) fn from_approved(
        approved: ApprovedHistoricalPriceOnlyArtifact,
    ) -> Result<Self, V2InputError> {
        let expected_sessions = expected_approved_sessions()?;
        validate_approved_metadata(&approved, &expected_sessions)?;
        let prices = validate_and_index_bars(approved.bars(), &expected_sessions)?;
        Ok(Self {
            approved,
            sessions: expected_sessions,
            prices,
        })
    }

    pub(crate) fn market_session(
        &self,
        date: TradingDate,
    ) -> Result<V2MarketSession, V2InputError> {
        let prices = self
            .prices
            .get(&date)
            .cloned()
            .ok_or(V2InputError::BarsInvalid)?;
        V2MarketSession::new(&date.to_iso(), prices).map_err(Into::into)
    }
}

fn expected_approved_sessions() -> Result<Vec<TradingDate>, V2InputError> {
    let start =
        TradingDate::parse(APPROVED_START_DATE).map_err(|_| V2InputError::MetadataInvalid)?;
    let end = TradingDate::parse(APPROVED_END_DATE).map_err(|_| V2InputError::MetadataInvalid)?;
    let expected = ExpectedRangeSessions::approved_xkrx(start, end)
        .map_err(|_| V2InputError::MetadataInvalid)?;
    if expected.sessions.len() != APPROVED_SESSION_COUNT as usize
        || expected.sessions.first() != Some(&start)
        || expected.sessions.last() != Some(&end)
        || expected.sessions.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(V2InputError::MetadataInvalid);
    }
    Ok(expected.sessions)
}

fn validate_approved_metadata(
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    expected_sessions: &[TradingDate],
) -> Result<(), V2InputError> {
    let summary = artifact
        .v3_approval_summary()
        .ok_or(V2InputError::MetadataInvalid)?;
    let pins = artifact.pins();
    let expected_hash =
        |value: &str| ContentHash::parse(value).map_err(|_| V2InputError::MetadataInvalid);
    let expected_instruments = approved_instruments();
    let expected_source_bars = expected_hash(APPROVED_SOURCE_BARS_SHA256)?;
    let expected_bars = expected_hash(APPROVED_BARS_SHA256)?;
    let expected_candidate = expected_hash(APPROVED_CANDIDATE_CONTENT_SHA256)?;
    let expected_manifest = expected_hash(APPROVED_ARTIFACT_MANIFEST_SHA256)?;
    let expected_stage5 = expected_hash(APPROVED_STAGE5_MANIFEST_SHA256)?;
    let expected_actions = expected_hash(APPROVED_ACTION_MANIFEST_SHA256)?;
    let expected_registry = expected_hash(APPROVED_REGISTRY_SHA256)?;
    let expected_start =
        TradingDate::parse(APPROVED_START_DATE).map_err(|_| V2InputError::MetadataInvalid)?;
    let expected_end =
        TradingDate::parse(APPROVED_END_DATE).map_err(|_| V2InputError::MetadataInvalid)?;

    let facts = ArtifactMetadataFacts {
        pins: [
            pins.candidate_content_sha256(),
            pins.artifact_manifest_sha256(),
            pins.stage5_manifest_sha256(),
            pins.action_manifest_sha256(),
            pins.approval_registry_sha256(),
        ],
        summary_pins: [
            summary.candidate_content_sha256(),
            summary.artifact_manifest_sha256(),
            summary.price_manifest_line_sha256(),
            summary.action_manifest_line_sha256(),
        ],
        price_bars_sha256: summary.price_bars_sha256(),
        bars_sha256: summary.bars_sha256(),
        range_start: summary.range_start(),
        range_end: summary.range_end(),
        instruments: summary.instruments(),
        instrument_count: summary.instrument_count(),
        session_count: summary.session_count(),
        row_count: summary.row_count(),
        bars_row_count: summary.bars_row_count(),
        audience: summary.audience(),
        capability: summary.capability(),
        vendor_snapshot: summary.vendor_snapshot(),
        strict_pit: summary.strict_pit(),
        action_count: summary.action_count(),
    };
    if !metadata_contract_matches(
        facts,
        expected_sessions,
        &expected_instruments,
        [
            &expected_candidate,
            &expected_manifest,
            &expected_stage5,
            &expected_actions,
            &expected_registry,
        ],
        ApprovedBarHashes {
            price_source: &expected_source_bars,
            artifact: &expected_bars,
        },
        expected_start,
        expected_end,
    ) {
        return Err(V2InputError::MetadataInvalid);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ArtifactMetadataFacts<'a> {
    pins: [&'a ContentHash; 5],
    summary_pins: [&'a ContentHash; 4],
    price_bars_sha256: &'a ContentHash,
    bars_sha256: &'a ContentHash,
    range_start: TradingDate,
    range_end: TradingDate,
    instruments: &'a [String],
    instrument_count: usize,
    session_count: usize,
    row_count: usize,
    bars_row_count: usize,
    audience: &'a str,
    capability: &'a str,
    vendor_snapshot: bool,
    strict_pit: bool,
    action_count: usize,
}

struct ApprovedBarHashes<'a> {
    price_source: &'a ContentHash,
    artifact: &'a ContentHash,
}

fn metadata_contract_matches(
    facts: ArtifactMetadataFacts<'_>,
    expected_sessions: &[TradingDate],
    expected_instruments: &[String],
    expected_pins: [&ContentHash; 5],
    expected_bars: ApprovedBarHashes<'_>,
    expected_start: TradingDate,
    expected_end: TradingDate,
) -> bool {
    facts.pins == expected_pins
        && facts.summary_pins == [facts.pins[0], facts.pins[1], facts.pins[2], facts.pins[3]]
        && facts.price_bars_sha256 == expected_bars.price_source
        && facts.bars_sha256 == expected_bars.artifact
        && facts.range_start == expected_start
        && facts.range_end == expected_end
        && facts.instruments == expected_instruments
        && facts.instrument_count == ETF11_INSTRUMENT_COUNT
        && facts.session_count == APPROVED_SESSION_COUNT as usize
        && facts.row_count == APPROVED_SESSION_COUNT as usize * ETF11_INSTRUMENT_COUNT
        && facts.bars_row_count == facts.row_count
        && expected_sessions.len() == APPROVED_SESSION_COUNT as usize
        && expected_sessions.first() == Some(&expected_start)
        && expected_sessions.last() == Some(&expected_end)
        && expected_sessions.windows(2).all(|pair| pair[0] < pair[1])
        && approved_metadata_flags(
            facts.audience,
            facts.capability,
            facts.vendor_snapshot,
            facts.strict_pit,
            facts.action_count,
        )
}

fn approved_metadata_flags(
    audience: &str,
    capability: &str,
    vendor_snapshot: bool,
    strict_pit: bool,
    action_count: usize,
) -> bool {
    audience == "OWNER_ONLY"
        && capability == "PRICE_RETURN_ONLY"
        && vendor_snapshot
        && !strict_pit
        && action_count == 0
}

fn validate_and_index_bars(
    bars: &[HistoricalPriceOnlyBar],
    expected_sessions: &[TradingDate],
) -> Result<PricesBySession, V2InputError> {
    let expected_instruments = approved_instruments();
    let expected_instrument_set = expected_instruments
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_dates = expected_sessions.iter().copied().collect::<BTreeSet<_>>();
    let expected_row_count = expected_sessions
        .len()
        .checked_mul(expected_instruments.len())
        .ok_or(V2InputError::BarsInvalid)?;
    if expected_sessions.is_empty()
        || expected_dates.len() != expected_sessions.len()
        || expected_instruments.len() != ETF11_INSTRUMENT_COUNT
        || expected_instrument_set.len() != ETF11_INSTRUMENT_COUNT
        || bars.len() != expected_row_count
    {
        return Err(V2InputError::BarsInvalid);
    }

    let mut indexed = PricesBySession::new();
    let mut observed = BTreeSet::<(TradingDate, String)>::new();
    let mut previous: Option<(String, TradingDate)> = None;
    for bar in bars {
        let instrument = bar.instrument_id.as_str().to_owned();
        let observed_key = (bar.session_date, instrument.clone());
        let chronological_key = (instrument.clone(), bar.session_date);
        if !expected_dates.contains(&bar.session_date)
            || !expected_instrument_set.contains(&instrument)
            || !bar_is_valid(bar)
            || !observed.insert(observed_key)
            || previous
                .as_ref()
                .is_some_and(|prior| prior >= &chronological_key)
        {
            return Err(V2InputError::BarsInvalid);
        }
        previous = Some(chronological_key);
        let open = price4(&bar.raw_open)?;
        let close = price4(&bar.raw_close)?;
        indexed.entry(bar.session_date).or_default().push((
            instrument,
            open.as_str().to_owned(),
            close.as_str().to_owned(),
            bar.raw_volume,
        ));
    }

    for date in expected_sessions {
        let session = indexed.get(date).ok_or(V2InputError::BarsInvalid)?;
        if session.len() != ETF11_INSTRUMENT_COUNT
            || session
                .iter()
                .map(|(instrument, _, _, _)| instrument.as_str())
                .ne(expected_instruments.iter().map(String::as_str))
        {
            return Err(V2InputError::BarsInvalid);
        }
    }
    Ok(indexed)
}

fn bar_is_valid(bar: &HistoricalPriceOnlyBar) -> bool {
    let raw = [&bar.raw_open, &bar.raw_high, &bar.raw_low, &bar.raw_close];
    let adjusted = [
        &bar.adjusted_open,
        &bar.adjusted_high,
        &bar.adjusted_low,
        &bar.adjusted_close,
    ];
    let raw_valid = raw
        .iter()
        .all(|price| price.bits() > 0 && price.scale() <= BAR_PRICE_SCALE && price4(price).is_ok());
    let adjusted_valid = adjusted
        .iter()
        .all(|price| price.bits() > 0 && price.scale() <= BAR_PRICE_SCALE && price4(price).is_ok());
    raw_valid
        && adjusted_valid
        && raw
            .iter()
            .zip(adjusted)
            .all(|(raw, adjusted)| **raw == *adjusted)
        && bar.raw_low <= bar.raw_open
        && bar.raw_open <= bar.raw_high
        && bar.raw_low <= bar.raw_close
        && bar.raw_close <= bar.raw_high
}

fn price4(value: &FixedPoint) -> Result<super::super::Price4, V2InputError> {
    if value.bits() <= 0 || value.scale() > BAR_PRICE_SCALE {
        return Err(V2InputError::BarsInvalid);
    }
    let scaled = value
        .with_scale(BAR_PRICE_SCALE)
        .map_err(|_| V2InputError::BarsInvalid)?;
    super::super::Price4::parse(&scaled.to_string()).map_err(|_| V2InputError::BarsInvalid)
}

/// Date selection is separate from readiness. The first-ready index is the
/// first evaluation date whose preceding signal session is common-ready.
pub(crate) fn select_window(
    sessions: &[TradingDate],
    first_ready_start: usize,
    requested_start: Option<TradingDate>,
    requested_end: Option<TradingDate>,
) -> Result<(usize, usize), V2InputError> {
    if sessions.len() < 2 || first_ready_start >= sessions.len() {
        return Err(V2InputError::InsufficientReadiness);
    }
    let start = match requested_start {
        Some(date) => sessions.iter().position(|session| *session == date),
        None => Some(first_ready_start),
    }
    .ok_or(V2InputError::InvalidWindow)?;
    let end = match requested_end {
        Some(date) => sessions.iter().position(|session| *session == date),
        None => Some(sessions.len() - 1),
    }
    .ok_or(V2InputError::InvalidWindow)?;
    if start < first_ready_start || end <= start {
        return Err(V2InputError::InvalidWindow);
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, NaiveDate};
    use domain::{ContentHash, FixedPoint, InstrumentId};
    use market_data::{HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS};

    use crate::owner_beta::simulation::{
        APPROVED_ACTION_MANIFEST_SHA256, APPROVED_ARTIFACT_MANIFEST_SHA256, APPROVED_BARS_SHA256,
        APPROVED_CANDIDATE_CONTENT_SHA256, APPROVED_END_DATE, APPROVED_REGISTRY_SHA256,
        APPROVED_SESSION_COUNT, APPROVED_STAGE5_MANIFEST_SHA256, APPROVED_START_DATE,
    };

    use super::{
        APPROVED_SOURCE_BARS_SHA256, ApprovedBarHashes, ArtifactMetadataFacts, V2InputError,
        bar_is_valid, expected_approved_sessions, metadata_contract_matches, select_window,
        validate_and_index_bars,
    };

    fn weekday_dates(count: usize) -> Vec<domain::TradingDate> {
        let mut date = NaiveDate::from_ymd_opt(2020, 1, 1).expect("test date");
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            if !matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                out.push(
                    domain::TradingDate::new(date.year(), date.month(), date.day())
                        .expect("weekday"),
                );
            }
            date += Duration::days(1);
        }
        out
    }

    fn synthetic_bars(
        dates: &[domain::TradingDate],
        instruments: &[String],
    ) -> Vec<HistoricalPriceOnlyBar> {
        let price = FixedPoint::parse("100.0000").expect("price");
        instruments
            .iter()
            .flat_map(|instrument| {
                dates.iter().map(move |date| HistoricalPriceOnlyBar {
                    instrument_id: InstrumentId::parse(instrument).expect("instrument"),
                    session_date: *date,
                    raw_open: price,
                    raw_high: price,
                    raw_low: price,
                    raw_close: price,
                    raw_volume: 1_000,
                    raw_trading_value: None,
                    adjusted_open: price,
                    adjusted_high: price,
                    adjusted_low: price,
                    adjusted_close: price,
                })
            })
            .collect()
    }

    #[test]
    fn rejects_invalid_price_relationship_and_raw_adjusted_mismatch() {
        let date = weekday_dates(1);
        let instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<Vec<_>>();
        let mut instruments = instruments;
        instruments.sort();
        let mut bars = synthetic_bars(&date, &instruments);
        assert!(bar_is_valid(&bars[0]));

        bars[0].raw_high = FixedPoint::parse("99.0000").expect("price");
        assert!(!bar_is_valid(&bars[0]));
        bars[0].raw_high = FixedPoint::parse("100.0000").expect("price");
        bars[0].adjusted_close = FixedPoint::parse("99.0000").expect("price");
        assert!(!bar_is_valid(&bars[0]));

        bars[0].adjusted_close = FixedPoint::parse("100.0000").expect("price");
        bars[0].raw_open = FixedPoint::parse("0").expect("zero");
        bars[0].adjusted_open = FixedPoint::parse("0").expect("zero");
        bars[0].raw_low = FixedPoint::parse("0").expect("zero");
        bars[0].adjusted_low = FixedPoint::parse("0").expect("zero");
        assert!(!bar_is_valid(&bars[0]));

        bars[0].raw_open = FixedPoint::parse("100.0000").expect("price");
        bars[0].adjusted_open = FixedPoint::parse("100.0000").expect("price");
        bars[0].raw_low = FixedPoint::parse("100.0000").expect("price");
        bars[0].adjusted_low = FixedPoint::parse("100.0000").expect("price");
        bars[0].raw_close = FixedPoint::parse("100.00001").expect("excess scale");
        bars[0].adjusted_close = FixedPoint::parse("100.00001").expect("excess scale");
        bars[0].raw_high = FixedPoint::parse("100.00001").expect("excess scale");
        bars[0].adjusted_high = FixedPoint::parse("100.00001").expect("excess scale");
        assert!(!bar_is_valid(&bars[0]));
    }

    #[test]
    fn validates_complete_unique_sorted_inventory() {
        let dates = weekday_dates(2);
        let instruments = KR_ETF_CORE_SYMBOLS
            .iter()
            .map(|symbol| format!("{symbol}.KRX"))
            .collect::<Vec<_>>();
        let mut instruments = instruments;
        instruments.sort();
        let bars = synthetic_bars(&dates, &instruments);
        assert!(validate_and_index_bars(&bars, &dates).is_ok());

        let mut missing = bars.clone();
        missing.pop();
        assert_eq!(
            validate_and_index_bars(&missing, &dates),
            Err(V2InputError::BarsInvalid)
        );

        let mut duplicate = bars.clone();
        duplicate[1] = duplicate[0].clone();
        assert_eq!(
            validate_and_index_bars(&duplicate, &dates),
            Err(V2InputError::BarsInvalid)
        );

        let mut unsorted = bars;
        unsorted.swap(0, 1);
        assert_eq!(
            validate_and_index_bars(&unsorted, &dates),
            Err(V2InputError::BarsInvalid)
        );

        let mut unknown = synthetic_bars(&dates, &instruments);
        unknown[0].instrument_id = InstrumentId::parse("999999.KRX").expect("unknown ETF");
        assert_eq!(
            validate_and_index_bars(&unknown, &dates),
            Err(V2InputError::BarsInvalid)
        );

        let mut outside_range = synthetic_bars(&dates, &instruments);
        outside_range[0].session_date = weekday_dates(3)[2];
        assert_eq!(
            validate_and_index_bars(&outside_range, &dates),
            Err(V2InputError::BarsInvalid)
        );
    }

    #[test]
    fn approved_v3_metadata_rejects_schema_pin_count_and_rights_changes() {
        let candidate = ContentHash::parse(APPROVED_CANDIDATE_CONTENT_SHA256).expect("candidate");
        let manifest = ContentHash::parse(APPROVED_ARTIFACT_MANIFEST_SHA256).expect("manifest");
        let stage5 = ContentHash::parse(APPROVED_STAGE5_MANIFEST_SHA256).expect("stage5");
        let actions = ContentHash::parse(APPROVED_ACTION_MANIFEST_SHA256).expect("actions");
        let registry = ContentHash::parse(APPROVED_REGISTRY_SHA256).expect("registry");
        let source_bars_hash =
            ContentHash::parse(APPROVED_SOURCE_BARS_SHA256).expect("source bars");
        let bars_hash = ContentHash::parse(APPROVED_BARS_SHA256).expect("bars");
        let wrong = ContentHash::parse(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("wrong hash");
        let start = domain::TradingDate::parse(APPROVED_START_DATE).expect("start");
        let end = domain::TradingDate::parse(APPROVED_END_DATE).expect("end");
        let sessions = expected_approved_sessions().expect("approved embedded calendar");
        let instruments = super::approved_instruments();
        let facts = ArtifactMetadataFacts {
            pins: [&candidate, &manifest, &stage5, &actions, &registry],
            summary_pins: [&candidate, &manifest, &stage5, &actions],
            price_bars_sha256: &source_bars_hash,
            bars_sha256: &bars_hash,
            range_start: start,
            range_end: end,
            instruments: &instruments,
            instrument_count: instruments.len(),
            session_count: APPROVED_SESSION_COUNT as usize,
            row_count: APPROVED_SESSION_COUNT as usize * instruments.len(),
            bars_row_count: APPROVED_SESSION_COUNT as usize * instruments.len(),
            audience: "OWNER_ONLY",
            capability: "PRICE_RETURN_ONLY",
            vendor_snapshot: true,
            strict_pit: false,
            action_count: 0,
        };
        let matches = |facts| {
            metadata_contract_matches(
                facts,
                &sessions,
                &instruments,
                [&candidate, &manifest, &stage5, &actions, &registry],
                ApprovedBarHashes {
                    price_source: &source_bars_hash,
                    artifact: &bars_hash,
                },
                start,
                end,
            )
        };

        assert_ne!(source_bars_hash, bars_hash);
        assert!(matches(facts));

        // The approved source sequence and artifact serialization are separate
        // commitments. Reject either altered pin and either conflation.
        for (source, artifact) in [
            (&wrong, &bars_hash),
            (&source_bars_hash, &wrong),
            (&bars_hash, &bars_hash),
            (&source_bars_hash, &source_bars_hash),
            (&bars_hash, &source_bars_hash),
        ] {
            let changed = ArtifactMetadataFacts {
                price_bars_sha256: source,
                bars_sha256: artifact,
                ..facts
            };
            assert!(!matches(changed));
        }

        let mut changed = facts;
        changed.summary_pins[0] = &wrong;
        assert!(!matches(changed));

        let mut changed = facts;
        changed.row_count -= 1;
        assert!(!matches(changed));

        let mut changed = facts;
        changed.action_count = 1;
        assert!(!matches(changed));

        let mut changed = facts;
        changed.capability = "PRICE_RETURN_AND_LIQUIDITY";
        assert!(!matches(changed));
    }

    #[test]
    fn explicit_window_must_be_ready_and_contain_two_valuation_sessions() {
        let sessions = weekday_dates(6);
        assert_eq!(
            select_window(&sessions, 2, Some(sessions[1]), Some(sessions[4])),
            Err(V2InputError::InvalidWindow)
        );
        assert_eq!(
            select_window(&sessions, 2, Some(sessions[3]), Some(sessions[3])),
            Err(V2InputError::InvalidWindow)
        );
        assert_eq!(select_window(&sessions, 2, None, None), Ok((2, 5)));
    }
}
