//! Daily factor snapshots have their own provenance and canonical preimage.

use std::collections::BTreeMap;

use domain::{ContentHash, TradingDate};
use market_data::daily_price_only::input::AdmittedDailyPriceInputV1;
use serde::Serialize;

use crate::bars::PriceOnlyBars;
use crate::contract::{Factor, FactorError};
use crate::factors::all_price_only_factors;
use crate::normalize::{NormalizePolicy, ZScorePolicy};
use crate::price_only::{PRICE_ONLY_CAPABILITY, compute_price_only_rows};
use crate::snapshot::{FactorRow, NormalizationMeta};

pub const DAILY_PRICE_ONLY_INPUT_KIND: &str = "owner_beta_daily_etf11_price_only_v1";
pub const DAILY_FACTOR_SNAPSHOT_SCHEMA: &str = "owner-beta-daily-factor-snapshot-v1";

/// Serializable computation evidence, never an admission or publication token.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyPriceOnlyFactorSnapshot {
    pub schema: String,
    pub input_kind: String,
    pub capability: String,
    pub as_of: TradingDate,
    pub content_sha256: String,
    pub manifest_sha256: String,
    pub admission_sha256: String,
    pub policy_sha256: String,
    pub factor_versions: BTreeMap<String, String>,
    pub normalization: NormalizationMeta,
    pub rows: Vec<FactorRow>,
    pub hash: ContentHash,
}

impl DailyPriceOnlyFactorSnapshot {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, FactorError> {
        for row in &self.rows {
            for value in [row.raw, row.normalized].into_iter().flatten() {
                if !value.is_finite() {
                    return Err(FactorError::NonFinite {
                        factor: row.factor.clone(),
                        instrument: row.instrument.clone(),
                        date: row.date.clone(),
                        value,
                    });
                }
            }
        }
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
            schema: &'a str,
            input_kind: &'a str,
            capability: &'a str,
            as_of: String,
            content_sha256: &'a str,
            manifest_sha256: &'a str,
            admission_sha256: &'a str,
            policy_sha256: &'a str,
            factor_versions: &'a BTreeMap<String, String>,
            normalization: &'a NormalizationMeta,
            rows: Vec<CanonicalRow<'a>>,
        }
        let value = Canonical {
            schema: &self.schema,
            input_kind: &self.input_kind,
            capability: &self.capability,
            as_of: self.as_of.to_iso(),
            content_sha256: &self.content_sha256,
            manifest_sha256: &self.manifest_sha256,
            admission_sha256: &self.admission_sha256,
            policy_sha256: &self.policy_sha256,
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
        serde_json::to_vec(&value).map_err(|_| FactorError::Serialize {
            detail: "canonical daily factor snapshot".to_owned(),
        })
    }

    pub fn compute_hash(&self) -> Result<ContentHash, FactorError> {
        Ok(ContentHash::from_bytes(&self.canonical_bytes()?))
    }
}

pub struct DailyPriceOnlyFactorSnapshotBuilder<'view, 'input, 'base> {
    input: &'view AdmittedDailyPriceInputV1<'input, 'base>,
    as_of: TradingDate,
    factors: Vec<Box<dyn Factor>>,
    normalization: Box<dyn NormalizePolicy>,
}

impl<'view, 'input, 'base> DailyPriceOnlyFactorSnapshotBuilder<'view, 'input, 'base> {
    pub fn new(input: &'view AdmittedDailyPriceInputV1<'input, 'base>, as_of: TradingDate) -> Self {
        Self {
            input,
            as_of,
            factors: all_price_only_factors(),
            normalization: Box::new(ZScorePolicy::default()),
        }
    }

    pub fn with_factors(mut self, factors: Vec<Box<dyn Factor>>) -> Result<Self, FactorError> {
        self.factors = crate::factors::registry_with(factors)?;
        Ok(self)
    }

    pub fn with_normalization(mut self, normalization: Box<dyn NormalizePolicy>) -> Self {
        self.normalization = normalization;
        self
    }

