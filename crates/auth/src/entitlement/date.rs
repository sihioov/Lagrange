//! Dependency-free civil calendar date used by the entitlement gate.
//!
//! The crate intentionally has **zero dependencies** (a policy engine must stay
//! portable and reviewable); dates are stored as days-since-epoch (1970-01-01)
//! and converted to/from `YYYY-MM-DD` civil form using Howard Hinnant's
//! `days_from_civil` / `civil_from_days` algorithms.

use std::fmt;
use std::str::FromStr;

/// A civil calendar date with `Ord` semantics on the day count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CalendarDate {
    days: i64,
}

/// Error produced when parsing or constructing an invalid [`CalendarDate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateError {
    /// The string is not a `YYYY-MM-DD` date.
    InvalidFormat(String),
    /// The month is outside `1..=12` or the day is outside the month.
    InvalidDay { year: i32, month: u32, day: u32 },
    /// The year is outside the supported `1..=9999` range.
    YearOutOfRange(i32),
}

impl fmt::Display for DateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(s) => write!(f, "invalid date format (expected YYYY-MM-DD): {s:?}"),
            Self::InvalidDay { year, month, day } => {
                write!(f, "invalid calendar day: {year:04}-{month:02}-{day:02}")
            }
            Self::YearOutOfRange(y) => write!(f, "year out of supported range 1..=9999: {y}"),
        }
    }
}

impl std::error::Error for DateError {}

/// Days from 1970-01-01 to the first day of `(y, m)` (Hinnant).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = i64::from(y - era * 400); // [0, 399]
    let mp = (i64::from(m) + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    i64::from(era) * 146_097 + doe - 719_468
}

/// Civil `(y, m, d)` from days since 1970-01-01 (Hinnant).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

const fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

impl CalendarDate {
    /// Construct a date directly from its days-since-epoch count.
    pub const fn from_days(days: i64) -> Self {
        Self { days }
    }

    /// Construct from `(year, month, day)` with strict validation.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, DateError> {
        if !(1..=9999).contains(&year) {
            return Err(DateError::YearOutOfRange(year));
        }
        if !(1..=12).contains(&month) {
            return Err(DateError::InvalidDay { year, month, day });
        }
        if day == 0 || day > days_in_month(year, month) {
            return Err(DateError::InvalidDay { year, month, day });
        }
        Ok(Self {
            days: days_from_civil(year, month, day),
        })
    }

    /// Parse a strict `YYYY-MM-DD` string.
    pub fn parse(s: &str) -> Result<Self, DateError> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return Err(DateError::InvalidFormat(s.to_owned()));
        }
        let digit = |i: usize| -> Option<u32> {
            if b[i].is_ascii_digit() {
                Some(u32::from(b[i] - b'0'))
            } else {
                None
            }
        };
        let year = {
            let mut y = 0i32;
            for i in 0..4 {
                let digit = digit(i).ok_or_else(|| DateError::InvalidFormat(s.to_owned()))? as i32;
                y = y * 10 + digit;
            }
            y
        };
        let month = digit(5).ok_or_else(|| DateError::InvalidFormat(s.to_owned()))? * 10
            + digit(6).ok_or_else(|| DateError::InvalidFormat(s.to_owned()))?;
        let day = digit(8).ok_or_else(|| DateError::InvalidFormat(s.to_owned()))? * 10
            + digit(9).ok_or_else(|| DateError::InvalidFormat(s.to_owned()))?;
        Self::from_ymd(year, month, day)
    }

    /// The date as `(year, month, day)`.
    pub fn to_ymd(&self) -> (i32, u32, u32) {
        civil_from_days(self.days)
    }

    /// Days since the Unix epoch (1970-01-01).
    pub const fn days_since_epoch(&self) -> i64 {
        self.days
    }

    /// The date `n` calendar days later (negative moves earlier).
    pub const fn add_days(self, n: i64) -> Self {
        Self {
            days: self.days + n,
        }
    }
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.to_ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl FromStr for CalendarDate {
    type Err = DateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips() {
        for s in [
            "1970-01-01",
            "2020-01-31",
            "2026-08-05",
            "2000-02-29",
            "2024-02-29",
            "9999-12-31",
            "0001-01-01",
        ] {
            let d = CalendarDate::parse(s).unwrap();
            assert_eq!(d.to_string(), s, "round-trip for {s}");
        }
    }

    #[test]
    fn rejects_invalid_dates() {
        for bad in [
            "2023-02-29", // not a leap year
            "2021-04-31",
            "2021-13-01",
            "2021-00-10",
            "2021-01-00",
            "2021-01-32",
            "2021-1-01",
            "2021-01-1",
            "20210101",
            "abcd-ef-gh",
            "",
        ] {
            assert!(CalendarDate::parse(bad).is_err(), "should reject {bad:?}");
        }
        assert!(CalendarDate::from_ymd(2023, 2, 29).is_err());
        assert!(CalendarDate::from_ymd(2024, 2, 29).is_ok());
        assert!(CalendarDate::from_ymd(0, 1, 1).is_err());
    }

    #[test]
    fn ordering_and_arithmetic() {
        let a = CalendarDate::parse("2026-01-01").unwrap();
        let b = CalendarDate::parse("2026-06-15").unwrap();
        let c = CalendarDate::parse("2026-12-31").unwrap();
        assert!(a < b && b < c);
        assert_eq!(a.add_days(165), b); // 2026-06-15 is day 165 of 2026
        assert_eq!(b.add_days(-165), a);
        // 2026 is not a leap year: Jan 1 + 364 days = Dec 31.
        assert_eq!(a.add_days(364), c);
        assert_eq!(a.add_days(365), CalendarDate::parse("2027-01-01").unwrap());
    }
}
