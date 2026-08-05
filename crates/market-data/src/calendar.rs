//! KRX Korean exchange trading calendar (Todo 9).
//!
//! The calendar is a **versioned dataset**, never an inference rule:
//! - sessions are materialized, sorted, explicit `TradingDate`s — queries
//!   (`next_trading_day`, `previous_trading_day`, month-end) only ever consult
//!   the materialized set, so they can never "infer sessions from weekdays";
//! - holidays are explicit records with reasons (FR-DATA-003); weekends are
//!   never sessions;
//! - every published version carries source/version/content-hash provenance
//!   (FR-DATA-005); a correction produces a NEW version with a NEW hash — the
//!   published version is never mutated in place.
//!
//! Session semantics: 09:00-15:30 KST continuous (design §6.4), Asia/Seoul,
//! no DST (fixed +09:00). Open/close instants are timezone-aware
//! [`domain::UtcTimestamp`]s (09:00 KST = 00:00 UTC) and venue-local
//! [`domain::VenueTimestamp`]s.
//!
//! [`krx_2020`] builds the documented synthetic/official 2020 KRX calendar
//! (Seollal 2020-01-24/27, Chuseok 2020-09-30..10-02, election day 2020-04-15,
//! etc.) with provenance `krx-official-calendar-2020-v1`. It agrees with the
//! Todo 6 fixture `tests/fixtures/kr-etf/2020-01-31/calendar.json`:
//! `next_trading_day(2020-01-31) == 2020-02-03` and
//! `next_trading_day(2020-01-23) == 2020-01-28` (Seollal break).

use std::collections::BTreeSet;

use chrono::Datelike;
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

use domain::{ContentHash, TradingDate, UtcTimestamp, Venue, VenueTimestamp, Zone};

/// The KRX session window (local wall-clock times).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTimes {
    /// Local open time (KRX: 09:00 KST).
    pub open: NaiveTime,
    /// Local close time (KRX: 15:30 KST).
    pub close: NaiveTime,
}

impl SessionTimes {
    /// The documented KRX session window 09:00-15:30 KST.
    pub fn krx_default() -> Self {
        Self {
            open: NaiveTime::from_hms_opt(9, 0, 0).expect("valid time"),
            close: NaiveTime::from_hms_opt(15, 30, 0).expect("valid time"),
        }
    }
}

/// An explicit closure record: a date that is NOT a session, with a reason.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Holiday {
    /// The non-session date.
    pub date: TradingDate,
    /// Why the exchange was closed (e.g. `Seollal (lunar new year) break`).
    pub reason: String,
}

/// Provenance of a published calendar version (FR-DATA-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarProvenance {
    /// Stable calendar id (e.g. `krx-2020`).
    pub calendar_id: String,
    /// The versioned source id (e.g. `krx-official-calendar-2020-v1`).
    pub source: String,
    /// The version of this calendar (1, 2, ... — corrections bump it).
    pub version: u32,
    /// SHA-256 over the canonical serialization of this version's data
    /// (sessions, holidays, session times, timezone).
    pub content_hash: ContentHash,
    /// When this version was published.
    pub published_at: UtcTimestamp,
    /// The IANA timezone of the sessions (Asia/Seoul).
    pub timezone: Zone,
    /// Free-form provenance notes (e.g. synthetic data rights status).
    pub notes: Vec<String>,
}

/// The specification of a calendar version, from which a [`KrCalendar`] is
/// built (validated + content-hashed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrCalendarSpec {
    /// Stable calendar id.
    pub calendar_id: String,
    /// Session timezone (must be the venue timezone).
    pub timezone: Zone,
    /// Session window (KRX: 09:00-15:30).
    pub session_times: SessionTimes,
    /// Explicit session dates (materialized data; weekends are rejected).
    pub sessions: Vec<TradingDate>,
    /// Explicit closure records (data, with reasons).
    pub holidays: Vec<Holiday>,
    /// The versioned source id.
    pub source: String,
    /// The version of this calendar.
    pub version: u32,
    /// Publication instant of this version.
    pub published_at: UtcTimestamp,
    /// Provenance notes.
    pub notes: Vec<String>,
}

