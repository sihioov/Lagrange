//! Todo 21 RED tests: train/validation/test holdout (FR-ROB-001).
//!
//! The FINAL test period is never read during parameter selection:
//!   - [`HoldoutBarrier`] rejects any date past the validation end;
//!   - [`select_equity_series`] fails with `HoldoutViolation` the moment a
//!     test-period date appears in the selection input (a buggy selector
//!     feeding the full series is caught, never silently filtered);
//!   - the test segment is only reachable through the explicit
//!     [`SplitResult::test`] escape hatch (final evaluation only).

mod common;

use result_model::robustness::{
    HoldoutBarrier, PeriodSplit, RobustnessError, SplitResult, select_equity_series,
};

fn split() -> PeriodSplit {
    PeriodSplit {
        train_end: "2020-02-28".to_owned(),
        validation_end: "2020-03-31".to_owned(),
    }
}

/// Deterministic consecutive-date equity series 2020-01-02 .. 2020-04-30.
fn equity_series() -> Vec<(String, i64)> {
    let mut series = Vec::new();
    let (mut y, mut m, mut d) = (2020, 1, 2);
    loop {
        let date = format!("{y:04}-{m:02}-{d:02}");
        series.push((date.clone(), series.len() as i64 * 100 + 100_000_000));
        if date == "2020-04-30" {
            break;
        }
        d += 1;
        if d > 31 {
            d = 1;
            m += 1;
        }
        if m > 12 {
            m = 1;
            y += 1;
        }
    }
    series
}

#[test]
fn barrier_rejects_test_period_dates() {
    let barrier = HoldoutBarrier::new(&split());
    assert!(barrier.guard("2020-02-28").is_ok(), "train end is selectable");
    assert!(barrier.guard("2020-03-31").is_ok(), "validation end is selectable");

    let error = barrier.guard("2020-04-01").expect_err(
        "the final test period must NEVER be readable during selection (FR-ROB-001)",
    );
    match error {
        RobustnessError::HoldoutViolation { date } => {
            assert_eq!(date, "2020-04-01");
        }
        other => panic!("expected HoldoutViolation, got: {other:?}"),
    }
}

#[test]
fn selection_returns_only_train_and_validation() {
    let series = equity_series();
    // The selection pipeline receives a train+validation-restricted view;
    // selection over it succeeds and returns exactly the same points.
    let n_selection = series
        .iter()
        .filter(|(d, _)| d.as_str() <= "2020-03-31")
        .count();
    let selection_input = &series[..n_selection];
    let selected = select_equity_series(selection_input, &split())
        .expect("selection must succeed when no test point is read");
    assert_eq!(selected, selection_input);
    assert!(selected.iter().all(|(date, _)| date.as_str() <= "2020-03-31"));
    let n_train = series.iter().filter(|(d, _)| d.as_str() <= "2020-02-28").count();
    let n_validation = series
        .iter()
        .filter(|(d, _)| d.as_str() > "2020-02-28" && d.as_str() <= "2020-03-31")
        .count();
    assert_eq!(selected.len(), n_train + n_validation);
}

#[test]
fn selection_with_test_period_read_is_rejected() {
    let series = equity_series();
    // A (buggy) selector feeds the FULL series — including the final test
    // period. The barrier must reject the read and name the first test date
    // instead of silently filtering (FR-ROB-001 violation proof).
    let error = select_equity_series(&series, &split()).expect_err(
        "selection must fail when test-period data would be read (FR-ROB-001 violation)",
    );
    match error {
        RobustnessError::HoldoutViolation { date } => {
            assert_eq!(date, "2020-04-01", "first test-period date must be named");
        }
        other => panic!("expected HoldoutViolation, got: {other:?}"),
    }
}

#[test]
fn split_result_exposes_segments_with_guarded_test() {
    let series = equity_series();
    let result = SplitResult::new(&series, &split()).expect("split over a full series succeeds");
    assert_eq!(
        result.train().last().map(|(d, _)| d.as_str()),
        Some("2020-02-28")
    );
    assert_eq!(
        result.validation().last().map(|(d, _)| d.as_str()),
        Some("2020-03-31")
    );

    // The test segment is reachable only through the explicit escape hatch.
    let test = result.test();
    assert!(test.iter().all(|(d, _)| d.as_str() > "2020-03-31"));
}

#[test]
fn invalid_split_boundaries_are_rejected() {
    let bad = PeriodSplit {
        train_end: "2020-04-30".to_owned(),
        validation_end: "2020-02-28".to_owned(),
    };
    let error = SplitResult::new(&equity_series(), &bad)
        .expect_err("train_end after validation_end must be rejected as a typed error");
    assert!(matches!(error, RobustnessError::InvalidSplit { .. }));
}
