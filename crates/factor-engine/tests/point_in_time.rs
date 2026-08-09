//! Point-in-time fundamentals: the answer depends on the date you ask from.
//!
//! These pin the property that a single-date fundamentals table cannot have.
//! The requirements' §14 risk table names 미래정보 참조 as a cause of 허위 성과;
//! the failure is silent by nature, because a backtest using a restated figure
//! produces a perfectly plausible number that the strategy could never have
//! earned. Each test below is a case where the naive implementation is wrong
//! and looks right.

use domain::{InstrumentId, TradingDate};
use factor_engine::fundamentals::Fundamentals;
use factor_engine::snapshot::FrozenUniverse;
use market_data::CurateStore;
use market_data::curate::schema::{CuratedFundamental, write_fundamentals};
use tempfile::tempdir;

const MARKET: &str = "kr";
const VERSION: u32 = 1;
const SYMBOL: &str = "A.KRX";
const METRIC: &str = "net_income";

fn d(iso: &str) -> TradingDate {
    TradingDate::parse(iso).expect("date")
}

fn id() -> InstrumentId {
    InstrumentId::parse(SYMBOL).expect("instrument id")
}

fn row(period_end: &str, value: f64, known_from: &str, revision: i64) -> CuratedFundamental {
    CuratedFundamental {
        instrument_id: id(),
        period_end: d(period_end),
        metric: METRIC.to_owned(),
        value,
        known_from: d(known_from),
        revision,
    }
}

/// One instrument's reporting history, carrying both kinds of restatement.
///
/// ```text
///   period    value  known_from   rev
///   Q1        100    2020-05-15   0
///   Q2        120    2020-08-14   0
///   Q2        110    2020-09-01   1   <- the CURRENT period, corrected
///   Q1         90    2020-10-01   1   <- an OLD period, corrected late
/// ```
///
/// The two restatements test different halves of the rule. Correcting the
/// current period is what makes one snapshot give two answers by date.
/// Correcting an old period LATER than the current one was published is what
/// separates "latest period" from "latest publication" -- ordering by
/// `known_from` alone answers 90 on 10-02, when Q2 is still current.
///
/// Every case below reads from this one fixture, because the cases only exist
/// in relation to each other.
fn fixture(dir: &std::path::Path) -> (CurateStore, FrozenUniverse) {
    let store = CurateStore::new(dir);
    let rows = vec![
        row("2020-03-31", 100.0, "2020-05-15", 0),
        row("2020-06-30", 120.0, "2020-08-14", 0),
        row("2020-06-30", 110.0, "2020-09-01", 1),
        row("2020-03-31", 90.0, "2020-10-01", 1),
    ];
    let path = store.fundamentals_path(MARKET, SYMBOL, VERSION);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    write_fundamentals(&path, &rows).expect("write fundamentals");
    (store, FrozenUniverse::new("universe-pit-1", &[SYMBOL]))
}

fn resolved(dir: &std::path::Path, as_of: &str) -> Fundamentals {
    let (store, universe) = fixture(dir);
    Fundamentals::from_curated(&store, MARKET, VERSION, &universe, d(as_of))
        .expect("fundamentals resolve")
}

/// THE test. One snapshot, two dates, two different answers.
///
/// A resolver that picked "the latest known value" once for the snapshot
/// would hand the 09-01 correction to a 06-01 bar and every 2020 decision
/// would improve retroactively. Both assertions come from the SAME resolved
/// object, because a per-snapshot resolver passes the second one alone.
#[test]
fn a_restatement_is_invisible_before_it_was_published() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");

    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-08-20")),
        Some(120.0),
        "an August reader must see Q2 as published, not the September correction"
    );
    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-09-30")),
        Some(110.0),
        "a September reader, after the correction, must see the corrected figure"
    );
}

/// A restatement of an OLD period must not shadow a newer period.
///
/// On 10-02 the newest row by publication date is Q1's October correction,
/// but the current period is still Q2. Ordering by `known_from` alone -- the
/// obvious single as-of join -- returns 90.0 here and is wrong: the strategy
/// would rebalance on a period it had already moved past.
#[test]
fn a_late_restatement_does_not_shadow_a_newer_period() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");

    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-10-02")),
        Some(110.0),
        "Q2 is still the current period after Q1 was restated in October"
    );
}

/// Before the first announcement the metric is ABSENT, not zero.
///
/// A factor that reads 0 for "has not reported yet" ranks that instrument as
/// though it had reported the worst possible result, and nothing downstream
/// can tell the two apart.
#[test]
fn before_the_first_announcement_there_is_no_value() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");

    assert_eq!(f.value_on(&id(), METRIC, d("2020-05-14")), None);
    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-01-02")),
        None,
        "the period had not even ended yet"
    );
}

/// `known_from` is the first date the value MAY be used, so its own date counts.
#[test]
fn the_known_from_date_itself_is_visible() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");

    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-05-15")),
        Some(100.0),
        "excluding its own date would delay every figure by one day"
    );
}

/// The snapshot ceiling: a snapshot taken before the correction cannot reach it.
///
/// Distinct from the per-date rule -- this is about what is LOADED at all. A
/// snapshot dated 2020-08-31 must be unable to produce 90.0 from any date,
/// including dates after the correction, because the correction did not exist
/// when the snapshot was frozen.
#[test]
fn a_snapshot_cannot_see_past_its_own_as_of() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-08-31");

    assert_eq!(f.value_on(&id(), METRIC, d("2020-09-30")), Some(120.0));
    assert_eq!(
        f.point_on(&id(), METRIC, d("2020-12-31"))
            .map(|p| p.value),
        Some(120.0),
        "the September restatement is not in this snapshot at all"
    );
    assert_eq!(
        f.value_on(&id(), METRIC, d("2020-06-01")),
        Some(100.0),
        "and the pre-Q2 answer is unchanged by the ceiling"
    );
}

/// The provenance survives, so a number can say which period produced it.
#[test]
fn a_resolved_value_carries_its_period_and_revision() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");

    let before = f.point_on(&id(), METRIC, d("2020-08-20")).expect("point");
    assert_eq!(before.period_end, d("2020-06-30"));
    assert_eq!(before.revision, 0);

    let after = f.point_on(&id(), METRIC, d("2020-09-30")).expect("point");
    assert_eq!(after.period_end, d("2020-06-30"));
    assert_eq!(after.revision, 1, "the correction is revision 1 of Q2");
}

/// A dataset with no fundamentals zone resolves empty rather than failing.
#[test]
fn a_missing_zone_is_empty_not_an_error() {
    let dir = tempdir().expect("temp");
    let store = CurateStore::new(dir.path());
    let universe = FrozenUniverse::new("universe-pit-1", &[SYMBOL]);
    let f = Fundamentals::from_curated(&store, MARKET, VERSION, &universe, d("2020-12-31"))
        .expect("a missing zone must not fail a snapshot");
    assert!(f.is_empty());
    assert_eq!(f.value_on(&id(), METRIC, d("2020-06-01")), None);
}

/// An unknown metric is absent, not an error and not a default.
#[test]
fn an_unreported_metric_is_absent() {
    let dir = tempdir().expect("temp");
    let f = resolved(dir.path(), "2020-12-31");
    assert_eq!(f.value_on(&id(), "book_value", d("2020-12-01")), None);
}
