//! Currency and venue primitives.
//!
//! `Currency` is an ISO 4217 code (three uppercase letters) used by every
//! money-valued contract. `Venue` is the market/broker code used inside
//! instrument IDs (`069500.KRX`) and venue-local timestamps. Neither may be
//! represented as a free-form string outside this module.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::DomainError;

/// An ISO 4217 currency code, stored as three uppercase ASCII letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Currency([u8; 3]);

impl Currency {
    /// Korean Won — the base currency of the fixed Korean ETF universe.
    pub const KRW: Self = Self(*b"KRW");
    /// United States Dollar.
    pub const USD: Self = Self(*b"USD");
    /// Japanese Yen.
    pub const JPY: Self = Self(*b"JPY");
    /// Euro.
    pub const EUR: Self = Self(*b"EUR");
    /// Chinese Yuan.
    pub const CNY: Self = Self(*b"CNY");
    /// Pound Sterling.
    pub const GBP: Self = Self(*b"GBP");
    /// Kuwaiti Dinar.
    pub const KWD: Self = Self(*b"KWD");

    /// Validates that `code` is exactly three uppercase ASCII letters.
    pub fn from_code(code: &str) -> Result<Self, DomainError> {
        let bytes = code.as_bytes();
        if bytes.len() == 3 && bytes.iter().all(|b| b.is_ascii_uppercase()) {
            Ok(Self([bytes[0], bytes[1], bytes[2]]))
        } else {
            Err(DomainError::InvalidCurrency {
                value: code.to_owned(),
            })
        }
    }

    /// The three-letter ISO 4217 code (always valid UTF-8 by construction).
    pub fn code(&self) -> &str {
        // Invariant: exactly three uppercase ASCII bytes, so this cannot fail.
        std::str::from_utf8(&self.0).expect("currency is always three ASCII bytes")
    }

    /// Number of decimal places of the currency's minor unit (informational;
    /// all money arithmetic uses the domain's canonical fixed-point scale).
    pub fn minor_units(&self) -> u8 {
        match self.code() {
            "KRW" | "JPY" => 0,
            "KWD" => 3,
            _ => 2,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Currency {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_code(s)
    }
}

impl Serialize for Currency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Self::from_code(&code).map_err(DeError::custom)
    }
}

/// A market venue or broker code used in instrument IDs and venue timestamps.
///
/// Serde uses lowercase codes (`"krx"`) on the wire; [`Venue::from_str`] and
/// [`fmt::Display`] are case-insensitive/uppercase respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Venue {
    /// Korea Exchange — primary venue of the fixed Korean ETF universe.
    Krx,
    /// NYSE Arca — venue of the design-doc `SPY.ARCA` example.
    Arca,
    /// New York Stock Exchange.
    Nyse,
    /// NASDAQ.
    Nasdaq,
    /// Korea Investment & Securities — the Owner-only broker channel.
    Kis,
}

impl Venue {
    /// Canonical uppercase venue code used inside instrument IDs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Krx => "KRX",
            Self::Arca => "ARCA",
            Self::Nyse => "NYSE",
            Self::Nasdaq => "NASDAQ",
            Self::Kis => "KIS",
        }
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Venue {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "KRX" => Ok(Self::Krx),
            "ARCA" => Ok(Self::Arca),
            "NYSE" => Ok(Self::Nyse),
            "NASDAQ" => Ok(Self::Nasdaq),
            "KIS" => Ok(Self::Kis),
            _ => Err(DomainError::InvalidVenue {
                value: s.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_validation() {
        assert_eq!(Currency::from_code("KRW"), Ok(Currency::KRW));
        assert_eq!(Currency::KRW.code(), "KRW");
        assert!(matches!(
            Currency::from_code("kRw"),
            Err(DomainError::InvalidCurrency { .. })
        ));
        assert!(matches!(
            Currency::from_code("KRWX"),
            Err(DomainError::InvalidCurrency { .. })
        ));
        assert!(matches!(
            Currency::from_code(""),
            Err(DomainError::InvalidCurrency { .. })
        ));
    }

    #[test]
    fn venue_round_trip() {
        assert_eq!(Venue::from_str("krx"), Ok(Venue::Krx));
        assert_eq!(Venue::from_str("KRX"), Ok(Venue::Krx));
        assert_eq!(Venue::Krx.to_string(), "KRX");
        assert_eq!(serde_json::to_string(&Venue::Krx).unwrap(), "\"krx\"");
        assert!(matches!(
            Venue::from_str("XYZ"),
            Err(DomainError::InvalidVenue { .. })
        ));
    }
}
