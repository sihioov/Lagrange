//! Time contracts.
//!
//! - [`UtcTimestamp`]: a UTC instant in RFC 3339 form.
//! - [`VenueTimestamp`]: a venue-local wall-clock instant with an explicit
//!   venue and IANA timezone, normalized from/to UTC. Ambiguous (DST
//!   fall-back) and nonexistent (DST spring-forward) local times are rejected
//!   as typed errors — "explicit timezone normalization" per the design.
//! - [`TradingDate`]: a calendar date with no time component.
//! - [`Zone`]: an IANA timezone name wrapper.

use std::fmt;
use std::str::FromStr;

use chrono::TimeZone as _;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, SecondsFormat, Utc, Weekday};
use serde::de::Error as DeError;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::currency::Venue;
use crate::error::DomainError;

/// An IANA timezone name (e.g. `Asia/Seoul`), wrapping `chrono_tz::Tz`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Zone(chrono_tz::Tz);

impl Zone {
    /// Seoul, Korea — KRX/KIS trading sessions.
    pub const SEOUL: Self = Self(chrono_tz::Tz::Asia__Seoul);
    /// New York — NYSE/ARCA/NASDAQ trading sessions.
    pub const NEW_YORK: Self = Self(chrono_tz::Tz::America__New_York);

    /// Resolves an IANA name (e.g. `"Asia/Seoul"`).
    pub fn from_name(name: &str) -> Result<Self, DomainError> {
        name.parse::<chrono_tz::Tz>()
            .map(Self)
            .map_err(|_| DomainError::InvalidTimeZone {
                value: name.to_owned(),
            })
    }

    /// The IANA name (e.g. `"Asia/Seoul"`).
    pub fn name(&self) -> &'static str {
        self.0.name()
    }

    /// The underlying `chrono_tz::Tz`.
    pub fn as_tz(&self) -> chrono_tz::Tz {
        self.0
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Zone {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s)
    }
}

impl Serialize for Zone {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Zone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::from_name(&name).map_err(DeError::custom)
    }
}

/// A UTC instant, serialized as RFC 3339 (`2026-08-04T15:00:00Z`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp(DateTime<Utc>);

impl UtcTimestamp {
    /// The current UTC instant.
    pub fn now() -> Self {
        Self(Utc::now())
    }

    /// Wraps a `chrono` UTC datetime.
    pub fn from_datetime(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    /// Parses an RFC 3339 timestamp, normalizing any offset to UTC.
    pub fn parse_rfc3339(s: &str) -> Result<Self, DomainError> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| Self(dt.with_timezone(&Utc)))
            .map_err(|_| DomainError::InvalidId {
                kind: "utc_timestamp".to_owned(),
                value: s.to_owned(),
            })
    }

    /// The inner `chrono` UTC datetime.
    pub fn as_datetime(&self) -> DateTime<Utc> {
        self.0
    }

    /// RFC 3339 UTC string (`2026-08-04T15:00:00Z`).
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl FromStr for UtcTimestamp {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_rfc3339(s)
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse_rfc3339(&s).map_err(DeError::custom)
    }
}

fn venue_timezone(venue: Venue) -> chrono_tz::Tz {
    venue.timezone().as_tz()
}

/// A venue-local timestamp: an explicit venue plus the venue's wall-clock
/// instant, normalized from/to UTC through the venue's IANA timezone.
///
/// Serializes as `{"venue":"krx","local":"2026-08-05T00:00:00+09:00"}` — the
/// venue and its explicit offset travel together (ISO 8601 with explicit
/// timezone, per design §12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VenueTimestamp {
    venue: Venue,
    local: DateTime<chrono_tz::Tz>,
}

impl VenueTimestamp {
    /// Interprets a naive (wall-clock) local time in the venue's timezone,
    /// rejecting ambiguous and nonexistent local times as typed errors.
    pub fn from_naive_local(venue: Venue, local: NaiveDateTime) -> Result<Self, DomainError> {
        let tz = venue_timezone(venue);
        match tz.from_local_datetime(&local) {
            chrono::LocalResult::Single(dt) => Ok(Self { venue, local: dt }),
            chrono::LocalResult::Ambiguous(_, _) => Err(DomainError::AmbiguousLocalTime {
                venue,
                local: local.to_string(),
            }),
            chrono::LocalResult::None => Err(DomainError::NonexistentLocalTime {
                venue,
                local: local.to_string(),
            }),
        }
    }

    /// Normalizes a UTC instant into the venue's wall clock (always valid).
    pub fn from_utc(venue: Venue, utc: UtcTimestamp) -> Self {
        let tz = venue_timezone(venue);
        Self {
            venue,
            local: utc.as_datetime().with_timezone(&tz),
        }
    }

    /// Parses a venue + RFC 3339 local string, normalizing the instant into
    /// the venue's true wall clock.
    pub fn parse_rfc3339(venue: Venue, s: &str) -> Result<Self, DomainError> {
        let dt = DateTime::parse_from_rfc3339(s).map_err(|_| DomainError::InvalidId {
            kind: "venue_timestamp".to_owned(),
            value: s.to_owned(),
        })?;
        let tz = venue_timezone(venue);
        Ok(Self {
            venue,
            local: dt.with_timezone(&tz),
        })
    }

    /// The venue.
    pub fn venue(&self) -> Venue {
        self.venue
    }

    /// The venue-local wall-clock datetime.
    pub fn local_naive(&self) -> NaiveDateTime {
        self.local.naive_local()
    }