impl KrCalendarSpec {
    /// The v2 2020 KRX calendar spec: the documented v1 set plus one
    /// additional closure (a correction), published at `published_at`.
    pub fn krx_2020_v2_with_additional_holiday(extra: Holiday, published_at: UtcTimestamp) -> Self {
        let v1 = krx_2020();
        let mut sessions: Vec<TradingDate> = v1.sessions().filter(|d| *d != extra.date).collect();
        sessions.sort_unstable();
        let mut holidays = v1.holidays().to_vec();
        holidays.push(extra);
        holidays.sort_unstable();
        Self {
            calendar_id: v1.provenance().calendar_id.clone(),
            timezone: v1.timezone(),
            session_times: v1.session_times(),
            sessions,
            holidays,
            source: "krx-official-calendar-2020-v2".to_owned(),
            version: 2,
            published_at,
            notes: v1.provenance().notes.clone(),
        }
    }
}

/// Typed calendar errors (queries never panic on bad dates).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    /// The date is not a session of this calendar.
    #[error("date {date} is not a session of {calendar_id}")]
    NotASession { calendar_id: String, date: String },
    /// The query falls outside the calendar's covered range (no session to
    /// move to).
    #[error("no session {direction} of {date} in {calendar_id}")]
    OutsideCoveredRange {
        calendar_id: String,
        date: String,
        direction: &'static str,
    },
    /// A date appears in BOTH the explicit sessions and the explicit
    /// holidays — the source data is contradictory.
    #[error("date {date} is both a session and a holiday in {calendar_id}")]
    ConflictingHolidaySession { calendar_id: String, date: String },
    /// A Saturday/Sunday appears in the explicit sessions — weekends are
    /// never sessions.
    #[error("date {date} is a weekend but listed as a session in {calendar_id}")]
    WeekendSession { calendar_id: String, date: String },
}

/// A published KRX trading calendar: an immutable, versioned session dataset.
#[derive(Debug, Clone)]
pub struct KrCalendar {
    venue: Venue,
    timezone: Zone,
    session_times: SessionTimes,
    sessions: BTreeSet<TradingDate>,
    holidays: Vec<Holiday>,
    provenance: CalendarProvenance,
}

impl KrCalendar {
    /// Builds and validates a calendar version from its spec: sessions are
    /// deduplicated and sorted, weekend sessions and holiday/session conflicts
    /// are rejected, and the version's content hash is computed over the
    /// canonical serialization.
    pub fn build(spec: KrCalendarSpec) -> Result<Self, CalendarError> {
        let mut seen = BTreeSet::new();
        for date in &spec.sessions {
            if date.is_weekend() {
                return Err(CalendarError::WeekendSession {
                    calendar_id: spec.calendar_id.clone(),
                    date: date.to_iso(),
                });
            }
            if !seen.insert(*date) {
                // duplicate input rows are tolerated and deduplicated
            }
        }
        for holiday in &spec.holidays {
            if seen.contains(&holiday.date) {
                return Err(CalendarError::ConflictingHolidaySession {
                    calendar_id: spec.calendar_id.clone(),
                    date: holiday.date.to_iso(),
                });
            }
        }

        let sessions = seen;
        let mut holidays = spec.holidays.clone();
        holidays.sort_unstable();

        #[derive(Serialize)]
        struct Canonical {
            calendar_id: String,
            timezone: String,
            session_times: SessionTimes,
            sessions: Vec<TradingDate>,
            holidays: Vec<Holiday>,
            source: String,
            version: u32,
        }
        let canonical = Canonical {
            calendar_id: spec.calendar_id.clone(),
            timezone: spec.timezone.to_string(),
            session_times: spec.session_times,
            sessions: sessions.iter().copied().collect(),
            holidays: holidays.clone(),
            source: spec.source.clone(),
            version: spec.version,
        };
        let canonical_json = serde_json::to_vec(&canonical).expect("canonical form serializes");
        let content_hash = ContentHash::from_bytes(&canonical_json);

        Ok(Self {
            venue: Venue::Krx,
            timezone: spec.timezone,
            session_times: spec.session_times,
            sessions,
            holidays,
            provenance: CalendarProvenance {
                calendar_id: spec.calendar_id,
                source: spec.source,
                version: spec.version,
                content_hash,
                published_at: spec.published_at,
                timezone: spec.timezone,
                notes: spec.notes,
            },
        })
    }

