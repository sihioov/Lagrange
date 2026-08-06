//! Todo 21 RED tests: missing-data policy (AT-05).
//!
//! AT-05: "특정 종목 일봉 누락 → 추천·백테스트가 경고 또는 차단 정책대로
//! 동작" — required-universe missing bars BLOCK the run (mirrors the T11
//! policy and the queue's `DataBlocked` class: never retried); strategy-
//! declared optional exclusion proceeds with a recorded reason.

mod common;

use result_model::robustness::{
    MissingDataOutcome, MissingDataPolicy, MissingInstrument, RobustnessError,
    apply_missing_data_policy, enforce_missing_data_policy,
};

fn missing_069500() -> Vec<MissingInstrument> {
    vec![MissingInstrument {
        instrument: "069500.KRX".to_owned(),
        missing_sessions: 5,
        last_observed: Some("2020-01-20".to_owned()),
    }]
}

#[test]
fn required_universe_missing_data_blocks_the_run() {
    let error = apply_missing_data_policy(&missing_069500(), MissingDataPolicy::RequiredUniverse)
        .expect_err("required-universe missing bars must BLOCK (AT-05)");
    match error {
        RobustnessError::DataBlocked { detail } => {
            assert!(detail.contains("069500.KRX"), "blocked detail must name the instrument");
        }
        other => panic!("expected DataBlocked, got: {other:?}"),
    }

    // The enforce flavor refuses to produce a result at all.
    let result = common::golden_result();
    let error =
        enforce_missing_data_policy(&result, &missing_069500(), MissingDataPolicy::RequiredUniverse)
            .expect_err("enforce must refuse the run entirely");
    assert!(matches!(error, RobustnessError::DataBlocked { .. }));
}

#[test]
fn optional_missing_data_excludes_with_recorded_reason() {
    let outcome =
        apply_missing_data_policy(&missing_069500(), MissingDataPolicy::OptionalExclude)
            .expect("optional exclusion must produce a policy outcome");
    match outcome {
        MissingDataOutcome::Warning { exclusions } => {
            assert_eq!(exclusions.len(), 1);
            assert_eq!(exclusions[0].instrument, "069500.KRX");
            assert!(!exclusions[0].reason.is_empty());
        }
        other => panic!("expected Warning outcome, got: {other:?}"),
    }

    // The enforce flavor attaches the structured warning to the result.
    let result = common::golden_result();
    let warned = enforce_missing_data_policy(&result, &missing_069500(), MissingDataPolicy::OptionalExclude)
        .expect("optional exclusion must still produce a result");
    assert!(warned.warnings.iter().any(|w| w.code == "missing_data_excluded"));
    let warning = warned
        .warnings
        .iter()
        .find(|w| w.code == "missing_data_excluded")
        .unwrap();
    let details = warning.details.as_ref().expect("warning carries details");
    assert_eq!(details["instrument"], "069500.KRX");
    assert_eq!(details["missing_sessions"], 5);
    assert_eq!(warning.severity, result_model::WarningSeverity::Warning);
}

#[test]
fn no_missing_data_passes_both_policies() {
    assert!(matches!(
        apply_missing_data_policy(&[], MissingDataPolicy::RequiredUniverse)
            .expect("empty missing list must pass"),
        MissingDataOutcome::Passed
    ));
    assert!(matches!(
        apply_missing_data_policy(&[], MissingDataPolicy::OptionalExclude)
            .expect("empty missing list must pass"),
        MissingDataOutcome::Passed
    ));
}

#[test]
fn multiple_missing_instruments_are_all_recorded() {
    let missing = vec![
        MissingInstrument {
            instrument: "069500.KRX".to_owned(),
            missing_sessions: 5,
            last_observed: None,
        },
        MissingInstrument {
            instrument: "229200.KRX".to_owned(),
            missing_sessions: 2,
            last_observed: Some("2020-01-28".to_owned()),
        },
    ];
    let outcome = apply_missing_data_policy(&missing, MissingDataPolicy::OptionalExclude)
        .expect("optional exclusion must produce a policy outcome");
    match outcome {
        MissingDataOutcome::Warning { exclusions } => {
            assert_eq!(exclusions.len(), 2);
            assert_eq!(exclusions[0].instrument, "069500.KRX");
            assert_eq!(exclusions[1].instrument, "229200.KRX");
        }
        other => panic!("expected Warning outcome, got: {other:?}"),
    }

    let error = apply_missing_data_policy(&missing, MissingDataPolicy::RequiredUniverse)
        .expect_err("required universe must block with ALL missing instruments named");
    match error {
        RobustnessError::DataBlocked { detail } => {
            assert!(detail.contains("069500.KRX") && detail.contains("229200.KRX"));
        }
        other => panic!("expected DataBlocked, got: {other:?}"),
    }
}
