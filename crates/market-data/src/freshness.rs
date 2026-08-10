//! Pure KRX EOD freshness semantics shared by database consumers.
//!
//! The database lookup remains in each SQLx-owning crate. This module only
//! defines how a selected batch date and retrieval timestamp become an age.

use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveTime, TimeZone, Utc};

const SEOUL_OFFSET_SECS: i32 = 9 * 60 * 60;

/// Returns the age of an applicable EOD batch without allowing a backfill's
/// retrieval timestamp to make historical market data look current.
///
/// The effective instant is the earlier of `retrieved_at` and the exclusive
/// end of `batch_date` in Asia/Seoul (midnight beginning the next civil day).
/// An effective instant in the future is invalid rather than clamped to zero.
pub fn applicable_eod_age(
    now_utc: DateTime<Utc>,
    batch_date: chrono::NaiveDate,
    retrieved_at: DateTime<Utc>,
) -> Option<Duration> {
    applicable_eod_freshness(now_utc, batch_date, retrieved_at).map(|(_, age)| age)
}

/// Returns both the effective freshness instant and its age. The pair is
/// absent when the effective instant would be in the future.
pub fn applicable_eod_freshness(
    now_utc: DateTime<Utc>,
    batch_date: chrono::NaiveDate,
    retrieved_at: DateTime<Utc>,
) -> Option<(DateTime<Utc>, Duration)> {
    let next_date = batch_date.succ_opt()?;
    let seoul = FixedOffset::east_opt(SEOUL_OFFSET_SECS)?;
    let end_of_batch_date = seoul
        .from_local_datetime(&next_date.and_time(NaiveTime::MIN))
        .single()?
        .with_timezone(&Utc);
    let effective_at = retrieved_at.min(end_of_batch_date);
    let age = now_utc.signed_duration_since(effective_at).to_std().ok()?;
    Some((effective_at, age))
}
