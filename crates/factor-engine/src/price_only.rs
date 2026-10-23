//! Bounded owner-beta, price-return-only factor snapshots.
//!
//! This is deliberately separate from [`crate::snapshot::FactorSnapshot`]:
//! its only production input is the owner-approved historical artifact, and
//! its provenance names the five approval pins instead of a curated dataset.

use std::collections::BTreeMap;

use domain::{ContentHash, InstrumentId, TradingDate};
use market_data::{
    ApprovedHistoricalPriceOnlyArtifact, HistoricalPriceOnlyArtifactPins, KR_ETF_CORE_SYMBOLS,
};
use serde::Serialize;

use crate::bars::PriceOnlyBars;
use crate::contract::{Factor, FactorContext, FactorError, FactorId};
use crate::factors::all_price_only_factors;
use crate::fundamentals::Fundamentals;
use crate::normalize::{NormalizePolicy, ZScorePolicy};
use crate::snapshot::{FactorRow, FrozenUniverse, NormalizationMeta};

/// Fixed provenance label for the sealed owner-beta input.
pub const PRICE_ONLY_INPUT_KIND: &str = "owner_beta_historical_price_only_v1";
/// The input is sufficient for price-return factors, but not liquidity.
pub const PRICE_ONLY_CAPABILITY: &str = "PRICE_RETURN_ONLY";

/// A deterministic snapshot derived solely from approved price-only bars.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceOnlyFactorSnapshot {
    pub input_kind: String,
    pub capability: String,
    pub as_of: TradingDate,
    pub candidate_content_sha256: String,
    pub artifact_manifest_sha256: String,
    pub stage5_manifest_sha256: String,
    pub action_manifest_sha256: String,
    pub approval_registry_sha256: String,
    pub factor_versions: BTreeMap<String, String>,
    pub normalization: NormalizationMeta,
    pub rows: Vec<FactorRow>,
    pub hash: ContentHash,
}

impl PriceOnlyFactorSnapshot {
    /// Canonical bytes covered by [`Self::hash`]. The hash itself is excluded.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, FactorError> {
        #[derive(Serialize)]
        struct CanonicalRow<'a> {
            date: &'a str,
            instrument: &'a str,
            factor: &'a str,
            raw: Option<f64>,
            normalized: Option<f64>,
        }
        #[derive(Serialize)]
        struct Canonical<'a> {
            input_kind: &'a str,
            capability: &'a str,
            as_of: &'a str,
            candidate_content_sha256: &'a str,
            artifact_manifest_sha256: &'a str,
            stage5_manifest_sha256: &'a str,
            action_manifest_sha256: &'a str,
            approval_registry_sha256: &'a str,
            factor_versions: &'a BTreeMap<String, String>,
            normalization: &'a NormalizationMeta,
            rows: Vec<CanonicalRow<'a>>,
        }
        let canonical = Canonical {
            input_kind: &self.input_kind,
            capability: &self.capability,
            as_of: &self.as_of.to_iso(),
            candidate_content_sha256: &self.candidate_content_sha256,
            artifact_manifest_sha256: &self.artifact_manifest_sha256,
            stage5_manifest_sha256: &self.stage5_manifest_sha256,
            action_manifest_sha256: &self.action_manifest_sha256,
            approval_registry_sha256: &self.approval_registry_sha256,
            factor_versions: &self.factor_versions,
            normalization: &self.normalization,
            rows: self
                .rows
                .iter()
                .map(|row| CanonicalRow {
                    date: &row.date,
                    instrument: &row.instrument,
                    factor: &row.factor,
                    raw: row.raw,
                    normalized: row.normalized,
                })
                .collect(),
        };
        serde_json::to_vec(&canonical).map_err(|error| FactorError::Serialize {
            detail: format!("canonical price-only snapshot: {error}"),
        })
    }

    pub fn compute_hash(&self) -> Result<ContentHash, FactorError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }
}

/// Builds a bounded snapshot from an owner-approved price-only artifact.
pub struct PriceOnlyFactorSnapshotBuilder<'a> {
    artifact: &'a ApprovedHistoricalPriceOnlyArtifact,
    as_of: TradingDate,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
}

impl<'a> PriceOnlyFactorSnapshotBuilder<'a> {
    /// Uses exactly the twelve close-only current factors and default
    /// fundamentals (which no price-only factor consumes).
    pub fn new(artifact: &'a ApprovedHistoricalPriceOnlyArtifact, as_of: TradingDate) -> Self {
        Self {
            artifact,
            as_of,
            factors: all_price_only_factors(),
            normalization: Box::new(ZScorePolicy::default()),
        }
    }

