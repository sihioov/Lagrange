//! Todo 32: backtest-vs-Paper signal parity (design §10.2, §15.3;
//! requirements FR-PAPER-004, FR-RPT-001).
//!
//! Design §10.2 requires the report to SHOW the fill-model difference
//! between Paper and backtest rather than smooth it over, and §15.3 grades
//! a Paper divergence as WARNING. So parity has three distinct outcomes,
//! not two:
//!
//! - `Match`      — same lineage, identical signals.
//! - `Divergent`  — same lineage, different signals. A real finding.
//! - `NotComparable` — the two sides were produced from different
//!   strategy/data/as-of inputs, so no parity claim is meaningful at all.
//!   Reporting this as "divergent" would be a lie about what was compared.
//!
//! Signals are scale-6 fixed-point weights on both sides, produced by the
//! same deterministic selector, so comparison is EXACT: a tolerance would
//! only hide bugs the golden guarantees already exclude.

use std::collections::BTreeMap;

use domain::provenance::{Engine, RandomSeed, RunProvenance};
use domain::version::{SemVer, StrategyVersion};
use domain::{CodeCommit, ContentHash, DatasetVersionId, InstrumentId, StrategyId, Weight, Zone};

use result_model::paper_parity::{ParityStatus, SignalSet, evaluate_parity};

fn instrument(symbol: &str) -> InstrumentId {
    InstrumentId::parse(symbol).expect("valid instrument")
}

fn weight(value: &str) -> Weight {
    Weight::parse(value).expect("valid weight")
}

fn provenance(strategy_version: &str, dataset: &str, engine_version: &str) -> RunProvenance {
    RunProvenance {
        engine: Engine::NautilusTrader,
        engine_version: SemVer::parse(engine_version).unwrap(),
        strategy_id: StrategyId::parse("dual_momentum").unwrap(),
        strategy_version: StrategyVersion::parse(strategy_version).unwrap(),
        dataset_version: DatasetVersionId::parse(dataset).unwrap(),
        config_hash: ContentHash::from_bytes(b"config"),
        code_commit: CodeCommit::parse("0123456789abcdef").unwrap(),
        random_seed: RandomSeed::new(42),
        timezone: Zone::SEOUL,
    }
}

fn signals(pairs: &[(&str, &str)]) -> BTreeMap<InstrumentId, Weight> {
    pairs
        .iter()
        .map(|(id, w)| (instrument(id), weight(w)))
        .collect()
}

fn backtest_side() -> SignalSet {
    SignalSet {
        provenance: provenance("1.2.0", "kr-etf-daily-20260804.1", "1.231.0"),
        as_of: "2026-01-30".to_owned(),
        targets: signals(&[("069500.KRX", "0.600000"), ("229200.KRX", "0.400000")]),
    }
}

fn paper_side() -> SignalSet {
    SignalSet {
        provenance: provenance("1.2.0", "kr-etf-daily-20260804.1", "1.231.0"),
        as_of: "2026-01-30".to_owned(),
        targets: signals(&[("069500.KRX", "0.600000"), ("229200.KRX", "0.400000")]),
    }
}

// ---------------------------------------------------------------------------
// Match
// ---------------------------------------------------------------------------

#[test]
fn identical_lineage_and_signals_report_a_match() {
    let report = evaluate_parity(&backtest_side(), &paper_side());
    assert_eq!(report.status, ParityStatus::Match);
    assert!(
        report.divergences.is_empty(),
        "a match has no divergences to explain"
    );
    assert!(
        report.lineage.matches(),
        "every compared lineage field agrees"
    );
}

#[test]
fn the_fill_model_difference_is_always_reported_even_on_a_match() {
    // Design §10.2: "Paper와 백테스트의 체결 모델 차이를 리포트에 표시한다".
    // The note is unconditional -- a matching report that hid it would let a
    // reader assume the two executions are interchangeable.
    let report = evaluate_parity(&backtest_side(), &paper_side());
    assert_eq!(report.status, ParityStatus::Match);
    assert!(
        !report.fill_model_difference.is_empty(),
        "the fill-model difference is stated on EVERY report, match included"
    );
}

// ---------------------------------------------------------------------------
// Divergent: same lineage, different signals
// ---------------------------------------------------------------------------

#[test]
fn a_changed_weight_is_a_divergence_with_both_sides_named() {
    let mut paper = paper_side();
    paper
        .targets
        .insert(instrument("069500.KRX"), weight("0.550000"));

    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(report.status, ParityStatus::Divergent);
    assert_eq!(report.divergences.len(), 1);
    let d = &report.divergences[0];
    assert_eq!(d.instrument_id, instrument("069500.KRX"));
    assert_eq!(d.backtest_weight, Some(weight("0.600000")));
    assert_eq!(d.paper_weight, Some(weight("0.550000")));
}

#[test]
fn an_instrument_on_only_one_side_is_a_divergence_not_a_silent_skip() {
    let mut paper = paper_side();
    paper.targets.remove(&instrument("229200.KRX"));
    paper
        .targets
        .insert(instrument("114260.KRX"), weight("0.400000"));

    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(report.status, ParityStatus::Divergent);

    let missing = report
        .divergences
        .iter()
        .find(|d| d.instrument_id == instrument("229200.KRX"))
        .expect("an instrument the backtest held and Paper did not is reported");
    assert_eq!(missing.backtest_weight, Some(weight("0.400000")));
    assert_eq!(missing.paper_weight, None);

    let extra = report
        .divergences
        .iter()
        .find(|d| d.instrument_id == instrument("114260.KRX"))
        .expect("an instrument Paper held and the backtest did not is reported");
    assert_eq!(extra.backtest_weight, None);
    assert_eq!(extra.paper_weight, Some(weight("0.400000")));
}