    pub fn build(self) -> Result<DailyPriceOnlyFactorSnapshot, FactorError> {
        let bars = PriceOnlyBars::from_admitted_daily(self.input, self.as_of)?;
        let computed = compute_price_only_rows(&bars, self.factors, self.normalization)?;
        let pins = self.input.pins();
        let mut snapshot = DailyPriceOnlyFactorSnapshot {
            schema: DAILY_FACTOR_SNAPSHOT_SCHEMA.to_owned(),
            input_kind: DAILY_PRICE_ONLY_INPUT_KIND.to_owned(),
            capability: PRICE_ONLY_CAPABILITY.to_owned(),
            as_of: self.as_of,
            content_sha256: pins.content_sha256.as_str().to_owned(),
            manifest_sha256: pins.manifest_sha256.as_str().to_owned(),
            admission_sha256: pins.admission_sha256.as_str().to_owned(),
            policy_sha256: pins.policy_sha256.as_str().to_owned(),
            factor_versions: computed.factor_versions,
            normalization: computed.normalization,
            rows: computed.rows,
            hash: ContentHash::from_bytes(b"daily-factor-placeholder"),
        };
        snapshot.hash = snapshot.compute_hash()?;
        self.input
            .revalidate()
            .map_err(|_| FactorError::InvalidDefinition {
                detail: "sealed daily input changed".to_owned(),
            })?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use domain::{FixedPoint, InstrumentId};
    use market_data::{HistoricalPriceOnlyBar, KR_ETF_CORE_SYMBOLS};

    use super::*;
    use crate::factors::{AvgValueFactor, ReturnFactor};

    fn date(value: &str) -> TradingDate {
        TradingDate::parse(value).unwrap()
    }
    fn bars() -> Vec<HistoricalPriceOnlyBar> {
        KR_ETF_CORE_SYMBOLS
            .iter()
            .flat_map(|symbol| {
                [
                    ("2026-07-31", 100),
                    ("2026-08-31", 120),
                    ("2026-09-30", 300),
                ]
                .map(|(day, close)| {
                    let adjusted = FixedPoint::from_i128(close, 0).unwrap();
                    let raw = FixedPoint::from_i128(999, 0).unwrap();
                    HistoricalPriceOnlyBar {
                        instrument_id: InstrumentId::parse(&format!("{symbol}.KRX")).unwrap(),
                        session_date: date(day),
                        raw_open: raw,
                        raw_high: raw,
                        raw_low: raw,
                        raw_close: raw,
                        raw_volume: 999,
                        raw_trading_value: Some(raw),
                        adjusted_open: adjusted,
                        adjusted_high: adjusted,
                        adjusted_low: adjusted,
                        adjusted_close: adjusted,
                    }
                })
            })
            .collect()
    }

    #[test]
    fn i3_shared_factor_math_preserves_historical_rows_versions_and_normalization() {
        let rows = bars();
        let as_of = date("2026-08-31");
        let historical = crate::price_only::build_from_test_parts(
            &rows,
            as_of,
            ["a", "b", "c", "d", "e"],
            vec![Box::new(ReturnFactor::one_month())],
        )
        .unwrap();
        let input = PriceOnlyBars::from_test_parts(&rows, as_of).unwrap();
        let computed = compute_price_only_rows(
            &input,
            vec![Box::new(ReturnFactor::one_month())],
            Box::new(ZScorePolicy::default()),
        )
        .unwrap();
        assert_eq!(computed.rows, historical.rows);
        assert_eq!(computed.factor_versions, historical.factor_versions);
        assert_eq!(computed.normalization, historical.normalization);
        let last = computed
            .rows
            .iter()
            .find(|r| r.instrument == "069500.KRX" && r.date == "2026-08-31")
            .unwrap();
        assert!((last.raw.unwrap() - 0.2).abs() < 1e-12);
    }

    #[test]
    fn i3_shared_factor_math_cannot_use_future_rows_or_fill_missing_sessions() {
        let rows = bars();
        let as_of = date("2026-08-31");
        let early = rows
            .iter()
            .filter(|bar| bar.session_date <= as_of)
            .cloned()
            .collect::<Vec<_>>();
        let compute = |rows: &[HistoricalPriceOnlyBar]| {
            let input = PriceOnlyBars::from_test_parts(rows, as_of).unwrap();
            compute_price_only_rows(
                &input,
                vec![Box::new(ReturnFactor::one_month())],
                Box::new(ZScorePolicy::default()),
            )
            .unwrap()
            .rows
        };
        assert_eq!(compute(&rows), compute(&early));
        let missing = rows
            .iter()
            .filter(|b| !(b.session_date == as_of && b.instrument_id.as_str() == "069500.KRX"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(PriceOnlyBars::from_test_parts(&missing, as_of).is_err());
        assert!(PriceOnlyBars::from_test_parts(&rows, date("2026-08-30")).is_err());
    }

    #[test]
    fn i3_shared_factor_math_rejects_liquidity_before_factor_execution() {
        let input = PriceOnlyBars::from_test_parts(&bars(), date("2026-08-31")).unwrap();
        assert!(matches!(
            compute_price_only_rows(
                &input,
                vec![Box::new(AvgValueFactor)],
                Box::new(ZScorePolicy::default())
            ),
            Err(FactorError::MissingField { .. })
        ));
    }
}
