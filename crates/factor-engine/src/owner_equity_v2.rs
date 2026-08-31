//! Pure dynamic price/volume factors for the owner-managed equity universe.
//!
//! The input is an already admitted generation candidate plus its exact
//! lifecycle and evidence pins.  This module performs no I/O and does not
//! know about persistence or orchestration.  A daily `TradingDate` is the
//! snapshot key: a candidate must contain that exact observation, and only
//! observations on or before it are used for factor math.

use std::collections::BTreeSet;

use domain::{
    ContentHash, InstrumentId, MINIMUM_OBSERVED_SESSIONS, OwnerEquityAdmissionPins,
    OwnerEquityGeneration, OwnerEquityMembershipState, OwnerEquityUniverseHash, TradingDate, Venue,
};
use market_data::owner_equity_v2::{
    OWNER_EQUITY_V2_CANDIDATE_VERSION, OWNER_EQUITY_V2_CONTRACT_VERSION, OWNER_ONLY_WARNING,
    OwnerEquityBar, OwnerEquityGenerationCandidate, OwnerEquitySourcePins, PRICE_SEMANTICS,
    RESEARCH_ONLY_WARNING, STRICT_PIT_WARNING, VENDOR_SNAPSHOT_WARNING,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fixed_stock_price_beta::{
    BEARISH_DRAWDOWN_MAX, BEARISH_RETURN_20_MAX, BULLISH_RETURN_20_MIN, BULLISH_VOLATILITY_120_MAX,
    ResearchCondition,
};

/// Stable schema identifier for the dynamic owner-equity signal candidate.
pub const OWNER_EQUITY_V2_SIGNAL_SCHEMA_ID: &str = "owner-equity-v2-signal-snapshot";
/// Schema version of [`OwnerEquitySignalSnapshotCandidate`].
pub const OWNER_EQUITY_V2_SIGNAL_SCHEMA_VERSION: u32 = 1;
/// Version of the factor equations and scoring policy.
pub const OWNER_EQUITY_V2_SIGNAL_FACTOR_VERSION: &str = "owner-equity-v2-price-volume-factors-v1";
/// Stable identifier for the owner-managed universe product.
pub const OWNER_EQUITY_V2_SIGNAL_UNIVERSE_ID: &str = "owner-managed-equity-universe-v2";
/// Audience limitation carried into every candidate.
pub const OWNER_EQUITY_V2_SIGNAL_AUDIENCE: &str = "OWNER_ONLY";
/// Capability limitation carried into every candidate.
pub const OWNER_EQUITY_V2_SIGNAL_CAPABILITY: &str = "PRICE_VOLUME_RESEARCH_ONLY";
/// Human-readable activity field limitation.
pub const OWNER_EQUITY_V2_SIGNAL_ACTIVITY_LABEL: &str =
    "Activity/liquidity proxy, not execution liquidity";

const SCORE_RETURN_20_COEFFICIENT: f64 = 0.20;
const SCORE_RETURN_60_COEFFICIENT: f64 = 0.30;
const SCORE_RETURN_120_COEFFICIENT: f64 = 0.25;
const SCORE_TREND_COEFFICIENT: f64 = 0.10;
const SCORE_ACTIVITY_COEFFICIENT: f64 = 0.10;
const SCORE_DRAWDOWN_COEFFICIENT: f64 = 0.05;
const REQUIRED_LIMITATION_WARNINGS: [&str; 4] = [
    OWNER_ONLY_WARNING,
    VENDOR_SNAPSHOT_WARNING,
    STRICT_PIT_WARNING,
    RESEARCH_ONLY_WARNING,
];

/// Typed reason an admitted input does not contribute a row to this
/// snapshot.  The diagnostics are deliberately separate from the published
/// row set and are not serialized into its canonical bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnerEquityEligibilityReason {
    /// The membership is soft-disabled or otherwise inactive.
    Inactive,
    /// The membership has not reached the READY state.
    NotReady,
    /// The exact snapshot date is not present in the candidate observations.
    Stale,
    /// Fewer than the required 121 observations are available by `as_of`.
    InsufficientHistory,
}

/// One typed eligibility diagnostic for an input candidate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEquityExclusion {
    pub instrument_id: InstrumentId,
    pub reason: OwnerEquityEligibilityReason,
}

/// The exact input envelope consumed by the pure factor engine.
///
/// `admission_pins` must match the corresponding fields in
/// `candidate.source_pins`; `generation` is the positive owner-membership
/// generation that the later snapshot row will carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEquityAdmittedCandidate {
    /// Canonical membership instrument identity pinned outside the candidate.
    pub instrument_id: InstrumentId,
    pub active: bool,
    pub state: OwnerEquityMembershipState,
    pub generation: OwnerEquityGeneration,
    pub admission_pins: OwnerEquityAdmissionPins,
    pub candidate: OwnerEquityGenerationCandidate,
}

impl OwnerEquityAdmittedCandidate {
    /// Constructs an input envelope without weakening the validation done by
    /// [`OwnerEquitySignalSnapshotCandidate::compute`].
    pub fn new(
        candidate: OwnerEquityGenerationCandidate,
        generation: OwnerEquityGeneration,
        admission_pins: OwnerEquityAdmissionPins,
        active: bool,
        state: OwnerEquityMembershipState,
    ) -> Self {
        Self {
            instrument_id: candidate.instrument_id.clone(),
            active,
            state,
            generation,
            admission_pins,
            candidate,
        }
    }