    /// The venue-local instant in RFC 3339 form with its explicit offset.
    pub fn to_rfc3339(&self) -> String {
        self.local.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// The equivalent UTC instant.
    pub fn to_utc(&self) -> UtcTimestamp {
        UtcTimestamp(self.local.with_timezone(&Utc))
    }
}

impl fmt::Display for VenueTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for VenueTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("VenueTimestamp", 2)?;
        state.serialize_field("venue", &self.venue)?;
        state.serialize_field("local", &self.to_rfc3339())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for VenueTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct VenueTimestampRepr {
            venue: Venue,
            local: String,
        }
        let repr = VenueTimestampRepr::deserialize(deserializer)?;
        Self::parse_rfc3339(repr.venue, &repr.local).map_err(DeError::custom)
    }
}

/// A calendar date (no time component), serialized as ISO `YYYY-MM-DD`.
///
/// This is the point-in-time key for daily bars, sessions, and corporate
/// actions — never a full timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TradingDate(NaiveDate);

impl TradingDate {
    /// Constructs a calendar date; rejects nonexistent dates (e.g. 2026-02-30).
    pub fn new(year: i32, month: u32, day: u32) -> Result<Self, DomainError> {
        NaiveDate::from_ymd_opt(year, month, day)
            .map(Self)
            .ok_or_else(|| DomainError::InvalidTradingDate {
                value: format!("{year:04}-{month:02}-{day:02}"),
            })
    }

    /// Parses an ISO `YYYY-MM-DD` date.
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Self)
            .map_err(|_| DomainError::InvalidTradingDate {
                value: s.to_owned(),
            })
    }

    /// The underlying `chrono` calendar date.
    pub fn as_naive_date(&self) -> NaiveDate {
        self.0
    }

    /// The day of the week.
    pub fn weekday(&self) -> Weekday {
        self.0.weekday()
    }

    /// Whether the date falls on Saturday or Sunday.
    pub fn is_weekend(&self) -> bool {
        matches!(self.0.weekday(), Weekday::Sat | Weekday::Sun)
    }

    /// The next calendar day.
    pub fn next_day(&self) -> Self {
        Self(self.0.succ_opt().unwrap_or(self.0))
    }

    /// The previous calendar day.
    pub fn previous_day(&self) -> Self {
        Self(self.0.pred_opt().unwrap_or(self.0))
    }

    /// Adds (or, with a negative argument, subtracts) a number of days,
    /// rejecting an out-of-range result.
    pub fn checked_add_days(&self, days: i64) -> Result<Self, DomainError> {
        let next = if days >= 0 {
            self.0.checked_add_days(chrono::Days::new(days as u64))
        } else {
            self.0.checked_sub_days(chrono::Days::new(days.unsigned_abs()))
        };
        next.map(Self)
            .ok_or_else(|| DomainError::InvalidTradingDate {
                value: self.to_iso(),
            })
    }

    /// ISO `YYYY-MM-DD` string.
    pub fn to_iso(&self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }
}

impl fmt::Display for TradingDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso())
    }
}

impl FromStr for TradingDate {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for TradingDate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_iso())
    }
}

impl<'de> Deserialize<'de> for TradingDate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::currency::Venue;

    fn naive_utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    #[test]
    fn venue_normalization_krx() {
        let local = naive_utc(2026, 8, 5, 9, 0, 0);
        let ts = VenueTimestamp::from_naive_local(Venue::Krx, local).unwrap();
        assert_eq!(ts.to_rfc3339(), "2026-08-05T09:00:00+09:00");
        assert_eq!(ts.to_utc().to_rfc3339(), "2026-08-05T00:00:00Z");
    }

    #[test]
    fn dst_ambiguity_and_nonexistence_rejected() {
        // US fall-back 2026-11-01 01:30 occurs twice.
        let ambiguous = naive_utc(2026, 11, 1, 1, 30, 0);
        assert!(matches!(
            VenueTimestamp::from_naive_local(Venue::Nyse, ambiguous),
            Err(DomainError::AmbiguousLocalTime { .. })
        ));
        // US spring-forward 2026-03-08 02:30 does not exist.
        let nonexistent = naive_utc(2026, 3, 8, 2, 30, 0);
        assert!(matches!(
            VenueTimestamp::from_naive_local(Venue::Nyse, nonexistent),
            Err(DomainError::NonexistentLocalTime { .. })
        ));
        // Seoul has no DST: both resolve fine.
        assert!(VenueTimestamp::from_naive_local(Venue::Krx, ambiguous).is_ok());
        assert!(VenueTimestamp::from_naive_local(Venue::Krx, nonexistent).is_ok());
    }

    #[test]
    fn trading_date_basics() {
        let td = TradingDate::new(2026, 8, 5).unwrap();
        assert_eq!(td.to_iso(), "2026-08-05");
        assert_eq!(td.weekday(), Weekday::Wed);
        assert!(!td.is_weekend());
        assert!(matches!(
            TradingDate::new(2026, 2, 30),
            Err(DomainError::InvalidTradingDate { .. })
        ));
        let sunday = TradingDate::new(2026, 8, 2).unwrap();
        assert!(sunday.is_weekend());
    }

    #[test]
    fn zone_round_trip() {
        assert_eq!(Zone::SEOUL.name(), "Asia/Seoul");
        assert_eq!(Zone::from_name("Asia/Seoul").unwrap(), Zone::SEOUL);
        let json = serde_json::to_string(&Zone::SEOUL).unwrap();
        assert_eq!(json, "\"Asia/Seoul\"");
        assert_eq!(serde_json::from_str::<Zone>(&json).unwrap(), Zone::SEOUL);
        assert!(matches!(
            Zone::from_name("Mars/Olympus"),
            Err(DomainError::InvalidTimeZone { .. })
        ));
    }
}
