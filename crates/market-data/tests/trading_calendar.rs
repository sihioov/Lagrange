//! Todo 9 red/green integration tests: the KRX Korean trading calendar.
//!
//! Acceptance contract (plan Todo 9 + FR-DATA-003 + design §6.4):
//! - Asia/Seoul timezone, explicit sessions 09:00-15:30 KST, no DST;
//! - holidays are explicit versioned data (never derived from weekdays) with
//!   source/version/hash provenance;
//! - `next_trading_day(2020-01-31) == 2020-02-03` (fixture-agreed) and
//!   `next_trading_day(2020-01-23) == 2020-01-28` (Seollal break);
//! - timezone-aware open/close instants for any trading date;
//! - holiday month-end: the last session of a month whose final weekdays are
//!   holidays is the last weekday before them;
//! - corrections create a new version (new content hash), never in-place
//!   mutation of the published calendar.

use std::path::Path;

use market_data::calendar::{
    CalendarError, CalendarProvenance, Holiday, KrCalendar, KrCalendarSpec, SessionTimes,
    krx_2020,
};

use domain::{ContentHash, TradingDate, UtcTimestamp, Venue, Zone};

fn d(s: &str) -> TradingDate {
    TradingDate::parse(s).unwrap()
}

fn calendar_json_fixture() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/kr-etf/2020-01-31/calendar.json");
    let text = std::fs::read_to_string(&path).expect("todo6 calendar fixture must exist");
    serde_json::from_str(&text).expect("fixture must be valid JSON")
}

#[test]
fn next_trading_day_2020_01_31_is_2020_02_03() {
    // The 2020-01-31 (Friday) close signal fills at the next KRX session,
    // which is 2020-02-03 (Monday) per the Todo 6 fixture semantics.
    let cal = krx_2020();
    assert_eq!(cal.next_trading_day(d("2020-01-31")).unwrap(), d("2020-02-03"));
}

#[test]
fn seollal_break_next_session() {
    // 2020-01-23 (Thursday) is followed by the Seollal break
    // (2020-01-24 Friday holiday, 2020-01-27 Monday substitute holiday),
    // so the next session is 2020-01-28 (Tuesday).
    let cal = krx_2020();
    assert_eq!(cal.next_trading_day(d("2020-01-23")).unwrap(), d("2020-01-28"));
    assert!(!cal.is_session(d("2020-01-24")));
    assert!(!cal.is_session(d("2020-01-27")));
}

#[test]
fn sessions_are_explicit_data_not_weekday_inference() {
    // Holidays are explicit data, never derived from weekdays, and weekends
    // are never sessions. A Friday that is a holiday is not a session even
    // though it is a weekday.
    let cal = krx_2020();

    // Documented 2020 official KRX closure dates that fall on weekdays.
    for (date, _reason) in [
        (d("2020-01-01"), "new year"),
        (d("2020-01-24"), "seollal"),
        (d("2020-01-27"), "seollal substitute"),
        (d("2020-04-15"), "election day"),
        (d("2020-04-30"), "buddha birthday"),
        (d("2020-05-05"), "children day"),
        (d("2020-09-30"), "chuseok"),
        (d("2020-10-01"), "chuseok"),
        (d("2020-10-02"), "chuseok"),
        (d("2020-10-09"), "hangul day"),
        (d("2020-12-25"), "christmas"),
    ] {
        assert!(!cal.is_session(date), "{date} must not be a session ({_reason})");
    }

    // Weekends are not sessions.
    assert!(!cal.is_session(d("2020-02-01")));
    assert!(!cal.is_session(d("2020-02-02")));

    // Ordinary weekdays are sessions.
    assert!(cal.is_session(d("2020-01-31")));
    assert!(cal.is_session(d("2020-02-03")));

    // Session records carry the documented reason.
    let reasons: Vec<&str> = cal
        .holidays()
        .iter()
        .filter(|h| h.date == d("2020-01-24"))
        .map(|h| h.reason.as_str())
        .collect();
    assert_eq!(reasons, vec!["Seollal (lunar new year) break"]);
}

#[test]
fn timezone_aware_open_close_instants() {
    // 09:00-15:30 KST == 00:00-06:30 UTC (Asia/Seoul, no DST, +09:00).
    let cal = krx_2020();
    let session = d("2020-02-03");

    let open_utc = cal.session_open_utc(session).unwrap();
    let close_utc = cal.session_close_utc(session).unwrap();
    assert_eq!(open_utc.to_rfc3339(), "2020-02-03T00:00:00Z");
    assert_eq!(close_utc.to_rfc3339(), "2020-02-03T06:30:00Z");

    let open_local = cal.session_open_local(session).unwrap();
    let close_local = cal.session_close_local(session).unwrap();
    assert_eq!(open_local.to_rfc3339(), "2020-02-03T09:00:00+09:00");
    assert_eq!(close_local.to_rfc3339(), "2020-02-03T15:30:00+09:00");

    // The venue is KRX; the timezone is Asia/Seoul.
    assert_eq!(cal.venue(), Venue::Krx);
    assert_eq!(cal.timezone(), Zone::SEOUL);

    // Querying a non-session date is a typed error, not a panic.
    assert!(matches!(
        cal.session_open_utc(d("2020-02-01")),
        Err(CalendarError::NotASession { .. })
    ));
}