    /// Overrides the registry, rejecting duplicate ids without a panic.
    pub fn with_factors(mut self, factors: Vec<Box<dyn Factor>>) -> Result<Self, FactorError> {
        self.factors = crate::factors::registry_with(factors)?;
        Ok(self)
    }

    /// Overrides the normalization policy.
    pub fn with_normalization(mut self, normalization: Box<dyn NormalizePolicy>) -> Self {
        self.normalization = normalization;
        self
    }

    /// Computes a point-in-time snapshot. Rows after `as_of` are filtered by
    /// the input layer before any factor sees them.
    pub fn build(self) -> Result<PriceOnlyFactorSnapshot, FactorError> {
        let bars = PriceOnlyBars::from_approved(self.artifact, self.as_of)?;
        build_from_bars(bars, self.artifact.pins(), self.factors, self.normalization)
    }
}

fn fixed_universe() -> Result<FrozenUniverse, FactorError> {
    let instruments = KR_ETF_CORE_SYMBOLS
        .iter()
        .map(|symbol| InstrumentId::parse(&format!("{symbol}.KRX")).map_err(FactorError::Domain))
        .collect::<Result<Vec<_>, _>>()?;
    FrozenUniverse::from_instruments("owner-beta-price-only-fixed-etf11", instruments)
}

fn build_from_bars(
    bars: PriceOnlyBars,
    pins: &HistoricalPriceOnlyArtifactPins,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
) -> Result<PriceOnlyFactorSnapshot, FactorError> {
    let pins = PriceOnlyPins::from(pins);
    build_from_bars_and_pins(bars, pins, factors, normalization)
}

#[derive(Clone)]
struct PriceOnlyPins {
    candidate_content_sha256: String,
    artifact_manifest_sha256: String,
    stage5_manifest_sha256: String,
    action_manifest_sha256: String,
    approval_registry_sha256: String,
}

impl From<&HistoricalPriceOnlyArtifactPins> for PriceOnlyPins {
    fn from(pins: &HistoricalPriceOnlyArtifactPins) -> Self {
        Self {
            candidate_content_sha256: pins.candidate_content_sha256().as_str().to_owned(),
            artifact_manifest_sha256: pins.artifact_manifest_sha256().as_str().to_owned(),
            stage5_manifest_sha256: pins.stage5_manifest_sha256().as_str().to_owned(),
            action_manifest_sha256: pins.action_manifest_sha256().as_str().to_owned(),
            approval_registry_sha256: pins.approval_registry_sha256().as_str().to_owned(),
        }
    }
}

pub(crate) struct ComputedPriceOnlyRows {
    pub factor_versions: BTreeMap<String, String>,
    pub normalization: NormalizationMeta,
    pub rows: Vec<FactorRow>,
}

/// The shared close-only arithmetic has no approval pins or serialization.
pub(crate) fn compute_price_only_rows(
    bars: &PriceOnlyBars,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
) -> Result<ComputedPriceOnlyRows, FactorError> {
    for factor in &factors {
        for field in factor.required_fields() {
            if !bars.available_fields().contains(field) {
                return Err(FactorError::MissingField {
                    factor: factor.id().to_owned(),
                    field: field.as_str().to_owned(),
                });
            }
        }
    }

    let universe = fixed_universe()?;
    let fundamentals = Fundamentals::default();
    let context = FactorContext {
        as_of: bars.as_of(),
        universe: &universe,
        bars: bars.as_bars(),
        fundamentals: &fundamentals,
    };
    let mut raw: BTreeMap<(String, String), BTreeMap<FactorId, Option<f64>>> = BTreeMap::new();
    for factor in &factors {
        let frame = factor.compute(&context)?;
        for row in frame.rows {
            raw.entry((row.date.to_iso(), row.instrument.to_string()))
                .or_default()
                .insert(frame.factor.clone(), row.value);
        }
    }

    let mut factor_ids: Vec<FactorId> = factors
        .iter()
        .map(|factor| factor.id().to_owned())
        .collect();
    factor_ids.sort_unstable();
    let mut by_date: BTreeMap<String, BTreeMap<String, BTreeMap<FactorId, Option<f64>>>> =
        BTreeMap::new();
    for ((date, instrument), values) in raw {
        by_date.entry(date).or_default().insert(instrument, values);
    }
    let mut rows = Vec::new();
    for (date, instruments) in &by_date {
        let members: Vec<&String> = instruments.keys().collect();
        let mut normalized = BTreeMap::<FactorId, Vec<Option<f64>>>::new();
        for factor in &factor_ids {
            let values = members
                .iter()
                .map(|instrument| instruments[*instrument].get(factor).copied().flatten())
                .collect::<Vec<_>>();
            normalized.insert(factor.clone(), normalization.apply(&values));
        }
        for (index, instrument) in members.iter().enumerate() {
            for factor in &factor_ids {
                rows.push(FactorRow {
                    date: date.clone(),
                    instrument: (*instrument).clone(),
                    factor: factor.clone(),
                    raw: instruments[*instrument].get(factor).copied().flatten(),
                    normalized: normalized[factor][index],
                });
            }
        }
    }

    let factor_versions = factors
        .iter()
        .map(|factor| (factor.id().to_owned(), factor.version().to_string()))
        .collect();
    let normalization = NormalizationMeta {
        id: normalization.id().to_owned(),
        version: normalization.version().to_string(),
        params: normalization.params(),
    };
    Ok(ComputedPriceOnlyRows {
        factor_versions,
        normalization,
        rows,
    })
}

