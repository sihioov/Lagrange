//! Todo 21 RED tests: period split and walk-forward (FR-ROB-001/004).
//!
//! The train/validation/test segments are disjoint and exhaustive, their
//! metrics are deterministic (hand-verified on the golden scenario), and the
//! walk-forward plan produces complete (train, validation) folds. Oversized
//! plans and invalid boundaries are typed errors, never panics.

mod common;

use result_model::robustness::{
    PeriodSplit, RobustnessError, split_period, walk_forward, WalkForwardPlan,
};

fn golden_points() -> Vec<String> {
    common::golden_result()
        .equity
        .iter()
        .map(|p| p.ts.to_rfc3339()[..10].to_owned())
        .collect()
}

#[test]
fn period_split_segments_are_disjoint_and_exhaustive() {
    let result = common::golden_result();
    let split = PeriodSplit {
        train_end: "2020-01-08".to_owned(),
        validation_end: "2020-01-13".to_owned(),
    };
    let segments = split_period(&result, &split).expect("split must succeed");

    // train = all points <= 01-08; validation = (01-08, 01-13]; test = > 01-13
    let train_dates: Vec<String> = segments.train.points.iter().map(|p| p.ts.to_rfc3339()[..10].to_owned()).collect();
    let validation_dates: Vec<String> = segments.validation.points.iter().map(|p| p.ts.to_rfc3339()[..10].to_owned()).collect();
    let test_dates: Vec<String> = segments.test.points.iter().map(|p| p.ts.to_rfc3339()[..10].to_owned()).collect();

    assert_eq!(train_dates, vec!["2020-01-01", "2020-01-02", "2020-01-03", "2020-01-06"]);
    assert_eq!(validation_dates, vec!["2020-01-09"]);
    assert_eq!(test_dates, vec!["2020-01-14", "2020-01-16"]);

    // exhaustive: every golden point lands in exactly one segment
    let mut seen: Vec<String> = train_dates.iter().chain(validation_dates.iter()).chain(test_dates.iter()).cloned().collect();
    seen.sort_unstable();
    let mut all = golden_points();
    all.sort_unstable();
    assert_eq!(seen, all);
}

#[test]
fn segment_metrics_are_deterministic_and_hand_verified() {
    let result = common::golden_result();
    let split = PeriodSplit {
        train_end: "2020-01-08".to_owned(),
        validation_end: "2020-01-13".to_owned(),
    };
    let segments = split_period(&result, &split).unwrap();

    // train: 4 points, 9,998,400 / 10,000,000 - 1 = -0.00016
    assert_eq!(segments.train.metrics.n_points, 4);
    assert_eq!(segments.train.metrics.start_date, "2020-01-01");
    assert_eq!(segments.train.metrics.end_date, "2020-01-06");
    assert!((segments.train.metrics.total_return.value() + 0.00016).abs() < 1e-9);

    // validation: single point -> zero return
    assert_eq!(segments.validation.metrics.n_points, 1);
    assert_eq!(segments.validation.metrics.total_return.value(), 0.0);

    // test: 9,921,987 / 10,022,430 - 1
    let expected_test = 9_921_987.0 / 10_022_430.0 - 1.0;
    assert!((segments.test.metrics.total_return.value() - expected_test).abs() < 1e-9);
    // every metric must be finite (ReportedStat construction already rejects NaN)
    assert!(segments.test.metrics.max_drawdown.value().is_finite());
    assert!(segments.test.metrics.volatility.value().is_finite());
}

#[test]
fn walk_forward_folds_cover_expected_windows() {
    let result = common::golden_result();
    let plan = WalkForwardPlan {
        window_sessions: 3,
        step_sessions: 2,
    };
    let folds = walk_forward(&result, &plan).expect("plan fits the 7-point curve");
    assert_eq!(folds.len(), 2, "expected exactly two complete folds");

    // fold 0: train points 0..3, validation points 3..5
    let fold0_dates: Vec<String> = folds[0]
        .train
        .points
        .iter()
        .map(|p| p.ts.to_rfc3339()[..10].to_owned())
        .collect();
    let all = golden_points();
    assert_eq!(fold0_dates, all[0..3]);
    let fold0_val: Vec<String> = folds[0]
        .validation
        .points
        .iter()
        .map(|p| p.ts.to_rfc3339()[..10].to_owned())
        .collect();
    assert_eq!(fold0_val, all[3..5]);

    // fold 1: train points 2..5, validation points 5..7
    let fold1_dates: Vec<String> = folds[1]
        .train
        .points
        .iter()
        .map(|p| p.ts.to_rfc3339()[..10].to_owned())
        .collect();
    assert_eq!(fold1_dates, all[2..5]);
    let fold1_val: Vec<String> = folds[1]
        .validation
        .points
        .iter()
        .map(|p| p.ts.to_rfc3339()[..10].to_owned())
        .collect();
    assert_eq!(fold1_val, all[5..7]);

    // per-fold metrics are finite and deterministic
    for fold in &folds {
        assert!(fold.train.metrics.total_return.value().is_finite());
        assert!(fold.validation.metrics.total_return.value().is_finite());
    }
}

#[test]
fn walk_forward_plan_too_large_is_a_typed_error() {
    let result = common::golden_result();
    let plan = WalkForwardPlan {
        window_sessions: 10,
        step_sessions: 2,
    };
    let error = walk_forward(&result, &plan)
        .expect_err("a window larger than the data must be a typed error");
    assert!(matches!(error, RobustnessError::InsufficientData { .. }));
}

#[test]
fn walk_forward_rejects_zero_step_or_window() {
    let result = common::golden_result();
    let error = walk_forward(
        &result,
        &WalkForwardPlan {
            window_sessions: 0,
            step_sessions: 2,
        },
    )
    .expect_err("zero window must be rejected");
    assert!(matches!(error, RobustnessError::InsufficientData { .. }));

    let error = walk_forward(
        &result,
        &WalkForwardPlan {
            window_sessions: 2,
            step_sessions: 0,
        },
    )
    .expect_err("zero step must be rejected");
    assert!(matches!(error, RobustnessError::InsufficientData { .. }));
}

#[test]
fn period_split_respects_the_same_barrier_as_selection() {
    // The validation segment boundary matches the holdout selection barrier:
    // dates past validation_end belong to the test segment only.
    let result = common::golden_result();
    let split = PeriodSplit {
        train_end: "2020-01-08".to_owned(),
        validation_end: "2020-01-13".to_owned(),
    };
    let segments = split_period(&result, &split).unwrap();
    assert!(segments
        .validation
        .points
        .iter()
        .all(|p| p.ts.to_rfc3339()[..10] <= *"2020-01-13"));
    assert!(segments
        .test
        .points
        .iter()
        .all(|p| p.ts.to_rfc3339()[..10] > *"2020-01-13"));
}