#[test]
fn previous_trading_day() {
    let cal = krx_2020();
    // Back over the weekend.
    assert_eq!(cal.previous_trading_day(d("2020-02-03")).unwrap(), d("2020-01-31"));
    // Back over the Seollal break.
    assert_eq!(cal.previous_trading_day(d("2020-01-28")).unwrap(), d("2020-01-23"));
}

#[test]
fn holiday_month_end() {
    // 2020-09-30 (Wednesday) is Chuseok, so the last trading day of
    // September 2020 is 2020-09-29 (Tuesday) — a holiday month-end must not
    // report the holiday as a trading day.
    let cal = krx_2020();
    assert_eq!(
        cal.last_trading_day_of_month(2020, 9).unwrap(),
        d("2020-09-29")
    );
    // Normal month end.
    assert_eq!(
        cal.last_trading_day_of_month(2020, 2).unwrap(),
        d("2020-02-28")
    );
    assert_eq!(
        cal.last_trading_day_of_month(2020, 10).unwrap(),
        d("2020-10-30")
    );
    assert_eq!(
        cal.first_trading_day_of_month(2020, 1).unwrap(),
        d("2020-01-02")
    );
}

#[test]
fn calendar_agrees_with_todo6_fixture() {
    // The Todo 6 fixture `tests/fixtures/kr-etf/2020-01-31/calendar.json`
    // encodes the 2020-01-31 -> 2020-02-03 next-session and the Seollal break
    // as data. The real KRX calendar must agree with it.
    let fixture = calendar_json_fixture();
    let cal = krx_2020();

    // Every session listed in the fixture is a session in the calendar.
    for session in fixture["sessions"].as_array().unwrap() {
        let date = d(session["date"].as_str().unwrap());
        assert!(cal.is_session(date), "fixture session {date} must be a session");
    }
    // Every fixture holiday is not a session.
    for holiday in fixture["holidays"].as_array().unwrap() {
        let date = d(holiday["date"].as_str().unwrap());
        assert!(!cal.is_session(date), "fixture holiday {date} must not be a session");
    }
    // The fixture's next_session_of mapping holds exactly.
    let next_of = fixture["next_session_of"].as_object().unwrap();
    for (given, expected) in next_of {
        let given = d(given);
        let expected = d(expected.as_str().unwrap());
        assert_eq!(
            cal.next_trading_day(given).unwrap(),
            expected,
            "next session of {given} must be {expected}"
        );
    }
    // Session times agree: open 00:00Z / close 06:30Z.
    assert_eq!(
        cal.session_open_utc(d("2020-01-31")).unwrap().to_rfc3339(),
        "2020-01-31T00:00:00Z"
    );
    assert_eq!(
        cal.session_close_utc(d("2020-01-31")).unwrap().to_rfc3339(),
        "2020-01-31T06:30:00Z"
    );
    assert_eq!(
        cal.session_open_utc(d("2020-02-03")).unwrap().to_rfc3339(),
        "2020-02-03T00:00:00Z"
    );
}

#[test]
fn calendar_provenance_is_versioned_and_hashed() {
    let cal = krx_2020();
    let prov = cal.provenance();
    assert_eq!(prov.source, "krx-official-calendar-2020-v1");
    assert_eq!(prov.version, 1);
    assert_eq!(prov.timezone, Zone::SEOUL);
    assert_eq!(prov.content_hash.algorithm(), "sha256");

    // Deterministic: rebuilding the same calendar yields the same hash.
    let rebuilt = krx_2020();
    assert_eq!(rebuilt.provenance().content_hash, prov.content_hash);
}