    /// Convenience constructor for the normal active READY path.
    pub fn active_ready(
        candidate: OwnerEquityGenerationCandidate,
        generation: OwnerEquityGeneration,
        admission_pins: OwnerEquityAdmissionPins,
    ) -> Self {
        Self::new(
            candidate,
            generation,
            admission_pins,
            true,
            OwnerEquityMembershipState::Ready,
        )
    }
}

/// A row-level typed failure from structural validation or canonicalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OwnerEquitySnapshotError {
    #[error("owner equity snapshot contains a duplicate instrument")]
    DuplicateInstrument,
    #[error("owner equity candidate instrument differs from its envelope")]
    InstrumentMismatch,
    #[error("owner equity admission pins differ from candidate source pins")]
    AdmissionPinsMismatch,
    #[error("owner equity candidate contract is invalid")]
    CandidateContractInvalid,
    #[error("owner equity candidate coverage is invalid")]
    CandidateCoverageInvalid,
    #[error("owner equity candidate limitation contract is invalid")]
    CandidateSemanticsInvalid,
    #[error("owner equity factor value is nonfinite or out of bounds")]
    NumericInvalid,
    #[error("owner equity snapshot canonicalization failed")]
    CanonicalizationFailed,
    #[error("owner equity snapshot is structurally invalid")]
    SnapshotInvalid,
    #[error("owner equity snapshot content hash does not match")]
    SnapshotHashMismatch,
}

/// A deterministic row of dynamic owner-equity factors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEquitySignalRow {
    pub instrument_id: InstrumentId,
    pub generation: OwnerEquityGeneration,
    pub admission_pins: OwnerEquityAdmissionPins,
    pub source_pins: OwnerEquitySourcePins,
    pub return_20: f64,
    pub return_60: f64,
    pub return_120: f64,
    pub volatility_20: f64,
    pub volatility_60: f64,
    pub volatility_120: f64,
    pub max_drawdown_120: f64,
    pub sma_20: f64,
    pub sma_60: f64,
    pub trend_20_60: f64,
    pub average_volume_20: f64,
    pub volume_ratio_20_60: f64,
    pub average_trading_value_20: f64,
    pub score: f64,
    pub rank: usize,
    pub condition: ResearchCondition,
}

/// The pure deterministic dynamic snapshot candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerEquitySignalSnapshotCandidate {
    pub schema_id: String,
    pub schema_version: u32,
    pub factor_version: String,
    pub audience: String,
    pub capability: String,
    pub owner_only: bool,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub price_semantics: String,
    pub universe_id: String,
    pub universe_sha256: OwnerEquityUniverseHash,
    pub as_of: TradingDate,
    pub activity_label: String,
    pub warnings: Vec<String>,
    pub rows: Vec<OwnerEquitySignalRow>,
    /// Diagnostics are useful to the worker/operator but are not part of the
    /// published snapshot identity. Preparing or unavailable instruments must
    /// not alter the active-ready row set or its hash.
    #[serde(skip)]
    pub exclusions: Vec<OwnerEquityExclusion>,
    pub content_sha256: String,
}

/// Compatibility alias for callers that use the shorter snapshot name.
pub type OwnerEquitySignalSnapshot = OwnerEquitySignalSnapshotCandidate;
/// Compatibility alias emphasizing that this is a generated candidate.
pub type OwnerEquitySnapshotCandidate = OwnerEquitySignalSnapshotCandidate;
/// Compatibility alias for the admitted generation input envelope.
pub type OwnerEquityAdmittedGeneration = OwnerEquityAdmittedCandidate;

