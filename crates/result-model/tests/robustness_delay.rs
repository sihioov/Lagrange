//! Todo 21 RED tests: execution delay (design §9.5 `ExecutionDelay`).
//!
//! A delayed run shifts every fill `delay_sessions` sessions later on the
//! deterministic session calendar (sorted unique fill dates), rebuilds the
//! full result through the shared replay (integrity must hold), and still
//! pins the parent context. Delay changes WHEN the execution is experienced
//! (re-dates the equity curve); fills/prices/fees are untouched, so the
//! terminal value is preserved — that is the honest timing-risk semantic.
//! A fill whose shifted session would fall beyond the last session settles
//! at the last session (the run window ends there).

mod common;

use result_model::robustness::{RobustnessError, delay_execution};

#[test]
fn delay_shifts_fills_by_the_declared_sessions() {
    let base = common::golden_result();
    let delayed = delay_execution(&base, 1).expect("one-session delay must succeed");
    delayed
        .validate()
        .expect("delayed result must pass integrity checks");

    // Session calendar = sorted unique fill dates:
    // 01-02, 01-03, 01-06, 01-09, 01-14, 01-16 (six sessions)
    // Each fill moves to the next session; the last-session fill settles at
    // the window end (documented clamp).
    let expected_dates = [
        "2020-01-03", "2020-01-06", "2020-01-09", "2020-01-14", "2020-01-16", "2020-01-16",
    ];
    for (i, fill) in delayed.fills.iter().enumerate() {
        assert_eq!(
            &fill.ts.to_rfc3339()[..10],
            expected_dates[i],
            "fill {} must land on the next session",
            fill.fill_id
        );
    }
    // Fees follow the shifted fills (parallel entries, same dates).
    for (fee, fill) in delayed.fees.iter().zip(delayed.fills.iter()) {
        assert_eq!(fee.ts, fill.ts);
    }
}

#[test]
fn delay_is_identical_for_identical_inputs() {
    let base = common::golden_result();
    let a = delay_execution(&base, 2).unwrap();
    let b = delay_execution(&base, 2).unwrap();
    assert_eq!(a.equity, b.equity);
    assert_eq!(a.fills, b.fills);
    a.validate().unwrap();
}

#[test]
fn delay_redates_the_curve_and_preserves_terminal_value() {
    let base = common::golden_result();
    let delayed = delay_execution(&base, 2).unwrap();
    // First fill shifts 01-02 -> 01-06 (+2 sessions), so day-0 sits 01-05.
    assert_eq!(delayed.summary.start_date, "2020-01-05");
    assert_eq!(delayed.summary.end_date, "2020-01-16");

    // Delay moves TIMING, not fills/prices/fees: terminal value is preserved
    // while the equity curve is re-dated.
    assert_eq!(
        delayed.summary.final_equity,
        base.summary.final_equity,
        "execution delay must not change fills, prices, or fees"
    );
    assert_ne!(
        delayed.equity, base.equity,
        "execution delay must re-date the equity curve"
    );
    assert_eq!(
        &delayed.equity[0].ts.to_rfc3339()[..10],
        "2020-01-05",
        "day-0 point must sit the day before the first shifted fill"
    );
}

#[test]
fn delay_beyond_the_session_calendar_is_a_typed_error() {
    let base = common::golden_result();
    let error = delay_execution(&base, 10)
        .expect_err("a delay beyond the session calendar must be rejected");
    assert!(matches!(error, RobustnessError::DelayOutOfRange { .. }));
}

#[test]
fn zero_delay_is_the_identity() {
    let base = common::golden_result();
    let delayed = delay_execution(&base, 0).unwrap();
    assert_eq!(delayed.equity, base.equity);
    assert_eq!(delayed.fills, base.fills);
}