fn build_from_bars_and_pins(
    bars: PriceOnlyBars,
    pins: PriceOnlyPins,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
) -> Result<PriceOnlyFactorSnapshot, FactorError> {
    let ComputedPriceOnlyRows {
        factor_versions,
        normalization,
        rows,
    } = compute_price_only_rows(&bars, factors, normalization)?;
    let snapshot = PriceOnlyFactorSnapshot {
        input_kind: PRICE_ONLY_INPUT_KIND.to_owned(),
        capability: PRICE_ONLY_CAPABILITY.to_owned(),
        as_of: bars.as_of(),
        candidate_content_sha256: pins.candidate_content_sha256,
        artifact_manifest_sha256: pins.artifact_manifest_sha256,
        stage5_manifest_sha256: pins.stage5_manifest_sha256,
        action_manifest_sha256: pins.action_manifest_sha256,
        approval_registry_sha256: pins.approval_registry_sha256,
        factor_versions,
        normalization,
        rows,
        hash: ContentHash::from_bytes(b"placeholder"),
    };
    let hash = snapshot.compute_hash()?;
    Ok(PriceOnlyFactorSnapshot { hash, ..snapshot })
}

#[cfg(test)]
pub(crate) fn build_from_test_parts(
    bars: &[market_data::HistoricalPriceOnlyBar],
    as_of: TradingDate,
    pins: [&str; 5],
    factors: Vec<Box<dyn Factor>>,
) -> Result<PriceOnlyFactorSnapshot, FactorError> {
    let bars = PriceOnlyBars::from_test_parts(bars, as_of)?;
    build_from_bars_and_pins(
        bars,
        PriceOnlyPins {
            candidate_content_sha256: pins[0].to_owned(),
            artifact_manifest_sha256: pins[1].to_owned(),
            stage5_manifest_sha256: pins[2].to_owned(),
            action_manifest_sha256: pins[3].to_owned(),
            approval_registry_sha256: pins[4].to_owned(),
        },
        factors,
        Box::new(ZScorePolicy::default()),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, NaiveDate, Weekday};
    use domain::{FixedPoint, InstrumentId, TradingDate};
    use market_data::{HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS};

    use super::*;
    use crate::factors::{AvgValueFactor, ReturnFactor, TrendFactor, price_only_factor_ids};

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("test date")
    }

    fn fixed(value: i128) -> FixedPoint {
        FixedPoint::from_i128(value, 0).expect("test fixed point")
    }

    fn synthetic_weekday_sessions(count: usize) -> Vec<TradingDate> {
        let mut date = NaiveDate::from_ymd_opt(2020, 1, 1).expect("test start date");
        let mut sessions = Vec::with_capacity(count);
        while sessions.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions.push(
                    TradingDate::new(date.year(), date.month(), date.day())
                        .expect("weekday session"),
                );
            }
            date += Duration::days(1);
        }
        sessions
    }

    fn synthetic_trend_bars(sessions: &[TradingDate]) -> Vec<HistoricalPriceOnlyBar> {
        sessions
            .iter()
            .enumerate()
            .flat_map(|(session_index, session)| {
                KR_ETF_CORE_SYMBOLS
                    .iter()
                    .enumerate()
                    .map(move |(instrument_index, symbol)| {
                        let close = fixed(100 + session_index as i128 + instrument_index as i128);
                        HistoricalPriceOnlyBar {
                            instrument_id: InstrumentId::parse(&format!("{symbol}.KRX"))
                                .expect("ETF11 instrument"),
                            session_date: *session,
                            raw_open: close.clone(),
                            raw_high: close.clone(),
                            raw_low: close.clone(),
                            raw_close: close.clone(),
                            raw_volume: 1_000 + instrument_index as u64,
                            raw_trading_value: Some(fixed(10_000)),
                            adjusted_open: close.clone(),
                            adjusted_high: close.clone(),
                            adjusted_low: close.clone(),
                            adjusted_close: close,
                        }
                    })
            })
            .collect()
    }

    fn pins() -> [&'static str; 5] {
        [
            "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            "sha256:0000000000000000000000000000000000000000000000000000000000000002",
            "sha256:0000000000000000000000000000000000000000000000000000000000000003",
            "sha256:0000000000000000000000000000000000000000000000000000000000000004",
            "sha256:0000000000000000000000000000000000000000000000000000000000000005",
        ]
    }

    fn bars(last_date: &str, multiplier: i128) -> Vec<HistoricalPriceOnlyBar> {
        KR_ETF_CORE_SYMBOLS
            .iter()
            .enumerate()
            .flat_map(|(index, symbol)| {
                let instrument_id = InstrumentId::parse(&format!("{symbol}.KRX")).expect("id");
                let start = 100 + index as i128;
                [
                    HistoricalPriceOnlyBar {
                        instrument_id: instrument_id.clone(),
                        session_date: date("2020-01-01"),
                        raw_open: fixed(1),
                        raw_high: fixed(2),
                        raw_low: fixed(1),
                        raw_close: fixed(3),
                        raw_volume: 7,
                        raw_trading_value: Some(fixed(11)),
                        adjusted_open: fixed(start),
                        adjusted_high: fixed(start),
                        adjusted_low: fixed(start),
                        adjusted_close: fixed(start),
                    },
                    HistoricalPriceOnlyBar {
                        instrument_id,
                        session_date: date(last_date),
                        raw_open: fixed(999),
                        raw_high: fixed(999),
                        raw_low: fixed(999),
                        raw_close: fixed(999),
                        raw_volume: 999,
                        raw_trading_value: Some(fixed(999)),
                        adjusted_open: fixed(start * multiplier),
                        adjusted_high: fixed(start * multiplier),
                        adjusted_low: fixed(start * multiplier),
                        adjusted_close: fixed(start * multiplier),
                    },
                ]
            })
            .collect()
    }

    #[test]
    fn defaults_are_exactly_twelve_close_only_factors() {
        assert_eq!(price_only_factor_ids().len(), 12);
        assert!(
            !price_only_factor_ids()
                .iter()
                .any(|id| id == "avg_value_20")
        );
    }

    #[test]
    fn uses_adjusted_close_and_ignores_raw_close_and_trading_value() {
        let original = bars("2020-02-01", 2);
        let mut altered_raw = original.clone();
        for bar in &mut altered_raw {
            bar.raw_close = fixed(9_999_999);
            bar.raw_trading_value = None;
        }
        let factors: Vec<Box<dyn Factor>> = vec![Box::new(ReturnFactor::one_month())];
        let snapshot = build_from_test_parts(&original, date("2020-02-01"), pins(), factors)
            .expect("snapshot");
        let altered = build_from_test_parts(
            &altered_raw,
            date("2020-02-01"),
            pins(),
            vec![Box::new(ReturnFactor::one_month())],
        )
        .expect("snapshot");
        assert_eq!(snapshot.hash, altered.hash);
        let golden = snapshot
            .rows
            .iter()
            .find(|row| {
                row.instrument == "069500.KRX"
                    && row.date == "2020-02-01"
                    && row.factor == "return_1m"
            })
            .expect("golden return row");
        assert_eq!(golden.raw, Some(1.0));
    }

    #[test]
    fn exposes_only_close_and_rejects_liquidity_before_compute() {
        let input = PriceOnlyBars::from_test_parts(&bars("2020-02-01", 2), date("2020-02-01"))
            .expect("input");
        assert_eq!(input.available_fields(), &[crate::Field::CLOSE]);
        let error = build_from_test_parts(
            &bars("2020-02-01", 2),
            date("2020-02-01"),
            pins(),
            vec![Box::new(AvgValueFactor)],
        )
        .expect_err("liquidity needs unavailable trading value");
        assert_eq!(
            error,
            FactorError::MissingField {
                factor: "avg_value_20".to_owned(),
                field: "trading_value".to_owned(),
            }
        );
    }

    #[test]
    fn later_rows_do_not_change_an_earlier_snapshot_or_hash() {
        let early = bars("2020-02-01", 2);
        let mut with_later = early.clone();
        with_later.extend(
            bars("2020-03-01", 3)
                .into_iter()
                .filter(|bar| bar.session_date == date("2020-03-01")),
        );
        let first = build_from_test_parts(
            &early,
            date("2020-02-01"),
            pins(),
            vec![Box::new(ReturnFactor::one_month())],
        )
        .expect("snapshot");
        let second = build_from_test_parts(
            &with_later,
            date("2020-02-01"),
            pins(),
            vec![Box::new(ReturnFactor::one_month())],
        )
        .expect("snapshot");
        assert_eq!(first, second);
    }

    #[test]
    fn trend_50_and_200_at_t_are_unchanged_when_only_later_prices_change() {
        let sessions = synthetic_weekday_sessions(230);
        let as_of = sessions[205];
        let original_bars = synthetic_trend_bars(&sessions);
        let mut perturbed_bars = original_bars.clone();
        for (row_index, bar) in perturbed_bars.iter_mut().enumerate() {
            if bar.session_date <= as_of {
                continue;
            }
            let days_after_t = bar
                .session_date
                .as_naive_date()
                .signed_duration_since(as_of.as_naive_date())
                .num_days() as i128;
            let price =
                fixed(10_000 + days_after_t + (row_index % KR_ETF_CORE_SYMBOLS.len()) as i128);
            bar.raw_open = price.clone();
            bar.raw_high = price.clone();
            bar.raw_low = price.clone();
            bar.raw_close = price.clone();
            bar.adjusted_open = price.clone();
            bar.adjusted_high = price.clone();
            bar.adjusted_low = price.clone();
            bar.adjusted_close = price;
        }

        let factors = || {
            vec![
                Box::new(TrendFactor::new(50).expect("trend 50")) as Box<dyn Factor>,
                Box::new(TrendFactor::new(200).expect("trend 200")) as Box<dyn Factor>,
            ]
        };
        let original = build_from_test_parts(&original_bars, as_of, pins(), factors())
            .expect("original trend snapshot");
        let mut changed_pins = pins();
        changed_pins[0] = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let perturbed = build_from_test_parts(&perturbed_bars, as_of, changed_pins, factors())
            .expect("perturbed trend snapshot");

        let original_t_rows = original
            .rows
            .iter()
            .filter(|row| row.date == as_of.to_iso())
            .collect::<Vec<_>>();
        let perturbed_t_rows = perturbed
            .rows
            .iter()
            .filter(|row| row.date == as_of.to_iso())
            .collect::<Vec<_>>();
        for factor_id in ["trend_50", "trend_200"] {
            assert!(original_t_rows.iter().any(|row| {
                row.instrument == "069500.KRX" && row.factor == factor_id && row.raw.is_some()
            }));
        }
        assert_eq!(original_t_rows, perturbed_t_rows);
        assert_eq!(original.factor_versions, perturbed.factor_versions);
        assert_eq!(
            original.factor_versions,
            std::collections::BTreeMap::from([
                ("trend_200".to_owned(), "1.0.0".to_owned()),
                ("trend_50".to_owned(), "1.0.0".to_owned()),
            ])
        );
    }

    #[test]
    fn incomplete_or_non_session_as_of_fails_closed() {
        let mut incomplete = bars("2020-02-01", 2);
        incomplete.pop();
        assert!(PriceOnlyBars::from_test_parts(&incomplete, date("2020-02-01")).is_err());
        assert!(
            PriceOnlyBars::from_test_parts(&bars("2020-02-01", 2), date("2020-02-02")).is_err()
        );
    }

    #[test]
    fn canonical_hash_binds_every_approval_pin() {
        let baseline =
            build_from_test_parts(&bars("2020-02-01", 2), date("2020-02-01"), pins(), vec![])
                .expect("snapshot");
        for index in 0..5 {
            let mut changed = pins();
            changed[index] =
                "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
            let snapshot =
                build_from_test_parts(&bars("2020-02-01", 2), date("2020-02-01"), changed, vec![])
                    .expect("snapshot");
            assert_ne!(baseline.hash, snapshot.hash, "pin {index} must be bound");
        }
    }

    #[test]
    fn duplicate_factor_override_is_a_typed_error_not_a_panic() {
        let result = crate::factors::registry_with(vec![
            Box::new(ReturnFactor::one_month()),
            Box::new(ReturnFactor::one_month()),
        ]);
        assert!(matches!(result, Err(FactorError::InvalidDefinition { .. })));
    }
}