impl OwnerEquitySignalSnapshotCandidate {
    /// Computes a candidate from active/admitted generation inputs.
    pub fn compute(
        inputs: &[OwnerEquityAdmittedCandidate],
        as_of: TradingDate,
    ) -> Result<Self, OwnerEquitySnapshotError> {
        let mut seen = BTreeSet::new();
        let mut ordered = inputs.iter().collect::<Vec<_>>();
        for input in &ordered {
            validate_input(input)?;
            if !seen.insert(input.instrument_id.clone()) {
                return Err(OwnerEquitySnapshotError::DuplicateInstrument);
            }
        }
        ordered.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));

        let mut rows = Vec::new();
        let mut exclusions = Vec::new();
        for input in ordered {
            if !input.active {
                exclusions.push(OwnerEquityExclusion {
                    instrument_id: input.instrument_id.clone(),
                    reason: OwnerEquityEligibilityReason::Inactive,
                });
                continue;
            }
            if input.state != OwnerEquityMembershipState::Ready {
                exclusions.push(OwnerEquityExclusion {
                    instrument_id: input.instrument_id.clone(),
                    reason: OwnerEquityEligibilityReason::NotReady,
                });
                continue;
            }

            let bars = bars_through(&input.candidate.bars, as_of);
            if !bars.iter().any(|bar| bar.session_date == as_of) {
                exclusions.push(OwnerEquityExclusion {
                    instrument_id: input.instrument_id.clone(),
                    reason: OwnerEquityEligibilityReason::Stale,
                });
                continue;
            }
            if bars.len() < MINIMUM_OBSERVED_SESSIONS as usize {
                exclusions.push(OwnerEquityExclusion {
                    instrument_id: input.instrument_id.clone(),
                    reason: OwnerEquityEligibilityReason::InsufficientHistory,
                });
                continue;
            }
            rows.push(metrics(input, &bars)?);
        }

        assign_scores_and_ranks(&mut rows)?;
        exclusions.sort();
        let universe_sha256 =
            OwnerEquityUniverseHash::from_active_ready(rows.iter().map(|row| &row.instrument_id))
                .map_err(|_| OwnerEquitySnapshotError::SnapshotInvalid)?;
        let mut snapshot = Self {
            schema_id: OWNER_EQUITY_V2_SIGNAL_SCHEMA_ID.to_owned(),
            schema_version: OWNER_EQUITY_V2_SIGNAL_SCHEMA_VERSION,
            factor_version: OWNER_EQUITY_V2_SIGNAL_FACTOR_VERSION.to_owned(),
            audience: OWNER_EQUITY_V2_SIGNAL_AUDIENCE.to_owned(),
            capability: OWNER_EQUITY_V2_SIGNAL_CAPABILITY.to_owned(),
            owner_only: true,
            vendor_snapshot: true,
            strict_pit: false,
            price_semantics: PRICE_SEMANTICS.to_owned(),
            universe_id: OWNER_EQUITY_V2_SIGNAL_UNIVERSE_ID.to_owned(),
            universe_sha256,
            as_of,
            activity_label: OWNER_EQUITY_V2_SIGNAL_ACTIVITY_LABEL.to_owned(),
            warnings: REQUIRED_LIMITATION_WARNINGS
                .iter()
                .map(|warning| (*warning).to_owned())
                .collect(),
            rows,
            exclusions: Vec::new(),
            content_sha256: String::new(),
        };

        // Cross the exact serde boundary before hashing, matching the V1
        // snapshot behavior and making the bytes/hash stable after readback.
        let canonical = serde_json::to_vec(&snapshot)
            .map_err(|_| OwnerEquitySnapshotError::CanonicalizationFailed)?;
        snapshot = serde_json::from_slice(&canonical)
            .map_err(|_| OwnerEquitySnapshotError::CanonicalizationFailed)?;
        assign_scores_and_ranks(&mut snapshot.rows)?;
        snapshot.content_sha256 = snapshot.compute_hash()?.as_str().to_owned();
        snapshot.exclusions = exclusions;
        snapshot.verify()?;
        Ok(snapshot)
    }

    /// Alias for [`Self::compute`].
    pub fn build(
        inputs: &[OwnerEquityAdmittedCandidate],
        as_of: TradingDate,
    ) -> Result<Self, OwnerEquitySnapshotError> {
        Self::compute(inputs, as_of)
    }

    /// Serializes the complete canonical snapshot, including its self-hash.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OwnerEquitySnapshotError> {
        serde_json::to_vec(self).map_err(|_| OwnerEquitySnapshotError::CanonicalizationFailed)
    }

    /// Computes the SHA-256 hash over canonical snapshot bytes with the
    /// self-hash field cleared.
    pub fn compute_hash(&self) -> Result<ContentHash, OwnerEquitySnapshotError> {
        let mut copy = self.clone();
        copy.content_sha256.clear();
        let bytes = serde_json::to_vec(&copy)
            .map_err(|_| OwnerEquitySnapshotError::CanonicalizationFailed)?;
        Ok(ContentHash::from_bytes(&bytes))
    }

    /// Returns the expected self-hash for this candidate.
    pub fn content_sha256(&self) -> Result<ContentHash, OwnerEquitySnapshotError> {
        self.compute_hash()
    }

    /// Verifies the self-contained structural, numeric, lineage and ranking
    /// invariants of this candidate.
    pub fn verify(&self) -> Result<(), OwnerEquitySnapshotError> {
        self.verify_structure()?;
        if self.compute_hash()?.as_str() != self.content_sha256 {
            return Err(OwnerEquitySnapshotError::SnapshotHashMismatch);
        }
        Ok(())
    }

    /// Typed diagnostics for inputs excluded from the canonical row set.
    pub fn exclusions(&self) -> &[OwnerEquityExclusion] {
        &self.exclusions
    }

    fn verify_structure(&self) -> Result<(), OwnerEquitySnapshotError> {
        if self.schema_id != OWNER_EQUITY_V2_SIGNAL_SCHEMA_ID
            || self.schema_version != OWNER_EQUITY_V2_SIGNAL_SCHEMA_VERSION
            || self.factor_version != OWNER_EQUITY_V2_SIGNAL_FACTOR_VERSION
            || self.audience != OWNER_EQUITY_V2_SIGNAL_AUDIENCE
            || self.capability != OWNER_EQUITY_V2_SIGNAL_CAPABILITY
            || !self.owner_only
            || !self.vendor_snapshot
            || self.strict_pit
            || self.price_semantics != PRICE_SEMANTICS
            || self.universe_id != OWNER_EQUITY_V2_SIGNAL_UNIVERSE_ID
            || self.activity_label != OWNER_EQUITY_V2_SIGNAL_ACTIVITY_LABEL
            || self.warnings != REQUIRED_LIMITATION_WARNINGS
        {
            return Err(OwnerEquitySnapshotError::SnapshotInvalid);
        }
        if !canonical_hash(&self.content_sha256) {
            return Err(OwnerEquitySnapshotError::SnapshotInvalid);
        }

        let mut ids = BTreeSet::new();
        for (position, row) in self.rows.iter().enumerate() {
            validate_instrument(&row.instrument_id)?;
            if !ids.insert(row.instrument_id.clone())
                || row.rank != position + 1
                || !canonical_row(row)
                || condition(row) != row.condition
                || !pins_match(&row.admission_pins, &row.source_pins)
            {
                return Err(OwnerEquitySnapshotError::SnapshotInvalid);
            }
        }
        let expected_universe = OwnerEquityUniverseHash::from_active_ready(
            self.rows.iter().map(|row| &row.instrument_id),
        )
        .map_err(|_| OwnerEquitySnapshotError::SnapshotInvalid)?;
        if expected_universe != self.universe_sha256 {
            return Err(OwnerEquitySnapshotError::SnapshotInvalid);
        }

        let scores = cross_section_scores(&self.rows)?;
        let mut expected_order = (0..self.rows.len()).collect::<Vec<_>>();
        expected_order.sort_by(|left, right| {
            scores[*right].total_cmp(&scores[*left]).then_with(|| {
                self.rows[*left]
                    .instrument_id
                    .cmp(&self.rows[*right].instrument_id)
            })
        });
        for (position, expected_index) in expected_order.into_iter().enumerate() {
            if self.rows[position].instrument_id != self.rows[expected_index].instrument_id
                || self.rows[position].rank != position + 1
                || !same_number(self.rows[position].score, scores[position])
            {
                return Err(OwnerEquitySnapshotError::SnapshotInvalid);
            }
        }

        let mut exclusion_ids = BTreeSet::new();
        for exclusion in &self.exclusions {
            validate_instrument(&exclusion.instrument_id)?;
            if ids.contains(&exclusion.instrument_id)
                || !exclusion_ids.insert(exclusion.instrument_id.clone())
            {
                return Err(OwnerEquitySnapshotError::SnapshotInvalid);
            }
        }
        if self.exclusions.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(OwnerEquitySnapshotError::SnapshotInvalid);
        }
        Ok(())
    }
}