#[test]
fn parity_is_exact_a_one_ulp_weight_difference_still_diverges() {
    // Both sides come from the same deterministic selector at scale 6, so
    // ANY difference is a real finding. A tolerance would hide bugs the
    // golden guarantees already exclude.
    let mut paper = paper_side();
    paper
        .targets
        .insert(instrument("069500.KRX"), weight("0.599999"));

    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(
        report.status,
        ParityStatus::Divergent,
        "signal parity is exact, never approximate"
    );
}

// ---------------------------------------------------------------------------
// NotComparable: the lineage itself differs
// ---------------------------------------------------------------------------

#[test]
fn a_changed_strategy_version_is_not_comparable_rather_than_divergent() {
    let paper = SignalSet {
        provenance: provenance("1.3.0", "kr-etf-daily-20260804.1", "1.231.0"),
        ..paper_side()
    };
    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(report.status, ParityStatus::NotComparable);
    assert!(
        report
            .lineage
            .mismatched_fields()
            .contains(&"strategy_version"),
        "the report names WHICH field made the comparison meaningless"
    );
}

#[test]
fn a_stale_as_of_is_not_comparable_and_names_the_field() {
    let paper = SignalSet {
        as_of: "2026-01-29".to_owned(),
        ..paper_side()
    };
    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(report.status, ParityStatus::NotComparable);
    assert!(report.lineage.mismatched_fields().contains(&"as_of"));
}

#[test]
fn a_changed_dataset_version_is_not_comparable() {
    let paper = SignalSet {
        provenance: provenance("1.2.0", "kr-etf-daily-20260805.1", "1.231.0"),
        ..paper_side()
    };
    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(report.status, ParityStatus::NotComparable);
    assert!(
        report
            .lineage
            .mismatched_fields()
            .contains(&"dataset_version")
    );
}

#[test]
fn identical_signals_under_a_changed_lineage_are_still_not_comparable() {
    // The dangerous case: the numbers happen to agree, but they were
    // produced from different inputs. Calling that a "match" would be a
    // false parity claim.
    let paper = SignalSet {
        provenance: provenance("1.3.0", "kr-etf-daily-20260804.1", "1.231.0"),
        ..paper_side()
    };
    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(
        report.status,
        ParityStatus::NotComparable,
        "matching numbers never upgrade an incomparable lineage to a match"
    );
    assert!(
        report.divergences.is_empty(),
        "an incomparable report makes no signal claims at all"
    );
}

// ---------------------------------------------------------------------------
// The engine difference is EXPECTED, not a lineage mismatch.
// ---------------------------------------------------------------------------

#[test]
fn a_different_engine_version_does_not_break_comparability() {
    // The backtest runs on NautilusTrader; the Paper runner models the open
    // itself. That difference is the whole point of the fill-model note --
    // treating it as a lineage mismatch would make every real parity report
    // NotComparable and the feature useless.
    let paper = SignalSet {
        provenance: provenance("1.2.0", "kr-etf-daily-20260804.1", "1.240.0"),
        ..paper_side()
    };
    let report = evaluate_parity(&backtest_side(), &paper);
    assert_eq!(
        report.status,
        ParityStatus::Match,
        "engine/engine_version are deliberately outside the parity lineage"
    );
    assert!(!report.fill_model_difference.is_empty());
}

// ---------------------------------------------------------------------------
// Reporting contract
// ---------------------------------------------------------------------------

#[test]
fn every_report_serializes_with_its_status_and_reason() {
    let mut paper = paper_side();
    paper
        .targets
        .insert(instrument("069500.KRX"), weight("0.550000"));
    let report = evaluate_parity(&backtest_side(), &paper);

    let json = serde_json::to_value(&report).expect("a report serializes");
    assert_eq!(json["status"], "DIVERGENT");
    assert!(
        json["fill_model_difference"].as_str().is_some(),
        "the fill-model note is part of the wire contract"
    );
    assert!(
        json["divergences"]
            .as_array()
            .is_some_and(|d| !d.is_empty()),
        "divergences travel with the report"
    );
}

#[test]
fn a_warning_grade_is_reported_for_a_divergence_and_not_for_a_match() {
    // Design §15.3 grades "Paper 불일치" as WARNING (web + admin alert).
    // The report carries the grade so the caller never has to re-derive it.
    let matched = evaluate_parity(&backtest_side(), &paper_side());
    assert!(!matched.warrants_alert(), "a match raises no alert");

    let mut paper = paper_side();
    paper
        .targets
        .insert(instrument("069500.KRX"), weight("0.550000"));
    assert!(
        evaluate_parity(&backtest_side(), &paper).warrants_alert(),
        "a divergence is a WARNING-grade alert"
    );

    let incomparable = SignalSet {
        provenance: provenance("1.3.0", "kr-etf-daily-20260804.1", "1.231.0"),
        ..paper_side()
    };
    assert!(
        evaluate_parity(&backtest_side(), &incomparable).warrants_alert(),
        "an incomparable lineage is also worth alerting on -- it means the \
         Paper account drifted off the strategy it was bound to"
    );
}
