//! Versioned cross-sectional normalization policies (design §6.5:
//! "winsorization, z-score, percentile 규칙은 팩터 버전에 포함").
//!
//! Each policy is applied per date over the FROZEN cross-sectional universe
//! of that date (universe membership is fixed at snapshot build time; only
//! the instruments present on the date participate). A cross-section with
//! fewer than [`MIN_NORMALIZATION_SAMPLE`] non-NULL values produces NULL for
//! the whole date/factor (never a degenerate statistic). NULL inputs pass
//! through as NULL.

use std::collections::BTreeMap;

use domain::FactorVersion;

use crate::contract::FactorError;

/// The documented minimum cross-section size for normalization.
pub const MIN_NORMALIZATION_SAMPLE: usize = 3;

/// A versioned cross-sectional normalization transformation.
pub trait NormalizePolicy: Send + Sync {
    /// The stable policy id (e.g. `z_score`).
    fn id(&self) -> &'static str;
    /// The immutable policy version (semver).
    fn version(&self) -> FactorVersion;
    /// The minimum non-NULL cross-section size (below: NULL for all).
    fn min_sample(&self) -> usize;
    /// The canonical parameter record (included in the snapshot hash).
    fn params(&self) -> BTreeMap<String, String>;
    /// Applies the policy to one date/factor cross-section (input order is
    /// the frozen universe's canonical instrument order).
    fn apply(&self, xs: &[Option<f64>]) -> Vec<Option<f64>>;
}

/// The z-score policy (version 1.0.0): `(x - mean) / population_sd` over the
/// frozen cross-section; population standard deviation (ddof 0). A
/// zero-variance cross-section is a typed NULL (all values), never a panic.
/// An optional symmetric cap clips the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZScorePolicy {
    cap: Option<f64>,
}

impl ZScorePolicy {
    pub fn new(cap: Option<f64>) -> Self {
        Self { cap }
    }

    /// The symmetric clip (None = no clip).
    pub fn cap(&self) -> Option<f64> {
        self.cap
    }
}

impl Default for ZScorePolicy {
    fn default() -> Self {
        Self::new(Some(3.0))
    }
}

impl NormalizePolicy for ZScorePolicy {
    fn id(&self) -> &'static str {
        "z_score"
    }

    fn version(&self) -> FactorVersion {
        FactorVersion::parse("1.0.0").expect("static version")
    }

    fn min_sample(&self) -> usize {
        MIN_NORMALIZATION_SAMPLE
    }

    fn params(&self) -> BTreeMap<String, String> {
        match self.cap {
            Some(c) => BTreeMap::from([("cap".to_owned(), format!("{c}"))]),
            None => BTreeMap::new(),
        }
    }

    fn apply(&self, xs: &[Option<f64>]) -> Vec<Option<f64>> {
        let present: Vec<f64> = xs.iter().flatten().copied().collect();
        if present.len() < self.min_sample() {
            return vec![None; xs.len()];
        }
        let n = present.len() as f64;
        let mean = present.iter().sum::<f64>() / n;
        let variance = present.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        if variance == 0.0 {
            return vec![None; xs.len()];
        }
        let sd = variance.sqrt();
        xs.iter()
            .map(|x| {
                x.map(|v| {
                    let z = (v - mean) / sd;
                    match self.cap {
                        Some(c) => z.clamp(-c, c),
                        None => z,
                    }
                })
            })
            .collect()
    }
}

/// The winsorize policy (version 1.0.0): values below the lower quantile are
/// clipped to it, values above the upper quantile to it. Quantile indices are
/// deterministic: `lower_idx = floor((n-1) * lower)`, `upper_idx =
/// ceil((n-1) * upper)` over the sorted non-NULL cross-section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WinsorizePolicy {
    lower: f64,
    upper: f64,
}

impl WinsorizePolicy {
    pub fn new(lower: f64, upper: f64) -> Result<Self, FactorError> {
        if !(0.0..=1.0).contains(&lower)
            || !(0.0..=1.0).contains(&upper)
            || lower.partial_cmp(&upper) != Some(std::cmp::Ordering::Less)
        {
            return Err(FactorError::InvalidDefinition {
                detail: format!(
                    "winsorize quantiles must satisfy 0 <= lower < upper <= 1, got {lower}, {upper}"
                ),
            });
        }
        Ok(Self { lower, upper })
    }

    /// The (lower, upper) quantiles.
    pub fn quantiles(&self) -> (f64, f64) {
        (self.lower, self.upper)
    }
}

impl NormalizePolicy for WinsorizePolicy {
    fn id(&self) -> &'static str {
        "winsorize"
    }

    fn version(&self) -> FactorVersion {
        FactorVersion::parse("1.0.0").expect("static version")
    }

    fn min_sample(&self) -> usize {
        MIN_NORMALIZATION_SAMPLE
    }

    fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("lower".to_owned(), format!("{}", self.lower)),
            ("upper".to_owned(), format!("{}", self.upper)),
        ])
    }

    fn apply(&self, xs: &[Option<f64>]) -> Vec<Option<f64>> {
        let mut present: Vec<f64> = xs.iter().flatten().copied().collect();
        if present.len() < self.min_sample() {
            return vec![None; xs.len()];
        }
        present.sort_by(f64::total_cmp);
        let n = present.len() as f64;
        let lower_idx = ((n - 1.0) * self.lower).floor() as usize;
        let upper_idx = ((n - 1.0) * self.upper).ceil() as usize;
        let lo = present[lower_idx];
        let hi = present[upper_idx];
        xs.iter().map(|x| x.map(|v| v.clamp(lo, hi))).collect()
    }
}

/// The percentile policy (version 1.0.0):
/// `pct(x) = (# non-NULL values strictly less than x) / (n - 1)`, in
/// `[0, 1]`. Equal values share the count of strictly-smaller values
/// (deterministic ties).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PercentilePolicy;

impl NormalizePolicy for PercentilePolicy {
    fn id(&self) -> &'static str {
        "percentile"
    }

    fn version(&self) -> FactorVersion {
        FactorVersion::parse("1.0.0").expect("static version")
    }

    fn min_sample(&self) -> usize {
        MIN_NORMALIZATION_SAMPLE
    }

    fn params(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn apply(&self, xs: &[Option<f64>]) -> Vec<Option<f64>> {
        let present: Vec<f64> = xs.iter().flatten().copied().collect();
        if present.len() < self.min_sample() {
            return vec![None; xs.len()];
        }
        let n = present.len() as f64;
        xs.iter()
            .map(|x| {
                x.map(|v| {
                    let less = present.iter().filter(|&&o| o < v).count() as f64;
                    less / (n - 1.0)
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winsorize_rejects_bad_quantiles() {
        assert!(WinsorizePolicy::new(0.5, 0.25).is_err());
        assert!(WinsorizePolicy::new(-0.1, 0.9).is_err());
        assert!(WinsorizePolicy::new(0.1, 1.1).is_err());
        assert!(WinsorizePolicy::new(0.25, 0.75).is_ok());
    }
}
