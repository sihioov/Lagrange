//! Input validation: calendar dates, decimal strings, currencies, and the
//! payload size gate. Every rejection is a typed 4xx with no side effects.

use chrono::NaiveDate;
use serde_json::Value;

/// Parse a strict `YYYY-MM-DD` calendar date.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    if !s.bytes().all(|b| b.is_ascii_digit() || b == b'-') || s.len() != 10 {
        return None;
    }
    let mut it = s.split('-');
    let year: i32 = it.next()?.parse().ok()?;
    let month: u32 = it.next()?.parse().ok()?;
    let day: u32 = it.next()?.parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Validate a fixed-point decimal string: optional sign, digits, one dot,
/// no exponent, finite. Used for money/weight/quantity wire values.
pub fn is_valid_decimal(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let body = s.strip_prefix('-').unwrap_or(s);
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if let Some(f) = frac_part
        && (f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    true
}

/// Supported settlement currencies (KRW first; the fixed Korean ETF
/// universe is KRW-only).
pub const SUPPORTED_CURRENCIES: &[&str] = &["KRW"];

/// Validate a currency code against the supported set.
pub fn is_supported_currency(c: &str) -> bool {
    SUPPORTED_CURRENCIES.contains(&c)
}

/// The canonical fixed universe (Todo 12): benchmark + members. A backtest
/// benchmark must be one of these canonical ids.
pub const FIXED_UNIVERSE: &[&str] = &[
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
];

/// The KRW benchmark of the fixed universe.
pub const BENCHMARK: &str = "069500.KRX";

/// Whether `id` is a canonical member of the fixed KR ETF v1 universe.
pub fn in_fixed_universe(id: &str) -> bool {
    FIXED_UNIVERSE.contains(&id)
}

/// Canonical sha256 hex of a JSON value (config hashes; 64 hex chars).
pub fn sha256_hex(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(v.to_string().as_bytes());
    hex::encode(hasher.finalize())
}
