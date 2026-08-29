//! Deterministic owner-only price/volume research signals for the fixed list.
//!
//! The signal is deliberately separate from candidate selection, PIT
//! publication, and execution.  [`PriceVolumeSignalSnapshot::verify`] checks
//! the self-contained structural contract.  Consumers that need source
//! authenticity must use [`PriceVolumeSignalSnapshot::verify_against`] (or
//! [`read_fixed_stock_price_beta_snapshot_against`]) with the independently
//! verified daily-bars artifact.  Factor values cannot be authenticated from
//! a snapshot hash alone.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use market_data::{
    DailyBar, FIXED_30_INSTRUMENT_IDS, FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256,
    FixedStockPriceBetaArtifact,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID: &str = "fixed-stock-price-beta-signal-snapshot";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION: u32 = 1;
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION: &str = "fixed-stock-price-beta-factors-v1";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID: &str = "kr-stock-price-beta-v1";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE: &str = "OWNER_ONLY";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY: &str = "PRICE_VOLUME_RESEARCH_ONLY";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS: &str = "CONFIGURED_FIXED_LIST";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP: &str = "NOT_EVALUATED";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL: &str =
    "Activity/liquidity proxy, not execution liquidity";
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING: &str = market_data::ORIGINAL_PRICE_WARNING;
/// Absolute tolerance used when checking recomputed IEEE-754 metrics.
///
/// JSON serialization uses the shortest round-trippable representation, so
/// values produced by the same artifact normally compare bit-for-bit.  This
/// small tolerance permits harmless platform/library last-bit variation while
/// rejecting ordinary tampering.
pub const FIXED_STOCK_PRICE_BETA_SIGNAL_FLOAT_TOLERANCE: f64 = 1.0e-12;

// Short names retained as public schema constants for callers of this module.
pub const FIXED_STOCK_PRICE_BETA_FACTOR_VERSION: &str =
    FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION;
pub const PRICE_VOLUME_SIGNAL_SCHEMA_ID: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID;
pub const PRICE_VOLUME_SIGNAL_SCHEMA_VERSION: u32 = FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION;
pub const PRICE_VOLUME_SIGNAL_AUDIENCE: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE;
pub const PRICE_VOLUME_SIGNAL_CAPABILITY: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY;
pub const PRICE_VOLUME_SIGNAL_SELECTION_BASIS: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS;
pub const PRICE_VOLUME_SIGNAL_INDEX_MEMBERSHIP: &str =
    FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP;
pub const PRICE_VOLUME_SIGNAL_ACTIVITY_LABEL: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL;
pub const PRICE_VOLUME_SIGNAL_WARNING: &str = FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING;

pub const WEIGHT_RETURN_20: f64 = 0.20;
pub const WEIGHT_RETURN_60: f64 = 0.30;
pub const WEIGHT_RETURN_120: f64 = 0.25;
pub const WEIGHT_TREND: f64 = 0.10;
pub const WEIGHT_ACTIVITY: f64 = 0.10;
pub const WEIGHT_DRAWDOWN: f64 = 0.05;
pub const BULLISH_RETURN_20_MIN: f64 = 0.02;
pub const BEARISH_RETURN_20_MAX: f64 = -0.02;
pub const BEARISH_DRAWDOWN_MAX: f64 = -0.15;
pub const BULLISH_VOLATILITY_120_MAX: f64 = 0.45;

