//! Reported statistics — the only place floats are allowed, behind explicit
//! finite-value checks (plan Todo 2: "floats may exist only for reported
//! statistics with finite-value checks"). Money never touches `f64`.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// A finite float used ONLY for reported statistics (metrics, ratios).
///
/// NaN and ±Infinity are rejected as typed errors at construction,
/// deserialization, and string parsing.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ReportedStat(f64);

impl ReportedStat {
    /// Validates that the value is finite (no NaN / Infinity).
    pub fn from_f64(value: f64) -> Result<Self, DomainError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(DomainError::NonFiniteMetric {
                metric: value.to_string(),
            })
        }
    }

    /// The underlying float.
    pub fn value(&self) -> f64 {
        self.0
    }
}

impl fmt::Display for ReportedStat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ReportedStat {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = s.parse::<f64>().map_err(|_| DomainError::NonFiniteMetric {
            metric: s.to_owned(),
        })?;
        Self::from_f64(value)
    }
}

impl Serialize for ReportedStat {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for ReportedStat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        Self::from_f64(value).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_only() {
        assert_eq!(ReportedStat::from_f64(1.23).unwrap().value(), 1.23);
        assert!(matches!(
            ReportedStat::from_f64(f64::NAN),
            Err(DomainError::NonFiniteMetric { .. })
        ));
        assert!(matches!(
            ReportedStat::from_f64(f64::INFINITY),
            Err(DomainError::NonFiniteMetric { .. })
        ));
        assert!(matches!(
            "NaN".parse::<ReportedStat>(),
            Err(DomainError::NonFiniteMetric { .. })
        ));
        assert!(serde_json::from_str::<ReportedStat>("1.5").is_ok());
    }
}