/// Builds a dynamic owner-equity snapshot candidate.
pub fn build_owner_equity_signal_snapshot(
    inputs: &[OwnerEquityAdmittedCandidate],
    as_of: TradingDate,
) -> Result<OwnerEquitySignalSnapshotCandidate, OwnerEquitySnapshotError> {
    OwnerEquitySignalSnapshotCandidate::compute(inputs, as_of)
}

/// Alias emphasizing that the operation is a pure computation.
pub fn compute_owner_equity_signal_snapshot(
    inputs: &[OwnerEquityAdmittedCandidate],
    as_of: TradingDate,
) -> Result<OwnerEquitySignalSnapshotCandidate, OwnerEquitySnapshotError> {
    OwnerEquitySignalSnapshotCandidate::compute(inputs, as_of)
}

fn validate_input(input: &OwnerEquityAdmittedCandidate) -> Result<(), OwnerEquitySnapshotError> {
    validate_instrument(&input.instrument_id)?;
    if input.candidate.instrument_id != input.instrument_id {
        return Err(OwnerEquitySnapshotError::InstrumentMismatch);
    }
    if input.candidate.candidate_version != OWNER_EQUITY_V2_CANDIDATE_VERSION
        || input.candidate.contract_version != OWNER_EQUITY_V2_CONTRACT_VERSION
        || input.candidate.minimum_observed_sessions < MINIMUM_OBSERVED_SESSIONS
        || input.candidate.target_observed_sessions < input.candidate.minimum_observed_sessions
    {
        return Err(OwnerEquitySnapshotError::CandidateContractInvalid);
    }
    if input.candidate.observed_sessions != input.candidate.bars.len() as u32
        || input.candidate.bars.is_empty()
        || input.candidate.first_observed_date
            != input
                .candidate
                .bars
                .first()
                .map(|bar| bar.session_date)
                .unwrap_or(input.candidate.first_observed_date)
        || input.candidate.last_observed_date
            != input
                .candidate
                .bars
                .last()
                .map(|bar| bar.session_date)
                .unwrap_or(input.candidate.last_observed_date)
        || input.candidate.requested_end < input.candidate.requested_start
    {
        return Err(OwnerEquitySnapshotError::CandidateCoverageInvalid);
    }
    let mut previous = None;
    for bar in &input.candidate.bars {
        if bar.session_date < input.candidate.requested_start
            || bar.session_date > input.candidate.requested_end
            || previous.is_some_and(|date| bar.session_date <= date)
            || bar.open == 0
            || bar.high == 0
            || bar.low == 0
            || bar.close == 0
            || bar.low > bar.high
            || bar.open < bar.low
            || bar.open > bar.high
            || bar.close < bar.low
            || bar.close > bar.high
        {
            return Err(OwnerEquitySnapshotError::CandidateCoverageInvalid);
        }
        previous = Some(bar.session_date);
    }
    if !input.candidate.owner_only
        || !input.candidate.vendor_snapshot
        || input.candidate.strict_pit
        || input.candidate.price_semantics != PRICE_SEMANTICS
        || REQUIRED_LIMITATION_WARNINGS
            .iter()
            .any(|warning| !input.candidate.warnings.iter().any(|item| item == warning))
    {
        return Err(OwnerEquitySnapshotError::CandidateSemanticsInvalid);
    }
    if !pins_match(&input.admission_pins, &input.candidate.source_pins) {
        return Err(OwnerEquitySnapshotError::AdmissionPinsMismatch);
    }
    Ok(())
}

fn validate_instrument(instrument: &InstrumentId) -> Result<(), OwnerEquitySnapshotError> {
    if instrument.venue() != Venue::Krx
        || instrument.symbol().len() != 6
        || !instrument
            .symbol()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        Err(OwnerEquitySnapshotError::CandidateContractInvalid)
    } else {
        Ok(())
    }
}

fn pins_match(admission: &OwnerEquityAdmissionPins, source: &OwnerEquitySourcePins) -> bool {
    admission.raw_manifest_sha256 == source.raw_manifest_sha256
        && admission.entitlement_sha256 == source.entitlement_sha256
        && admission.capture_code_commit == source.capture_code_commit
        && admission.materializer_code_commit == source.materializer_code_commit
}

fn bars_through(bars: &[OwnerEquityBar], as_of: TradingDate) -> Vec<&OwnerEquityBar> {
    bars.iter()
        .take_while(|bar| bar.session_date <= as_of)
        .collect()
}