pub use market_data::ORIGINAL_PRICE_WARNING;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchCondition {
    Bullish,
    Neutral,
    Bearish,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceVolumeSignalRow {
    pub instrument_id: String,
    pub return_20: f64,
    pub return_60: f64,
    pub return_120: f64,
    pub volatility_20: f64,
    pub volatility_60: f64,
    pub volatility_120: f64,
    pub max_drawdown_120: f64,
    pub sma_20: f64,
    pub sma_60: f64,
    pub average_volume_20: f64,
    /// Activity/liquidity proxy only; it is not execution liquidity.
    pub volume_ratio_20_60: f64,
    pub average_trading_value_20: f64,
    pub score: f64,
    pub rank: usize,
    pub condition: ResearchCondition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceVolumeSignalSnapshot {
    pub schema_id: String,
    pub schema_version: u32,
    pub factor_version: String,
    pub audience: String,
    pub capability: String,
    pub vendor_snapshot: bool,
    pub strict_pit: bool,
    pub universe_id: String,
    pub universe_file_sha256: String,
    pub selection_basis: String,
    pub index_membership: String,
    pub original_price: bool,
    pub warning: String,
    pub artifact_content_sha256: String,
    pub as_of: String,
    pub activity_label: String,
    pub rows: Vec<PriceVolumeSignalRow>,
    pub content_sha256: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PriceVolumeSignalError {
    #[error("invalid price/volume signal input: {0}")]
    Invalid(&'static str),
    #[error("snapshot is missing or tampered")]
    Tampered,
    #[error("immutable snapshot conflict")]
    Conflict,
    #[error("snapshot path is unsafe")]
    UnsafePath,
    #[error("platform cannot provide descriptor-safe snapshot I/O")]
    UnsupportedPlatform,
    #[error("I/O failure")]
    Io,
    #[error("serialization failure")]
    Serialize,
}

impl PriceVolumeSignalSnapshot {
    /// Computes a snapshot from one verified bars artifact.
    ///
    /// A 120-day return needs 121 observations: the start and end prices are
    /// both included.  Rows after `as_of` are excluded before every metric is
    /// calculated.
    pub fn compute(
        artifact: &FixedStockPriceBetaArtifact,
        as_of: &str,
    ) -> Result<Self, PriceVolumeSignalError> {
        artifact
            .verify()
            .map_err(|_| PriceVolumeSignalError::Invalid("unverified daily-bars artifact"))?;
        require_iso_session(as_of, &artifact.sessions)?;

        let mut rows = Vec::with_capacity(FIXED_30_INSTRUMENT_IDS.len());
        for id in FIXED_30_INSTRUMENT_IDS {
            let mut bars: Vec<&DailyBar> = artifact
                .bars
                .iter()
                .filter(|bar| bar.instrument_id == id && bar.date.as_str() <= as_of)
                .collect();
            bars.sort_by(|left, right| left.date.cmp(&right.date));
            if bars.len() < 121 {
                return Err(PriceVolumeSignalError::Invalid(
                    "insufficient history for 120-day factors",
                ));
            }
            rows.push(metrics(id, &bars)?);
        }
        assign_scores_and_ranks(&mut rows)?;

        let mut snapshot = Self {
            schema_id: FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID.to_owned(),
            schema_version: FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION,
            factor_version: FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION.to_owned(),
            audience: FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE.to_owned(),
            capability: FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY.to_owned(),
            vendor_snapshot: true,
            strict_pit: false,
            universe_id: FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID.to_owned(),
            universe_file_sha256: FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256.to_owned(),
            selection_basis: FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS.to_owned(),
            index_membership: FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP.to_owned(),
            original_price: true,
            warning: FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING.to_owned(),
            artifact_content_sha256: artifact.content_sha256.clone(),
            as_of: as_of.to_owned(),
            activity_label: FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL.to_owned(),
            rows,
            content_sha256: String::new(),
        };
        snapshot.content_sha256 = snapshot.compute_hash()?;
        Ok(snapshot)
    }

    /// Computes the canonical content hash, excluding the hash field itself.
    pub fn compute_hash(&self) -> Result<String, PriceVolumeSignalError> {
        let mut copy = self.clone();
        copy.content_sha256.clear();
        let bytes = serde_json::to_vec(&copy).map_err(|_| PriceVolumeSignalError::Serialize)?;
        Ok(hash(&bytes))
    }

    /// Verifies fields that are independently checkable from the snapshot.
    ///
    /// This method intentionally cannot authenticate raw factor inputs: an
    /// attacker who has the bars can alter both a factor and this snapshot's
    /// self-hash.  Use [`Self::verify_against`] before exposing or consuming a
    /// snapshot as an artifact-derived result.
    pub fn verify(&self) -> Result<(), PriceVolumeSignalError> {
        self.verify_structure()?;
        if self.compute_hash()? != self.content_sha256 {
            return Err(PriceVolumeSignalError::Tampered);
        }
        Ok(())
    }

    /// Verifies this snapshot against the exact, independently verified bars
    /// artifact from which it claims to have been computed.
    pub fn verify_against(
        &self,
        artifact: &FixedStockPriceBetaArtifact,
    ) -> Result<(), PriceVolumeSignalError> {
        artifact
            .verify()
            .map_err(|_| PriceVolumeSignalError::Tampered)?;
        self.verify()?;
        if self.artifact_content_sha256 != artifact.content_sha256
            || self.universe_id != artifact.universe_id
            || self.universe_file_sha256 != artifact.universe_file_sha256
        {
            return Err(PriceVolumeSignalError::Tampered);
        }

        let expected =
            Self::compute(artifact, &self.as_of).map_err(|_| PriceVolumeSignalError::Tampered)?;
        if self.rows.len() != expected.rows.len()
            || self.content_sha256.len() != 64
            || self.content_sha256 != self.compute_hash()?
        {
            return Err(PriceVolumeSignalError::Tampered);
        }
        for (actual, expected) in self.rows.iter().zip(expected.rows.iter()) {
            if !row_matches(actual, expected) {
                return Err(PriceVolumeSignalError::Tampered);
            }
        }
        Ok(())
    }

    fn verify_structure(&self) -> Result<(), PriceVolumeSignalError> {
        if self.schema_id != FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_ID
            || self.schema_version != FIXED_STOCK_PRICE_BETA_SIGNAL_SCHEMA_VERSION
            || self.factor_version != FIXED_STOCK_PRICE_BETA_SIGNAL_FACTOR_VERSION
            || self.audience != FIXED_STOCK_PRICE_BETA_SIGNAL_AUDIENCE
            || self.capability != FIXED_STOCK_PRICE_BETA_SIGNAL_CAPABILITY
            || !self.vendor_snapshot
            || self.strict_pit
            || self.universe_id != FIXED_STOCK_PRICE_BETA_SIGNAL_UNIVERSE_ID
            || self.universe_file_sha256 != FIXED_STOCK_PRICE_BETA_UNIVERSE_FILE_SHA256
            || self.selection_basis != FIXED_STOCK_PRICE_BETA_SIGNAL_SELECTION_BASIS
            || self.index_membership != FIXED_STOCK_PRICE_BETA_SIGNAL_INDEX_MEMBERSHIP
            || !self.original_price
            || self.warning != FIXED_STOCK_PRICE_BETA_SIGNAL_WARNING
            || self.activity_label != FIXED_STOCK_PRICE_BETA_SIGNAL_ACTIVITY_LABEL
            || !is_sha256(&self.artifact_content_sha256)
            || !is_sha256(&self.content_sha256)
        {
            return Err(PriceVolumeSignalError::Tampered);
        }
        if !is_iso_date(&self.as_of) || self.rows.len() != FIXED_30_INSTRUMENT_IDS.len() {
            return Err(PriceVolumeSignalError::Tampered);
        }

        let configured: BTreeSet<&str> = FIXED_30_INSTRUMENT_IDS.into_iter().collect();
        let mut ids = BTreeSet::new();
        for row in &self.rows {
            if !configured.contains(row.instrument_id.as_str())
                || !ids.insert(row.instrument_id.as_str())
                || !canonical_row(row)
                || condition(row) != row.condition
            {
                return Err(PriceVolumeSignalError::Tampered);
            }
        }
        if ids != configured {
            return Err(PriceVolumeSignalError::Tampered);
        }

        let scores = cross_section_scores(&self.rows)?;
        for (row, score) in self.rows.iter().zip(scores.iter()) {
            if !close_enough(row.score, *score) {
                return Err(PriceVolumeSignalError::Tampered);
            }
        }
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|left, right| {
            scores[*right].total_cmp(&scores[*left]).then_with(|| {
                self.rows[*left]
                    .instrument_id
                    .cmp(&self.rows[*right].instrument_id)
            })
        });
        for (position, expected_index) in order.into_iter().enumerate() {
            let row = &self.rows[position];
            let expected = &self.rows[expected_index];
            if row.instrument_id != expected.instrument_id || row.rank != position + 1 {
                return Err(PriceVolumeSignalError::Tampered);
            }
        }
        Ok(())
    }
}

fn require_iso_session(as_of: &str, sessions: &[String]) -> Result<(), PriceVolumeSignalError> {
    if !is_iso_date(as_of) || !sessions.iter().any(|session| session == as_of) {
        return Err(PriceVolumeSignalError::Invalid(
            "as_of is not a confirmed ISO session",
        ));
    }
    Ok(())
}

fn is_iso_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(|date| date.to_string() == value)
            .unwrap_or(false)
}

fn metrics(id: &str, bars: &[&DailyBar]) -> Result<PriceVolumeSignalRow, PriceVolumeSignalError> {
    let closes: Vec<f64> = bars.iter().map(|bar| bar.close as f64).collect();
    let volumes: Vec<f64> = bars.iter().map(|bar| bar.volume as f64).collect();
    let ret = |window: usize| closes[closes.len() - 1] / closes[closes.len() - 1 - window] - 1.0;
    let sma = |window: usize| closes[closes.len() - window..].iter().sum::<f64>() / window as f64;
    let average_volume =
        |window: usize| volumes[volumes.len() - window..].iter().sum::<f64>() / window as f64;
    let volatility = |window: usize| annualized_volatility(&closes[closes.len() - window - 1..]);
    let average_volume_20 = average_volume(20);
    let row = PriceVolumeSignalRow {
        instrument_id: id.to_owned(),
        return_20: ret(20),
        return_60: ret(60),
        return_120: ret(120),
        volatility_20: volatility(20),
        volatility_60: volatility(60),
        volatility_120: volatility(120),
        max_drawdown_120: drawdown(&closes[closes.len() - 120..]),
        sma_20: sma(20),
        sma_60: sma(60),
        average_volume_20,
        volume_ratio_20_60: average_volume_20 / average_volume(60),
        average_trading_value_20: bars[bars.len() - 20..]
            .iter()
            .map(|bar| bar.close as f64 * bar.volume as f64)
            .sum::<f64>()
            / 20.0,
        score: 0.0,
        rank: 0,
        condition: ResearchCondition::Neutral,
    };
    if !canonical_row(&row) {
        return Err(PriceVolumeSignalError::Invalid(
            "nonfinite or noncanonical factor",
        ));
    }
    Ok(PriceVolumeSignalRow {
        condition: condition(&row),
        ..row
    })
}

fn assign_scores_and_ranks(
    rows: &mut [PriceVolumeSignalRow],
) -> Result<(), PriceVolumeSignalError> {
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

fn cross_section_scores(rows: &[PriceVolumeSignalRow]) -> Result<Vec<f64>, PriceVolumeSignalError> {
    if rows.len() != FIXED_30_INSTRUMENT_IDS.len() {
        return Err(PriceVolumeSignalError::Tampered);
    }
    let return20: Vec<f64> = rows.iter().map(|row| row.return_20).collect();
    let return60: Vec<f64> = rows.iter().map(|row| row.return_60).collect();
    let return120: Vec<f64> = rows.iter().map(|row| row.return_120).collect();
    let trend: Vec<f64> = rows
        .iter()
        .map(|row| row.sma_20 / row.sma_60 - 1.0)
        .collect();
    let activity: Vec<f64> = rows.iter().map(|row| row.volume_ratio_20_60).collect();
    let drawdowns: Vec<f64> = rows.iter().map(|row| row.max_drawdown_120).collect();
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
        return Err(PriceVolumeSignalError::Tampered);
    }
    let scores: Vec<f64> = (0..rows.len())
        .map(|position| {
            WEIGHT_RETURN_20 * percentile(&return20, position)
                + WEIGHT_RETURN_60 * percentile(&return60, position)
                + WEIGHT_RETURN_120 * percentile(&return120, position)
                + WEIGHT_TREND * percentile(&trend, position)
                + WEIGHT_ACTIVITY * percentile(&activity, position)
                + WEIGHT_DRAWDOWN * percentile(&drawdowns, position)
        })
        .collect();
    if scores
        .iter()
        .any(|score| !canonical_number(*score) || !(0.0..=1.0).contains(score))
    {
        return Err(PriceVolumeSignalError::Tampered);
    }
    Ok(scores)
}

fn canonical_row(row: &PriceVolumeSignalRow) -> bool {
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
        && row.max_drawdown_120 <= 0.0
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

fn row_matches(actual: &PriceVolumeSignalRow, expected: &PriceVolumeSignalRow) -> bool {
    actual.instrument_id == expected.instrument_id
        && exact_number(actual.return_20, expected.return_20)
        && exact_number(actual.return_60, expected.return_60)
        && exact_number(actual.return_120, expected.return_120)
        && exact_number(actual.volatility_20, expected.volatility_20)
        && exact_number(actual.volatility_60, expected.volatility_60)
        && exact_number(actual.volatility_120, expected.volatility_120)
        && exact_number(actual.max_drawdown_120, expected.max_drawdown_120)
        && exact_number(actual.sma_20, expected.sma_20)
        && exact_number(actual.sma_60, expected.sma_60)
        && exact_number(actual.average_volume_20, expected.average_volume_20)
        && exact_number(actual.volume_ratio_20_60, expected.volume_ratio_20_60)
        && exact_number(
            actual.average_trading_value_20,
            expected.average_trading_value_20,
        )
        && exact_number(actual.score, expected.score)
        && actual.rank == expected.rank
        && actual.condition == expected.condition
}

fn exact_number(actual: f64, expected: f64) -> bool {
    canonical_number(actual) && canonical_number(expected) && actual.to_bits() == expected.to_bits()
}

fn close_enough(actual: f64, expected: f64) -> bool {
    canonical_number(actual)
        && canonical_number(expected)
        && (actual - expected).abs() <= FIXED_STOCK_PRICE_BETA_SIGNAL_FLOAT_TOLERANCE
}

fn annualized_volatility(prices: &[f64]) -> f64 {
    let returns: Vec<f64> = prices
        .windows(2)
        .map(|pair| (pair[1] / pair[0]).ln())
        .collect();
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

fn condition(row: &PriceVolumeSignalRow) -> ResearchCondition {
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Writes a structurally verified snapshot.  Callers that have the source
/// artifact should prefer [`write_fixed_stock_price_beta_snapshot_against`].
pub fn write_fixed_stock_price_beta_snapshot(
    root: &Path,
    snapshot: &PriceVolumeSignalSnapshot,
) -> Result<PathBuf, PriceVolumeSignalError> {
    snapshot.verify()?;
    let bytes = serde_json::to_vec(snapshot).map_err(|_| PriceVolumeSignalError::Serialize)?;
    write_snapshot_bytes(root, snapshot, &bytes)
}

/// Writes a snapshot only after independently recomputing it from `artifact`.
pub fn write_fixed_stock_price_beta_snapshot_against(
    root: &Path,
    snapshot: &PriceVolumeSignalSnapshot,
    artifact: &FixedStockPriceBetaArtifact,
) -> Result<PathBuf, PriceVolumeSignalError> {
    snapshot.verify_against(artifact)?;
    let bytes = serde_json::to_vec(snapshot).map_err(|_| PriceVolumeSignalError::Serialize)?;
    write_snapshot_bytes(root, snapshot, &bytes)
}

fn write_snapshot_bytes(
    root: &Path,
    snapshot: &PriceVolumeSignalSnapshot,
    bytes: &[u8],
) -> Result<PathBuf, PriceVolumeSignalError> {
    #[cfg(unix)]
    {
        unix::write(root, snapshot, bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, snapshot, bytes);
        Err(PriceVolumeSignalError::UnsupportedPlatform)
    }
}

/// Reads and structurally verifies a snapshot.  Source authenticity requires
/// [`read_fixed_stock_price_beta_snapshot_against`].
pub fn read_fixed_stock_price_beta_snapshot(
    root: &Path,
    content_sha256: &str,
) -> Result<PriceVolumeSignalSnapshot, PriceVolumeSignalError> {
    if !is_sha256(content_sha256) {
        return Err(PriceVolumeSignalError::UnsafePath);
    }
    #[cfg(unix)]
    {
        unix::read(root, content_sha256)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, content_sha256);
        Err(PriceVolumeSignalError::UnsupportedPlatform)
    }
}

/// Reads and verifies a snapshot against the exact bars artifact it claims to
/// derive from.  This is the source-authenticating reader for API use.
pub fn read_fixed_stock_price_beta_snapshot_against(
    root: &Path,
    content_sha256: &str,
    artifact: &FixedStockPriceBetaArtifact,
) -> Result<PriceVolumeSignalSnapshot, PriceVolumeSignalError> {
    let snapshot = read_fixed_stock_price_beta_snapshot(root, content_sha256)?;
    snapshot.verify_against(artifact)?;
    Ok(snapshot)
}

#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, RenameFlags, fstat, fsync, mkdirat, open, openat,
        renameat_with, statat, unlinkat,
    };
    use rustix::process::geteuid;
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static STAGE: AtomicU64 = AtomicU64::new(1);
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    const SNAPSHOT_NAME: &[u8] = b"snapshot.json";
    const MANIFEST_NAME: &[u8] = b"snapshot.sha256";

    fn err(error: rustix::io::Errno) -> PriceVolumeSignalError {
        if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
            PriceVolumeSignalError::UnsafePath
        } else if error == rustix::io::Errno::NOSYS
            || error == rustix::io::Errno::INVAL
            || error == rustix::io::Errno::NOTSUP
            || error == rustix::io::Errno::OPNOTSUPP
        {
            PriceVolumeSignalError::UnsupportedPlatform
        } else {
            PriceVolumeSignalError::Io
        }
    }

    fn mode(stat: &rustix::fs::Stat) -> u32 {
        Mode::from_raw_mode(stat.st_mode).bits() & 0o7777
    }

    fn check_dir(stat: &rustix::fs::Stat) -> Result<(), PriceVolumeSignalError> {
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
            || stat.st_uid != geteuid().as_raw()
            || mode(stat) != 0o700
        {
            Err(PriceVolumeSignalError::UnsafePath)
        } else {
            Ok(())
        }
    }

    fn check_file(stat: &rustix::fs::Stat) -> Result<(), PriceVolumeSignalError> {
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != geteuid().as_raw()
            || mode(stat) != 0o600
            || stat.st_nlink != 1
            || stat.st_size < 0
            || stat.st_size as usize > MAX_BYTES
        {
            Err(PriceVolumeSignalError::UnsafePath)
        } else {
            Ok(())
        }
    }

    fn components(path: &Path) -> Result<Vec<Vec<u8>>, PriceVolumeSignalError> {
        let bytes = path.as_os_str().as_bytes();
        if bytes.len() < 2 || bytes[0] != b'/' || bytes[1] == b'/' || bytes.ends_with(b"/") {
            return Err(PriceVolumeSignalError::UnsafePath);
        }
        let components: Vec<Vec<u8>> = bytes[1..]
            .split(|byte| *byte == b'/')
            .map(|component| component.to_vec())
            .collect();
        if components.iter().any(|component| {
            component.is_empty()
                || component == b"."
                || component == b".."
                || component.contains(&0)
        }) {
            Err(PriceVolumeSignalError::UnsafePath)
        } else {
            Ok(components)
        }
    }

    fn directory_entries(d: &impl AsFd) -> Result<Vec<Vec<u8>>, PriceVolumeSignalError> {
        use rustix::fs::{RawDir, SeekFrom, seek};
        use rustix::io::dup;
        use std::mem::MaybeUninit;

        seek(d, SeekFrom::Start(0)).map_err(err)?;
        let duplicate = dup(d).map_err(err)?;
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
        let mut raw = RawDir::new(&duplicate, &mut buffer);
        let mut entries = Vec::new();
        while let Some(entry) = raw.next() {
            let entry = entry.map_err(err)?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                entries.push(name.to_vec());
            }
        }
        entries.sort_unstable();
        Ok(entries)
    }

    fn reject_orphan_staging(d: &impl AsFd) -> Result<(), PriceVolumeSignalError> {
        if directory_entries(d)?
            .iter()
            .any(|name| name.first() == Some(&b'.'))
        {
            Err(PriceVolumeSignalError::Conflict)
        } else {
            Ok(())
        }
    }

    fn root(path: &Path) -> Result<OwnedFd, PriceVolumeSignalError> {
        let mut descriptor = open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?;
        for component in components(path)? {
            descriptor = openat(
                &descriptor,
                &component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(err)?;
        }
        check_dir(&fstat(&descriptor).map_err(err)?)?;
        reject_orphan_staging(&descriptor)?;
        Ok(descriptor)
    }

    fn leaf(parent: &impl AsFd, name: &[u8]) -> Result<OwnedFd, PriceVolumeSignalError> {
        let named = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
        check_dir(&named)?;
        let descriptor = openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(err)?;
        let opened = fstat(&descriptor).map_err(err)?;
        check_dir(&opened)?;
        if named.st_dev != opened.st_dev || named.st_ino != opened.st_ino {
            Err(PriceVolumeSignalError::UnsafePath)
        } else {
            Ok(descriptor)
        }
    }

    fn exact_leaf(d: &OwnedFd) -> Result<(), PriceVolumeSignalError> {
        if directory_entries(d)? != vec![SNAPSHOT_NAME.to_vec(), MANIFEST_NAME.to_vec()] {
            return Err(PriceVolumeSignalError::Tampered);
        }
        for name in [SNAPSHOT_NAME, MANIFEST_NAME] {
            let stat = statat(d, name, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
            check_file(&stat)?;
        }
        Ok(())
    }

    fn read_named(d: &impl AsFd, name: &[u8]) -> Result<Vec<u8>, PriceVolumeSignalError> {
        let named = statat(d, name, AtFlags::SYMLINK_NOFOLLOW).map_err(err)?;
        check_file(&named)?;
        let descriptor = openat(
            d,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(err)?;
        let mut file = std::fs::File::from(descriptor);
        let opened = fstat(&file).map_err(err)?;
        check_file(&opened)?;
        if named.st_dev != opened.st_dev || named.st_ino != opened.st_ino {
            return Err(PriceVolumeSignalError::UnsafePath);
        }
        let mut bytes = Vec::with_capacity(opened.st_size as usize);
        file.read_to_end(&mut bytes)
            .map_err(|_| PriceVolumeSignalError::Io)?;
        if bytes.len() != opened.st_size as usize {
            return Err(PriceVolumeSignalError::Tampered);
        }
        Ok(bytes)
    }

    fn payload(parent: &OwnedFd, content_sha256: &str) -> Result<Vec<u8>, PriceVolumeSignalError> {
        let descriptor = leaf(parent, content_sha256.as_bytes())?;
        exact_leaf(&descriptor)?;
        let snapshot = read_named(&descriptor, SNAPSHOT_NAME)?;
        let manifest = read_named(&descriptor, MANIFEST_NAME)?;
        if manifest != hash(&snapshot).as_bytes() {
            return Err(PriceVolumeSignalError::Tampered);
        }
        Ok(snapshot)
    }

    fn stage(parent: &impl AsFd) -> Result<(OwnedFd, Vec<u8>), PriceVolumeSignalError> {
        for _ in 0..128 {
            let name = format!(
                ".stage-{}-{}",
                std::process::id(),
                STAGE.fetch_add(1, Ordering::Relaxed)
            )
            .into_bytes();
            match mkdirat(parent, &name, Mode::from_raw_mode(0o700)) {
                Ok(()) => match leaf(parent, &name) {
                    Ok(descriptor) => {
                        fsync(parent).map_err(err)?;
                        return Ok((descriptor, name));
                    }
                    Err(error) => {
                        let _ = unlinkat(parent, &name, AtFlags::REMOVEDIR);
                        return Err(error);
                    }
                },
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(error) => return Err(err(error)),
            }
        }
        Err(PriceVolumeSignalError::Conflict)
    }

    fn put(parent: &impl AsFd, name: &[u8], bytes: &[u8]) -> Result<(), PriceVolumeSignalError> {
        let descriptor = openat(
            parent,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(err)?;
        let mut file = std::fs::File::from(descriptor);
        check_file(&fstat(&file).map_err(err)?)?;
        file.write_all(bytes)
            .map_err(|_| PriceVolumeSignalError::Io)?;
        file.sync_all().map_err(|_| PriceVolumeSignalError::Io)?;
        let stat = fstat(&file).map_err(err)?;
        check_file(&stat)?;
        if stat.st_size as usize != bytes.len() {
            return Err(PriceVolumeSignalError::Io);
        }
        Ok(())
    }

    fn cleanup(parent: &impl AsFd, stage: &OwnedFd, name: &[u8]) {
        let names = directory_entries(stage).unwrap_or_default();
        if names
            .iter()
            .all(|entry| entry == SNAPSHOT_NAME || entry == MANIFEST_NAME)
        {
            for entry in names {
                if let Ok(stat) = statat(stage, &entry, AtFlags::SYMLINK_NOFOLLOW)
                    && check_file(&stat).is_ok()
                {
                    let _ = unlinkat(stage, &entry, AtFlags::empty());
                }
            }
            if directory_entries(stage).is_ok_and(|entries| entries.is_empty()) {
                let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
            }
        }
        let _ = fsync(parent);
    }

    fn same_directory(a: &rustix::fs::Stat, b: &rustix::fs::Stat) -> bool {
        a.st_dev == b.st_dev && a.st_ino == b.st_ino && a.st_uid == b.st_uid && mode(a) == mode(b)
    }

    pub(super) fn write(
        path: &Path,
        snapshot: &PriceVolumeSignalSnapshot,
        bytes: &[u8],
    ) -> Result<PathBuf, PriceVolumeSignalError> {
        let parent = root(path)?;
        let name = snapshot.content_sha256.as_bytes();
        match statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => {
                let existing = match payload(&parent, snapshot.content_sha256.as_str()) {
                    Ok(bytes) => bytes,
                    Err(PriceVolumeSignalError::UnsafePath) => {
                        return Err(PriceVolumeSignalError::UnsafePath);
                    }
                    Err(_) => return Err(PriceVolumeSignalError::Conflict),
                };
                return if existing == bytes {
                    Ok(path.join(snapshot.content_sha256.as_str()))
                } else {
                    Err(PriceVolumeSignalError::Conflict)
                };
            }
            Err(error) if error == rustix::io::Errno::NOENT => {}
            Err(error) => return Err(err(error)),
        }

        let (stage, stage_name) = stage(&parent)?;
        let stage_stat = fstat(&stage).map_err(err)?;
        let manifest = hash(bytes).into_bytes();
        if let Err(error) = put(&stage, SNAPSHOT_NAME, bytes)
            .and_then(|_| put(&stage, MANIFEST_NAME, &manifest))
            .and_then(|_| exact_leaf(&stage))
            .and_then(|_| fsync(&stage).map_err(err))
        {
            cleanup(&parent, &stage, &stage_name);
            return Err(error);
        }
        let named_stage = match statat(&parent, &stage_name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if same_directory(&stat, &stage_stat) => stat,
            Ok(_) => {
                cleanup(&parent, &stage, &stage_name);
                return Err(PriceVolumeSignalError::UnsafePath);
            }
            Err(error) => {
                cleanup(&parent, &stage, &stage_name);
                return Err(err(error));
            }
        };
        if !same_directory(&fstat(&stage).map_err(err)?, &named_stage) {
            cleanup(&parent, &stage, &stage_name);
            return Err(PriceVolumeSignalError::UnsafePath);
        }
        match renameat_with(&parent, &stage_name, &parent, name, RenameFlags::NOREPLACE) {
            Ok(()) => {
                fsync(&parent).map_err(err)?;
                read(path, snapshot.content_sha256.as_str())
                    .map(|_| path.join(snapshot.content_sha256.as_str()))
            }
            Err(error) if error == rustix::io::Errno::EXIST => {
                cleanup(&parent, &stage, &stage_name);
                match payload(&parent, snapshot.content_sha256.as_str()) {
                    Ok(existing) if existing == bytes => {
                        Ok(path.join(snapshot.content_sha256.as_str()))
                    }
                    Err(PriceVolumeSignalError::UnsafePath) => {
                        Err(PriceVolumeSignalError::UnsafePath)
                    }
                    _ => Err(PriceVolumeSignalError::Conflict),
                }
            }
            Err(error) => {
                cleanup(&parent, &stage, &stage_name);
                Err(err(error))
            }
        }
    }

    pub(super) fn read(
        path: &Path,
        content_sha256: &str,
    ) -> Result<PriceVolumeSignalSnapshot, PriceVolumeSignalError> {
        let parent = root(path)?;
        let bytes = payload(&parent, content_sha256)?;
        let snapshot: PriceVolumeSignalSnapshot =
            serde_json::from_slice(&bytes).map_err(|_| PriceVolumeSignalError::Tampered)?;
        if snapshot.content_sha256 != content_sha256 {
            return Err(PriceVolumeSignalError::Tampered);
        }
        let canonical =
            serde_json::to_vec(&snapshot).map_err(|_| PriceVolumeSignalError::Tampered)?;
        if bytes != canonical {
            return Err(PriceVolumeSignalError::Tampered);
        }
        snapshot.verify()?;
        Ok(snapshot)
    }
}
