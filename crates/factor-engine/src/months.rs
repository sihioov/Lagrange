//! Calendar month-window resolution for the return factors.
//!
//! `months_back(d, n)` = the calendar date `d` minus `n` months, with the
//! day-of-month clamped to the target month's last day (so 2020-03-31 minus
//! 1 month = 2020-02-29). The return factors then use the documented
//! MONTH-END convention: the reference bar is the LAST bar on or before the
//! last day of the target month (`month_end(months_back(...))`). This matches
//! standard monthly momentum practice (month-end series) and guarantees the
//! reference is always a strictly past observation (no forward fill, no
//! post-as-of leakage); a missing month falls back to the last bar before it.

use chrono::{Datelike, NaiveDate};

/// `date` minus `months` calendar months, day clamped to the month's length.
pub fn months_back(date: NaiveDate, months: u32) -> NaiveDate {
    let total = date.year() * 12 + date.month0() as i32 - months as i32;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) as u32 + 1;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).expect("clamped month date is valid")
}

/// The last day of `date`'s month.
pub fn month_end(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(
        date.year(),
        date.month(),
        days_in_month(date.year(), date.month()),
    )
    .expect("month end is a valid date")
}

/// The number of days in a month (proleptic Gregorian, leap years included).
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => unreachable!("month is always 1..=12"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_day_to_target_month() {
        let d = NaiveDate::from_ymd_opt(2020, 3, 31).expect("date");
        assert_eq!(
            months_back(d, 1),
            NaiveDate::from_ymd_opt(2020, 2, 29).expect("leap feb")
        );
        let d = NaiveDate::from_ymd_opt(2020, 4, 30).expect("date");
        assert_eq!(
            months_back(d, 1),
            NaiveDate::from_ymd_opt(2020, 3, 30).expect("mar")
        );
        let d = NaiveDate::from_ymd_opt(2019, 3, 31).expect("date");
        assert_eq!(
            months_back(d, 1),
            NaiveDate::from_ymd_opt(2019, 2, 28).expect("feb")
        );
    }

    #[test]
    fn crosses_year_boundaries() {
        let d = NaiveDate::from_ymd_opt(2020, 1, 15).expect("date");
        assert_eq!(
            months_back(d, 1),
            NaiveDate::from_ymd_opt(2019, 12, 15).expect("dec")
        );
        let d = NaiveDate::from_ymd_opt(2020, 12, 31).expect("date");
        assert_eq!(
            months_back(d, 12),
            NaiveDate::from_ymd_opt(2019, 12, 31).expect("dec")
        );
    }

    #[test]
    fn month_end_is_last_day() {
        assert_eq!(
            month_end(NaiveDate::from_ymd_opt(2020, 2, 15).expect("d")),
            NaiveDate::from_ymd_opt(2020, 2, 29).expect("leap")
        );
        assert_eq!(
            month_end(NaiveDate::from_ymd_opt(2020, 4, 1).expect("d")),
            NaiveDate::from_ymd_opt(2020, 4, 30).expect("apr")
        );
        assert_eq!(
            month_end(NaiveDate::from_ymd_opt(2020, 12, 31).expect("d")),
            NaiveDate::from_ymd_opt(2020, 12, 31).expect("dec")
        );
    }
}