fn metrics(
    input: &OwnerEquityAdmittedCandidate,
    bars: &[&OwnerEquityBar],
) -> Result<OwnerEquitySignalRow, OwnerEquitySnapshotError> {
    let closes = bars.iter().map(|bar| bar.close as f64).collect::<Vec<_>>();
    let volumes = bars.iter().map(|bar| bar.volume as f64).collect::<Vec<_>>();
    let end = closes.len() - 1;
    let trailing_return = |window: usize| closes[end] / closes[end - window] - 1.0;
    let sma = |window: usize| closes[closes.len() - window..].iter().sum::<f64>() / window as f64;
    let average_volume =
        |window: usize| volumes[volumes.len() - window..].iter().sum::<f64>() / window as f64;
    let volatility = |window: usize| annualized_volatility(&closes[closes.len() - window - 1..]);
    let average_volume_20 = average_volume(20);
    let average_volume_60 = average_volume(60);
    if average_volume_60 == 0.0 {
        return Err(OwnerEquitySnapshotError::NumericInvalid);
    }
    let sma_20 = sma(20);
    let sma_60 = sma(60);
    let row = OwnerEquitySignalRow {
        instrument_id: input.candidate.instrument_id.clone(),
        generation: input.generation,
        admission_pins: input.admission_pins.clone(),
        source_pins: input.candidate.source_pins.clone(),
        return_20: normalize_zero(trailing_return(20)),
        return_60: normalize_zero(trailing_return(60)),
        return_120: normalize_zero(trailing_return(120)),
        volatility_20: normalize_zero(volatility(20)),
        volatility_60: normalize_zero(volatility(60)),
        volatility_120: normalize_zero(volatility(120)),
        max_drawdown_120: normalize_zero(drawdown(&closes[closes.len() - 120..])),
        sma_20: normalize_zero(sma_20),
        sma_60: normalize_zero(sma_60),
        trend_20_60: normalize_zero(sma_20 / sma_60 - 1.0),
        average_volume_20: normalize_zero(average_volume_20),
        volume_ratio_20_60: normalize_zero(average_volume_20 / average_volume_60),
        average_trading_value_20: normalize_zero(
            bars[bars.len() - 20..]
                .iter()
                .map(|bar| bar.close as f64 * bar.volume as f64)
                .sum::<f64>()
                / 20.0,
        ),
        score: 0.0,
        rank: 0,
        condition: ResearchCondition::Neutral,
    };
    if !canonical_row(&row) {
        return Err(OwnerEquitySnapshotError::NumericInvalid);
    }
    Ok(OwnerEquitySignalRow {
        condition: condition(&row),
        ..row
    })
}

fn assign_scores_and_ranks(
    rows: &mut [OwnerEquitySignalRow],
) -> Result<(), OwnerEquitySnapshotError> {
    let scores = cross_section_scores(rows)?;
    for (row, score) in rows.iter_mut().zip(scores) {
        row.score = score;
    }
    rows.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.instrument_id.cmp(&right.instrument_id))
    });
    for (position, row) in rows.iter_mut().enumerate() {
        row.rank = position + 1;
    }
    Ok(())
}

fn cross_section_scores(
    rows: &[OwnerEquitySignalRow],
) -> Result<Vec<f64>, OwnerEquitySnapshotError> {
    let return20 = rows.iter().map(|row| row.return_20).collect::<Vec<_>>();
    let return60 = rows.iter().map(|row| row.return_60).collect::<Vec<_>>();
    let return120 = rows.iter().map(|row| row.return_120).collect::<Vec<_>>();
    let trend = rows.iter().map(|row| row.trend_20_60).collect::<Vec<_>>();
    let activity = rows
        .iter()
        .map(|row| row.volume_ratio_20_60)
        .collect::<Vec<_>>();
    let drawdowns = rows
        .iter()
        .map(|row| row.max_drawdown_120)
        .collect::<Vec<_>>();
    let factors = [
        &return20[..],
        &return60[..],
        &return120[..],
        &trend[..],
        &activity[..],
        &drawdowns[..],
    ];
    if factors
        .iter()
        .flat_map(|values| values.iter())
        .any(|value| !canonical_number(*value))
    {
        return Err(OwnerEquitySnapshotError::NumericInvalid);
    }
    let scores = (0..rows.len())
        .map(|position| {
            SCORE_RETURN_20_COEFFICIENT * percentile(&return20, position)
                + SCORE_RETURN_60_COEFFICIENT * percentile(&return60, position)
                + SCORE_RETURN_120_COEFFICIENT * percentile(&return120, position)
                + SCORE_TREND_COEFFICIENT * percentile(&trend, position)
                + SCORE_ACTIVITY_COEFFICIENT * percentile(&activity, position)
                + SCORE_DRAWDOWN_COEFFICIENT * percentile(&drawdowns, position)
        })
        .map(normalize_zero)
        .collect::<Vec<_>>();
    if scores
        .iter()
        .any(|score| !canonical_number(*score) || !(0.0..=1.0).contains(score))
    {
        return Err(OwnerEquitySnapshotError::NumericInvalid);
    }
    Ok(scores)
}

