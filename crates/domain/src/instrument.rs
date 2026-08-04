//! Instrument identity and asset-class types.
//!
//! `InstrumentId = {canonical_symbol}.{venue}` (e.g. `069500.KRX`, `SPY.ARCA`)
//! per the system design §6.4. The internal id is deliberately separated from
//! provider tickers — a ticker change updates alias history, never identity.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::currency::Venue;
use crate::error::DomainError;

/// Asset class of an instrument (design §6.4 instrument master).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    /// Exchange-traded fund — the fixed Korean ETF universe.
    Etf,
    /// Equity.
    Equity,
    /// Bond.
    Bond,
    /// Cash instrument.
    Cash,
    /// Market index.
    Index,
}

/// Canonical instrument identity `{symbol}.{VENUE}`.
///
/// The symbol is 1-12 uppercase alphanumerics (KRX codes are six digits); the
/// venue is a known [`Venue`]. Strings are the JSON representation, but the
/// type is what crosses boundaries — never a raw symbol string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId {
    symbol: String,
    venue: Venue,
}

impl InstrumentId {
    /// Validates and wraps a symbol + venue pair.
    pub fn from_parts(symbol: &str, venue: Venue) -> Result<Self, DomainError> {
        let b = symbol.as_bytes();
        let valid = !b.is_empty()
            && b.len() <= 12
            && (b[0].is_ascii_uppercase() || b[0].is_ascii_digit())
            && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if valid {
            Ok(Self {
                symbol: symbol.to_owned(),
                venue,
            })
        } else {
            Err(DomainError::InvalidId {
                kind: "instrument_id".to_owned(),
                value: format!("{symbol}.{venue}"),
            })
        }
    }

    /// Parses a `{symbol}.{venue}` string, validating both parts.
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let (symbol, venue) = s.rsplit_once('.').ok_or_else(|| DomainError::InvalidId {
            kind: "instrument_id".to_owned(),
            value: s.to_owned(),
        })?;
        let venue = Venue::from_str(venue).map_err(|_| DomainError::InvalidId {
            kind: "instrument_id".to_owned(),
            value: s.to_owned(),
        })?;
        Self::from_parts(symbol, venue).map_err(|_| DomainError::InvalidId {
            kind: "instrument_id".to_owned(),
            value: s.to_owned(),
        })
    }

    /// The canonical symbol (upper-cased by validation).
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// The venue.
    pub fn venue(&self) -> Venue {
        self.venue
    }

    /// The canonical `{symbol}.{VENUE}` string.
    pub fn as_str(&self) -> String {
        format!("{}.{}", self.symbol, self.venue)
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl FromStr for InstrumentId {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for InstrumentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for InstrumentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DomainError;

    #[test]
    fn parses_documented_format() {
        let id = InstrumentId::parse("069500.KRX").unwrap();
        assert_eq!(id.symbol(), "069500");
        assert_eq!(id.venue(), Venue::Krx);
        assert_eq!(id.to_string(), "069500.KRX");

        let spy = InstrumentId::parse("SPY.ARCA").unwrap();
        assert_eq!(spy.to_string(), "SPY.ARCA");
    }

    #[test]
    fn rejects_invalid() {
        assert!(matches!(
            InstrumentId::parse("lower.krx"),
            Err(DomainError::InvalidId { .. })
        ));
        assert!(matches!(
            InstrumentId::parse("069500.NOPE"),
            Err(DomainError::InvalidId { .. })
        ));
        assert!(matches!(
            InstrumentId::parse("069500KRX"),
            Err(DomainError::InvalidId { .. })
        ));
        assert!(matches!(
            InstrumentId::from_parts("", Venue::Krx),
            Err(DomainError::InvalidId { .. })
        ));
    }

    #[test]
    fn json_round_trip() {
        let id = InstrumentId::parse("069500.KRX").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"069500.KRX\"");
        assert_eq!(serde_json::from_str::<InstrumentId>(&json).unwrap(), id);
    }
}