    /// The venue of these sessions (KRX).
    pub fn venue(&self) -> Venue {
        self.venue
    }

    /// The IANA timezone of these sessions (Asia/Seoul).
    pub fn timezone(&self) -> Zone {
        self.timezone
    }

    /// The session window.
    pub fn session_times(&self) -> SessionTimes {
        self.session_times
    }

    /// The provenance of this calendar version.
    pub fn provenance(&self) -> &CalendarProvenance {
        &self.provenance
    }

    /// All explicit closure records.
    pub fn holidays(&self) -> &[Holiday] {
        &self.holidays
    }

    /// All materialized session dates, ascending.
    pub fn sessions(&self) -> impl Iterator<Item = TradingDate> + '_ {
        self.sessions.iter().copied()
    }

    /// Whether `date` is a session of this calendar (explicit data only).
    pub fn is_session(&self, date: TradingDate) -> bool {
        self.sessions.contains(&date)
    }

    /// The next session strictly after `date`.
    pub fn next_trading_day(&self, date: TradingDate) -> Result<TradingDate, CalendarError> {
        self.sessions
            .range((std::ops::Bound::Excluded(date), std::ops::Bound::Unbounded))
            .next()
            .copied()
            .ok_or_else(|| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: date.to_iso(),
                direction: "after",
            })
    }

    /// The previous session strictly before `date`.
    pub fn previous_trading_day(&self, date: TradingDate) -> Result<TradingDate, CalendarError> {
        self.sessions
            .range((std::ops::Bound::Unbounded, std::ops::Bound::Excluded(date)))
            .next_back()
            .copied()
            .ok_or_else(|| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: date.to_iso(),
                direction: "before",
            })
    }

    /// The last session of `year`/`month`. A holiday on the month's final
    /// weekday is never reported as a trading day (FR-DATA-003 month-end).
    pub fn last_trading_day_of_month(
        &self,
        year: i32,
        month: u32,
    ) -> Result<TradingDate, CalendarError> {
        let month_start =
            TradingDate::new(year, month, 1).map_err(|_| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: format!("{year:04}-{month:02}-01"),
                direction: "month",
            })?;
        self.sessions
            .range((
                std::ops::Bound::Included(month_start),
                std::ops::Bound::Unbounded,
            ))
            .take_while(|d| d.as_naive_date().month() == month)
            .last()
            .copied()
            .ok_or_else(|| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: format!("{year:04}-{month:02}"),
                direction: "month",
            })
    }

    /// The first session of `year`/`month`.
    pub fn first_trading_day_of_month(
        &self,
        year: i32,
        month: u32,
    ) -> Result<TradingDate, CalendarError> {
        let month_start =
            TradingDate::new(year, month, 1).map_err(|_| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: format!("{year:04}-{month:02}-01"),
                direction: "month",
            })?;
        self.sessions
            .range((
                std::ops::Bound::Included(month_start),
                std::ops::Bound::Unbounded,
            ))
            .take_while(|d| d.as_naive_date().month() == month)
            .next()
            .copied()
            .ok_or_else(|| CalendarError::OutsideCoveredRange {
                calendar_id: self.provenance.calendar_id.clone(),
                date: format!("{year:04}-{month:02}"),
                direction: "month",
            })
    }

    /// The timezone-aware UTC instant of the session open on `date`
    /// (2020-02-03 -> `2020-02-03T00:00:00Z`).
    pub fn session_open_utc(&self, date: TradingDate) -> Result<UtcTimestamp, CalendarError> {
        Ok(self.session_open_venue(date)?.to_utc())
    }

    /// The timezone-aware UTC instant of the session close on `date`
    /// (2020-02-03 -> `2020-02-03T06:30:00Z`).
    pub fn session_close_utc(&self, date: TradingDate) -> Result<UtcTimestamp, CalendarError> {
        Ok(self.session_close_venue(date)?.to_utc())
    }

    /// The venue-local session open (`2020-02-03T09:00:00+09:00`).
    pub fn session_open_local(&self, date: TradingDate) -> Result<VenueTimestamp, CalendarError> {
        self.session_open_venue(date)
    }

    /// The venue-local session close (`2020-02-03T15:30:00+09:00`).
    pub fn session_close_local(&self, date: TradingDate) -> Result<VenueTimestamp, CalendarError> {
        self.session_close_venue(date)
    }

    fn session_open_venue(&self, date: TradingDate) -> Result<VenueTimestamp, CalendarError> {
        self.check_session(date)?;
        let naive = date.as_naive_date().and_time(self.session_times.open);
        VenueTimestamp::from_naive_local(self.venue, naive).map_err(|_| {
            CalendarError::NotASession {
                calendar_id: self.provenance.calendar_id.clone(),
                date: date.to_iso(),
            }
        })
    }

    fn session_close_venue(&self, date: TradingDate) -> Result<VenueTimestamp, CalendarError> {
        self.check_session(date)?;
        let naive = date.as_naive_date().and_time(self.session_times.close);
        VenueTimestamp::from_naive_local(self.venue, naive).map_err(|_| {
            CalendarError::NotASession {
                calendar_id: self.provenance.calendar_id.clone(),
                date: date.to_iso(),
            }
        })
    }

    fn check_session(&self, date: TradingDate) -> Result<(), CalendarError> {
        if self.is_session(date) {
            Ok(())
        } else {
            Err(CalendarError::NotASession {
                calendar_id: self.provenance.calendar_id.clone(),
                date: date.to_iso(),
            })
        }
    }
}