fn canonical_row(row: &OwnerEquitySignalRow) -> bool {
    [
        row.return_20,
        row.return_60,
        row.return_120,
        row.volatility_20,
        row.volatility_60,
        row.volatility_120,
        row.max_drawdown_120,
        row.sma_20,
        row.sma_60,
        row.trend_20_60,
        row.average_volume_20,
        row.volume_ratio_20_60,
        row.average_trading_value_20,
        row.score,
    ]
    .iter()
    .all(|value| canonical_number(*value))
        && row.return_20 > -1.0
        && row.return_60 > -1.0
        && row.return_120 > -1.0
        && row.volatility_20 >= 0.0
        && row.volatility_60 >= 0.0
        && row.volatility_120 >= 0.0
        && (-1.0..=0.0).contains(&row.max_drawdown_120)
        && row.sma_20 > 0.0
        && row.sma_60 > 0.0
        && row.average_volume_20 >= 0.0
        && row.volume_ratio_20_60 >= 0.0
        && row.average_trading_value_20 >= 0.0
        && (0.0..=1.0).contains(&row.score)
}

fn canonical_number(value: f64) -> bool {
    value.is_finite() && !(value == 0.0 && value.is_sign_negative())
}

fn same_number(left: f64, right: f64) -> bool {
    canonical_number(left) && canonical_number(right) && left.to_bits() == right.to_bits()
}

fn normalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn annualized_volatility(prices: &[f64]) -> f64 {
    let returns = prices
        .windows(2)
        .map(|pair| (pair[1] / pair[0]).ln())
        .collect::<Vec<_>>();
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / returns.len() as f64;
    variance.sqrt() * 252.0_f64.sqrt()
}

fn drawdown(prices: &[f64]) -> f64 {
    let mut peak = prices[0];
    prices.iter().fold(0.0_f64, |worst, price| {
        peak = peak.max(*price);
        worst.min(*price / peak - 1.0)
    })
}

fn percentile(values: &[f64], position: usize) -> f64 {
    values
        .iter()
        .filter(|value| **value <= values[position])
        .count() as f64
        / values.len() as f64
}

fn condition(row: &OwnerEquitySignalRow) -> ResearchCondition {
    if row.return_20 >= BULLISH_RETURN_20_MIN
        && row.sma_20 >= row.sma_60
        && row.volatility_120 <= BULLISH_VOLATILITY_120_MAX
    {
        ResearchCondition::Bullish
    } else if row.return_20 <= BEARISH_RETURN_20_MAX
        || (row.sma_20 < row.sma_60 && row.max_drawdown_120 <= BEARISH_DRAWDOWN_MAX)
    {
        ResearchCondition::Bearish
    } else {
        ResearchCondition::Neutral
    }
}