#[test]
fn calendar_correction_creates_new_version_not_mutation() {
    // stale_state: a correction (e.g. a previously unknown holiday) creates a
    // new version with a new content hash — the published v1 calendar is
    // never mutated in place.
    let v1 = krx_2020();
    let before = v1.next_trading_day(d("2020-06-05")).unwrap();

    // v2: same base spec + one additional closure date (2020-06-05, Friday).
    let base = KrCalendarSpec::krx_2020_v2_with_additional_holiday(
        Holiday {
            date: d("2020-06-05"),
            reason: "unexpected market closure (correction)".to_owned(),
        },
        UtcTimestamp::parse_rfc3339("2020-06-01T00:00:00Z").unwrap(),
    );
    let v2 = KrCalendar::build(base).unwrap();

    assert_eq!(v2.provenance().version, 2);
    assert_eq!(v2.provenance().source, "krx-official-calendar-2020-v2");
    assert_ne!(v2.provenance().content_hash, v1.provenance().content_hash);
    assert!(!v2.is_session(d("2020-06-05")));
    // The correction changes the next session...
    assert_eq!(v2.next_trading_day(d("2020-06-04")).unwrap(), d("2020-06-08"));
    // ...while the published v1 calendar is unchanged (no in-place mutation).
    assert!(v1.is_session(d("2020-06-05")));
    assert_eq!(v1.next_trading_day(d("2020-06-05")).unwrap(), before);
}

#[test]
fn build_rejects_holiday_that_is_also_a_session() {
    // A date cannot be both an explicit holiday and an explicit session.
    let spec = KrCalendarSpec::krx_2020_v2_with_additional_holiday(
        Holiday {
            date: d("2020-06-05"),
            reason: "conflicting record".to_owned(),
        },
        UtcTimestamp::parse_rfc3339("2020-06-01T00:00:00Z").unwrap(),
    );
    // The v2 spec builder already removes the date from sessions; a raw spec
    // that lists the same date in both must be rejected at build time.
    let raw = KrCalendarSpec {
        calendar_id: "krx-2020-conflict".to_owned(),
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions: vec![d("2020-06-05"), d("2020-06-08")],
        holidays: vec![Holiday {
            date: d("2020-06-05"),
            reason: "conflicting record".to_owned(),
        }],
        source: "krx-official-calendar-2020-conflict".to_owned(),
        version: 1,
        published_at: UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").unwrap(),
        notes: vec![],
    };
    assert!(matches!(
        KrCalendar::build(raw),
        Err(CalendarError::ConflictingHolidaySession { .. })
    ));
}

#[test]
fn build_rejects_weekend_sessions() {
    // Sessions are explicit data and weekends are never sessions.
    let raw = KrCalendarSpec {
        calendar_id: "krx-2020-weekend".to_owned(),
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions: vec![d("2020-02-01")],
        holidays: vec![],
        source: "krx-official-calendar-2020-weekend".to_owned(),
        version: 1,
        published_at: UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").unwrap(),
        notes: vec![],
    };
    assert!(matches!(
        KrCalendar::build(raw),
        Err(CalendarError::WeekendSession { .. })
    ));
}

#[test]
fn outside_range_is_typed_error() {
    let cal = krx_2020();
    // No session exists after 2020-12-31 (calendar covers 2020).
    assert!(matches!(
        cal.next_trading_day(d("2020-12-31")),
        Err(CalendarError::OutsideCoveredRange { .. })
    ));
    assert!(matches!(
        cal.previous_trading_day(d("2020-01-01")),
        Err(CalendarError::OutsideCoveredRange { .. })
    ));
}

#[test]
fn provenance_round_trip() {
    // CalendarProvenance serializes with the documented fields.
    let prov = krx_2020().provenance().clone();
    let json = serde_json::to_string(&prov).unwrap();
    let back: CalendarProvenance = serde_json::from_str(&json).unwrap();
    assert_eq!(back, prov);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["source"], "krx-official-calendar-2020-v1");
    assert_eq!(value["version"], 1);
    assert_eq!(value["timezone"], "Asia/Seoul");
    assert!(value["content_hash"].as_str().unwrap().starts_with("sha256:"));
}

#[test]
fn content_hash_deterministic_across_builds() {
    // The same spec produces the same content hash (byte-stable provenance).
    let h1 = KrCalendar::build(KrCalendarSpec {
        calendar_id: "krx-2020-det".to_owned(),
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions: vec![d("2020-01-02"), d("2020-01-03")],
        holidays: vec![],
        source: "krx-official-calendar-2020-det".to_owned(),
        version: 1,
        published_at: UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").unwrap(),
        notes: vec![],
    })
    .unwrap()
    .provenance()
    .content_hash
    .clone();
    let h2 = KrCalendar::build(KrCalendarSpec {
        calendar_id: "krx-2020-det".to_owned(),
        timezone: Zone::SEOUL,
        session_times: SessionTimes::krx_default(),
        sessions: vec![d("2020-01-03"), d("2020-01-02")], // different input order
        holidays: vec![],
        source: "krx-official-calendar-2020-det".to_owned(),
        version: 1,
        published_at: UtcTimestamp::parse_rfc3339("2020-01-01T00:00:00Z").unwrap(),
        notes: vec![],
    })
    .unwrap()
    .provenance()
    .content_hash
    .clone();
    assert_eq!(h1, h2);
    let _ = ContentHash::parse(h1.as_str()).unwrap();
}