/// The documented synthetic/official 2020 KRX calendar, version 1.
///
/// Sessions are materialized from the official weekday calendar minus the
/// explicit holiday closures below and locked by a content hash; queries
/// never infer sessions. No real KRX data rights exist — the closure set is
/// the documented official 2020 Korean exchange holidays.
pub fn krx_2020() -> KrCalendar {
    let holidays = [
        Holiday {
            date: TradingDate::new(2020, 1, 1).expect("valid date"),
            reason: "New Year's Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 1, 24).expect("valid date"),
            reason: "Seollal (lunar new year) break".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 1, 27).expect("valid date"),
            reason: "Seollal substitute holiday".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 4, 15).expect("valid date"),
            reason: "21st National Assembly election day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 4, 30).expect("valid date"),
            reason: "Buddha's Birthday".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 5, 5).expect("valid date"),
            reason: "Children's Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 6, 6).expect("valid date"),
            reason: "Memorial Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 8, 15).expect("valid date"),
            reason: "Liberation Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 9, 30).expect("valid date"),
            reason: "Chuseok (lunar thanksgiving)".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 10, 1).expect("valid date"),
            reason: "Chuseok (lunar thanksgiving)".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 10, 2).expect("valid date"),
            reason: "Chuseok (lunar thanksgiving)".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 10, 3).expect("valid date"),
            reason: "National Foundation Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 10, 9).expect("valid date"),
            reason: "Hangul Day".to_owned(),
        },
        Holiday {
            date: TradingDate::new(2020, 12, 25).expect("valid date"),
            reason: "Christmas Day".to_owned(),
        },
    ];

    let mut sessions: Vec<TradingDate> = Vec::new();
    let mut day = TradingDate::new(2020, 1, 1).expect("valid date");
    let year_end = TradingDate::new(2020, 12, 31).expect("valid date");
    while day <= year_end {
        if !day.is_weekend() && !holidays.iter().any(|h| h.date == day) {
            sessions.push(day);
        }
        day = day.next_day();
    }

    KrCalendar::build(KrCalendarSpec {
        calendar_id: "krx-2020".to_owned(),
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions,
        holidays: holidays.to_vec(),
        source: "krx-official-calendar-2020-v1".to_owned(),
        version: 1,
        published_at: UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z")
            .expect("valid published_at"),
        notes: vec![
            "Synthetic/documented official 2020 Korean exchange holiday set - no real KRX data rights.".to_owned(),
            "Sessions are materialized data (official weekdays minus explicit holidays), content-hashed per version; corrections produce v2+.".to_owned(),
            "Agrees with tests/fixtures/kr-etf/2020-01-31/calendar.json next_session_of and session times.".to_owned(),
        ],
    })
    .expect("krx 2020 calendar builds")
}