fn canonical_hash(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{BatchId, CodeCommit};
    use market_data::owner_equity_v2::{OwnerEquityCaptureKind, OwnerEquityRawFilePin};

    const AS_OF: &str = "2026-08-31";
    const COMMIT: &str = "abcdef0123456789abcdef0123456789abcdef01";

    fn as_of() -> TradingDate {
        TradingDate::parse(AS_OF).unwrap()
    }

    fn hash(label: &str) -> ContentHash {
        ContentHash::from_bytes(label.as_bytes())
    }

    fn pins(symbol: &str) -> (OwnerEquityAdmissionPins, OwnerEquitySourcePins) {
        let raw = hash(&format!("raw-{symbol}"));
        let entitlement = hash("owner-entitlement");
        let capture = CodeCommit::parse(COMMIT).unwrap();
        let materializer = CodeCommit::parse("fedcba9876543210fedcba9876543210fedcba98").unwrap();
        let admission = OwnerEquityAdmissionPins {
            raw_manifest_sha256: raw.clone(),
            artifact_manifest_sha256: hash(&format!("artifact-{symbol}")),
            entitlement_sha256: entitlement.clone(),
            capture_code_commit: capture.clone(),
            materializer_code_commit: materializer.clone(),
        };
        let source = OwnerEquitySourcePins {
            capture_identity_sha256: hash(&format!("identity-{symbol}")),
            raw_batch_id: "00000000-0000-0000-0000-000000000001"
                .parse::<BatchId>()
                .unwrap(),
            raw_manifest_sha256: raw,
            batch_json_sha256: hash(&format!("batch-{symbol}")),
            entitlement_reference: "owner-approved-fixture".to_owned(),
            entitlement_sha256: entitlement,
            capture_code_commit: capture,
            materializer_code_commit: materializer,
            prior_candidate_sha256: None,
            prior_artifact_manifest_sha256: None,
            files: vec![OwnerEquityRawFilePin {
                kind: market_data::ResponseKind::Bars,
                window_sequence: Some(1),
                file_name: format!("{symbol}-bars.json"),
                sha256: hash(&format!("file-{symbol}")),
                size_bytes: 1,
            }],
        };
        (admission, source)
    }

    fn candidate(
        symbol: &str,
        count: usize,
        slope: u64,
        end: TradingDate,
    ) -> OwnerEquityGenerationCandidate {
        let start = end.checked_add_days(-((count - 1) as i64)).unwrap();
        let bars = (0..count)
            .map(|offset| {
                let date = start.checked_add_days(offset as i64).unwrap();
                let close = 100 + offset as u64 * slope;
                OwnerEquityBar {
                    session_date: date,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    volume: 1_000,
                }
            })
            .collect::<Vec<_>>();
        let (_, source) = pins(symbol);
        OwnerEquityGenerationCandidate {
            candidate_version: OWNER_EQUITY_V2_CANDIDATE_VERSION.to_owned(),
            contract_version: OWNER_EQUITY_V2_CONTRACT_VERSION.to_owned(),
            capture_kind: OwnerEquityCaptureKind::Initial,
            instrument_id: InstrumentId::parse(&format!("{symbol}.KRX")).unwrap(),
            display_name: None,
            requested_start: start,
            requested_end: end,
            target_observed_sessions: 261,
            minimum_observed_sessions: 121,
            observed_sessions: count as u32,
            first_observed_date: start,
            last_observed_date: end,
            bars,
            source_pins: source,
            price_semantics: PRICE_SEMANTICS.to_owned(),
            owner_only: true,
            vendor_snapshot: true,
            strict_pit: false,
            warnings: REQUIRED_LIMITATION_WARNINGS
                .iter()
                .map(|warning| (*warning).to_owned())
                .collect(),
            claims_not_made: vec!["STRICT_POINT_IN_TIME".to_owned()],
        }
    }

    fn input(symbol: &str, count: usize, slope: u64) -> OwnerEquityAdmittedCandidate {
        let candidate = candidate(symbol, count, slope, as_of());
        let (admission, _) = pins(symbol);
        OwnerEquityAdmittedCandidate::active_ready(
            candidate,
            OwnerEquityGeneration::new(1).unwrap(),
            admission,
        )
    }

    fn ids(snapshot: &OwnerEquitySignalSnapshotCandidate) -> Vec<String> {
        snapshot
            .rows
            .iter()
            .map(|row| row.instrument_id.to_string())
            .collect()
    }

    #[test]
    fn numeric_bounds_and_edge_values_are_explicit() {
        let snapshot =
            OwnerEquitySignalSnapshotCandidate::compute(&[input("000001", 121, 0)], as_of())
                .unwrap();
        let row = &snapshot.rows[0];
        assert_eq!(row.return_20, 0.0);
        assert_eq!(row.return_60, 0.0);
        assert_eq!(row.return_120, 0.0);
        assert_eq!(row.volatility_20, 0.0);
        assert_eq!(row.volatility_120, 0.0);
        assert_eq!(row.max_drawdown_120, 0.0);
        assert_eq!(row.trend_20_60, 0.0);
        assert_eq!(row.volume_ratio_20_60, 1.0);
        assert_eq!(row.score, 1.0);
        assert!(canonical_row(row));

        let mut zero_volume = input("000002", 121, 1);
        zero_volume
            .candidate
            .bars
            .iter_mut()
            .for_each(|bar| bar.volume = 0);
        assert_eq!(
            OwnerEquitySignalSnapshotCandidate::compute(&[zero_volume], as_of()),
            Err(OwnerEquitySnapshotError::NumericInvalid)
        );
    }

    #[test]
    fn exact_121_is_eligible_and_120_is_typed_insufficient_history() {
        let eligible =
            OwnerEquitySignalSnapshotCandidate::compute(&[input("000003", 121, 1)], as_of())
                .unwrap();
        assert_eq!(eligible.rows.len(), 1);
        assert!(eligible.exclusions.is_empty());

        let insufficient =
            OwnerEquitySignalSnapshotCandidate::compute(&[input("000004", 120, 1)], as_of())
                .unwrap();
        assert!(insufficient.rows.is_empty());
        assert_eq!(
            insufficient.exclusions,
            vec![OwnerEquityExclusion {
                instrument_id: InstrumentId::parse("000004.KRX").unwrap(),
                reason: OwnerEquityEligibilityReason::InsufficientHistory,
            }]
        );
    }

    #[test]
    fn missing_exact_as_of_is_typed_stale_and_not_ready_is_ignored() {
        let stale_end = as_of().previous_day();
        let stale_candidate = candidate("000005", 121, 1, stale_end);
        let (stale_admission, _) = pins("000005");
        let stale = OwnerEquityAdmittedCandidate::active_ready(
            stale_candidate,
            OwnerEquityGeneration::new(1).unwrap(),
            stale_admission,
        );
        let mut not_ready = input("000006", 121, 100);
        not_ready.state = OwnerEquityMembershipState::Backfilling;
        let stale_only =
            OwnerEquitySignalSnapshotCandidate::compute(std::slice::from_ref(&stale), as_of())
                .unwrap();
        let with_not_ready =
            OwnerEquitySignalSnapshotCandidate::compute(&[stale, not_ready], as_of()).unwrap();
        assert!(with_not_ready.rows.is_empty());
        assert_eq!(
            with_not_ready.universe_sha256,
            OwnerEquityUniverseHash::from_active_ready(std::iter::empty()).unwrap()
        );
        assert_eq!(
            with_not_ready.exclusions[0].reason,
            OwnerEquityEligibilityReason::Stale
        );
        assert_eq!(
            with_not_ready.exclusions[1].reason,
            OwnerEquityEligibilityReason::NotReady
        );
        assert_eq!(
            stale_only.canonical_bytes().unwrap(),
            with_not_ready.canonical_bytes().unwrap()
        );
    }

    #[test]
    fn active_ready_cardinality_is_dynamic_from_one_to_one_hundred() {
        for count in [1usize, 2, 31, 100] {
            let inputs = (0..count)
                .map(|index| input(&format!("{:06}", index + 10), 121, (index + 1) as u64))
                .collect::<Vec<_>>();
            let snapshot = OwnerEquitySignalSnapshotCandidate::compute(&inputs, as_of())
                .unwrap_or_else(|error| panic!("count {count}: {error:?}"));
            assert_eq!(snapshot.rows.len(), count);
            assert_eq!(
                snapshot.rows.iter().map(|row| row.rank).collect::<Vec<_>>(),
                (1..=count).collect::<Vec<_>>()
            );
            assert_eq!(
                snapshot.universe_sha256,
                OwnerEquityUniverseHash::from_active_ready(
                    snapshot.rows.iter().map(|row| &row.instrument_id)
                )
                .unwrap()
            );
        }
    }

    #[test]
    fn permutation_is_byte_and_hash_idempotent() {
        let inputs = (0..4)
            .map(|index| input(&format!("{:06}", index + 20), 121, (index + 1) as u64))
            .collect::<Vec<_>>();
        let first = OwnerEquitySignalSnapshotCandidate::compute(&inputs, as_of()).unwrap();
        let mut shuffled = inputs.clone();
        shuffled.reverse();
        let second = OwnerEquitySignalSnapshotCandidate::compute(&shuffled, as_of()).unwrap();
        assert_eq!(
            first.canonical_bytes().unwrap(),
            second.canonical_bytes().unwrap()
        );
        assert_eq!(first.content_sha256, second.content_sha256);
        assert_eq!(ids(&first), ids(&second));
    }

    #[test]
    fn equal_scores_are_tied_by_canonical_instrument_id() {
        let inputs = vec![
            input("000032", 121, 1),
            input("000030", 121, 1),
            input("000031", 121, 1),
        ];
        let snapshot = OwnerEquitySignalSnapshotCandidate::compute(&inputs, as_of()).unwrap();
        assert_eq!(
            ids(&snapshot),
            vec!["000030.KRX", "000031.KRX", "000032.KRX"]
        );
        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| row.score)
                .collect::<Vec<_>>(),
            vec![1.0; 3]
        );
        assert_eq!(
            snapshot.rows.iter().map(|row| row.rank).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn adding_or_removing_an_eligible_instrument_reranks_the_exact_set() {
        let first = input("000040", 121, 1);
        let second = input("000041", 121, 2);
        let third = input("000042", 121, 3);
        let pair =
            OwnerEquitySignalSnapshotCandidate::compute(&[first.clone(), second.clone()], as_of())
                .unwrap();
        let triple =
            OwnerEquitySignalSnapshotCandidate::compute(&[first, second, third], as_of()).unwrap();
        assert_eq!(ids(&pair), vec!["000041.KRX", "000040.KRX"]);
        assert_eq!(ids(&triple), vec!["000042.KRX", "000041.KRX", "000040.KRX"]);
        assert_eq!(pair.rows[0].rank, 1);
        assert_eq!(triple.rows[1].rank, 2);
        assert_ne!(pair.content_sha256, triple.content_sha256);

        let removed = OwnerEquitySignalSnapshotCandidate::compute(
            &[input("000040", 121, 1), input("000042", 121, 3)],
            as_of(),
        )
        .unwrap();
        assert_eq!(ids(&removed), vec!["000042.KRX", "000040.KRX"]);
        assert_eq!(
            removed.rows.iter().map(|row| row.rank).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn invalid_duplicate_and_mismatched_inputs_are_rejected() {
        let duplicate = input("000050", 121, 1);
        assert_eq!(
            OwnerEquitySignalSnapshotCandidate::compute(&[duplicate.clone(), duplicate], as_of()),
            Err(OwnerEquitySnapshotError::DuplicateInstrument)
        );

        let mut instrument_mismatch = input("000051", 121, 1);
        instrument_mismatch.candidate.instrument_id = InstrumentId::parse("000052.KRX").unwrap();
        assert_eq!(
            OwnerEquitySignalSnapshotCandidate::compute(&[instrument_mismatch], as_of()),
            Err(OwnerEquitySnapshotError::InstrumentMismatch)
        );

        let mut pins_mismatch = input("000053", 121, 1);
        pins_mismatch.admission_pins.raw_manifest_sha256 = hash("different-raw");
        assert_eq!(
            OwnerEquitySignalSnapshotCandidate::compute(&[pins_mismatch], as_of()),
            Err(OwnerEquitySnapshotError::AdmissionPinsMismatch)
        );
    }

    #[test]
    fn future_observations_do_not_change_as_of_factors() {
        let baseline = input("000060", 121, 1);
        let mut future_candidate =
            candidate("000060", 126, 1, as_of().checked_add_days(5).unwrap());
        future_candidate.source_pins = baseline.candidate.source_pins.clone();
        let (admission, _) = pins("000060");
        let future = OwnerEquityAdmittedCandidate::active_ready(
            future_candidate,
            OwnerEquityGeneration::new(1).unwrap(),
            admission,
        );
        let baseline_snapshot =
            OwnerEquitySignalSnapshotCandidate::compute(&[baseline], as_of()).unwrap();
        let future_snapshot =
            OwnerEquitySignalSnapshotCandidate::compute(&[future], as_of()).unwrap();
        assert_eq!(
            baseline_snapshot.rows[0].return_120,
            future_snapshot.rows[0].return_120
        );
        assert_eq!(
            baseline_snapshot.rows[0].score,
            future_snapshot.rows[0].score
        );
        assert_eq!(
            baseline_snapshot.canonical_bytes().unwrap(),
            future_snapshot.canonical_bytes().unwrap()
        );
    }

    #[test]
    fn golden_canonical_snapshot_hash() {
        let snapshot = OwnerEquitySignalSnapshotCandidate::compute(
            &[input("000070", 121, 1), input("000071", 121, 2)],
            as_of(),
        )
        .unwrap();
        assert_eq!(
            snapshot.content_sha256,
            "sha256:2e3f4bb3a0a369a682e045a6c0375396d2c0292532df3a0479f04c28205b91bf"
        );
    }

    #[test]
    fn serde_round_trip_preserves_canonical_snapshot() {
        let snapshot =
            OwnerEquitySignalSnapshotCandidate::compute(&[input("000080", 121, 1)], as_of())
                .unwrap();
        let bytes = snapshot.canonical_bytes().unwrap();
        let parsed: OwnerEquitySignalSnapshotCandidate = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.exclusions.is_empty());
        assert_eq!(parsed.canonical_bytes().unwrap(), bytes);
        parsed.verify().unwrap();
    }
}
