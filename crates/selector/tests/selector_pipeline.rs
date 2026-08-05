//! Todo 16 acceptance target: the fixed-universe selector and explainable
//! constrained target portfolio.
//!
//! `cargo test -p selector --test selector_pipeline` proves the documented
//! pipeline `UniverseBuilder -> EligibilityFilter -> FactorSnapshot ->
//! ScoreComposer -> Ranker -> PortfolioConstraints -> TargetPortfolio`
//! (design §6.6, requirements FR-SEL-003/004/005):
//!
//! - identical inputs -> identical ordering / reasons / weights (FR-SEL-003);
//! - weights finite, nonnegative, sum <= 1 within the declared tolerance, and
//!   the cash floor / per-instrument max-weight constraints hold (FR-SEL-004);
//! - every selected and every excluded instrument carries structured evidence:
//!   a reason code plus localized (ko/en) text (FR-SEL-005);
//! - ties break deterministically by canonical `InstrumentId`;
//! - NULL mandatory factor -> typed exclusion; BLOCKED dataset -> typed
//!   `DATA_BLOCKED` denial; all-ineligible universe -> deterministic all-cash
//!   outcome; impossible constraints -> typed error; weight-rounding residue
//!   -> deterministic cash allocation, never a silent drop.
//!
//! The selector outputs TARGETS ONLY: this suite also asserts the serialized
//! portfolio carries no order vocabulary (no orders are ever created here).

use std::collections::{BTreeMap, BTreeSet};

use domain::{ContentHash, DataState, InstrumentId, TradingDate};
use factor_engine::snapshot::NormalizationMeta;
use factor_engine::{FactorRow, FactorSnapshot};
use market_data::{IssueCode, QualityIssue, QualityReport, Severity};
use selector::eligibility::Exclusion;
use selector::reason::ReasonCode;
use selector::spec::SelectionSpec;
use selector::target::TargetPortfolio;
use selector::{SelectorError, select_targets};

// ---------------------------------------------------------------------------
// Fixture helpers (synthetic, never production data)
// ---------------------------------------------------------------------------

const V1_SYMBOLS: [&str; 11] = [
    "069500", "102110", "229200", "143850", "133690", "195930", "192090", "148070", "114260",
    "153130", "132030",
];

/// One fixture instrument: (symbol, raw12, norm12, rawvol, normvol).
type FactorFixture = (
    &'static str,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

fn td(y: i32, m: u32, d: u32) -> TradingDate {
    TradingDate::new(y, m, d).expect("valid date")
}

fn weights(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn mandatory(pairs: &[&str]) -> BTreeSet<String> {
    pairs.iter().map(|s| s.to_string()).collect()
}

fn id(symbol: &str) -> InstrumentId {
    InstrumentId::parse(&format!("{symbol}.KRX")).expect("valid id")
}

fn universe_hash() -> String {
    ContentHash::from_bytes(b"universe-fixture")
        .as_str()
        .to_owned()
}

/// A published-style universe snapshot over the given symbols (all fields
/// public in `selector::publish::PublishedSnapshot`).
fn fixture_universe(symbols: &[&str]) -> selector::publish::PublishedSnapshot {
    use domain::{AssetClass, Currency};
    use selector::publish::PublishedSnapshot;
    use selector::universe::{Eligibility, SourceSnapshot};

    PublishedSnapshot {
        universe_id: "kr-etf-core-v1".to_owned(),
        base_currency: Currency::KRW,
        effective_from: td(2020, 1, 31),
        effective_until: None,
        benchmark: id("069500"),
        eligibility: Eligibility {
            unleveraged: true,
            non_inverse: true,
            asset_class: AssetClass::Etf,
        },
        instruments: symbols.iter().map(|s| id(s)).collect(),
        source_snapshot: SourceSnapshot {
            source: "krx-reference-2019-v1".to_owned(),
            version: "1.0".to_owned(),
            captured_at: "2019-12-31".to_owned(),
        },
        universe_snapshot_id: ContentHash::from_bytes(b"universe-fixture"),
    }
}

/// A factor snapshot with the two MVP factors `return_12m` and `vol_20d` over
/// the given per-instrument (raw12, norm12, rawvol, normvol) values on as_of.
fn fixture_factors(as_of: TradingDate, values: &[FactorFixture]) -> FactorSnapshot {
    let mut rows = Vec::new();
    for (symbol, raw12, norm12, rawvol, normvol) in values {
        rows.push(FactorRow {
            date: as_of.to_iso(),
            instrument: id(symbol).as_str(),
            factor: "return_12m".to_owned(),
            raw: *raw12,
            normalized: *norm12,
        });
        rows.push(FactorRow {
            date: as_of.to_iso(),
            instrument: id(symbol).as_str(),
            factor: "vol_20d".to_owned(),
            raw: *rawvol,
            normalized: *normvol,
        });
    }
    FactorSnapshot {
        as_of,
        universe_snapshot_id: universe_hash(),
        dataset_id: "kr-etf-daily".to_owned(),
        dataset_version: 1,
        factor_versions: BTreeMap::from([
            ("return_12m".to_owned(), "1.0.0".to_owned()),
            ("vol_20d".to_owned(), "1.0.0".to_owned()),
        ]),
        normalization: NormalizationMeta {
            id: "z_score".to_owned(),
            version: "1.0.0".to_owned(),
            params: BTreeMap::from([("cap".to_owned(), "3.0".to_owned())]),
        },
        rows,
        hash: ContentHash::from_bytes(b"factors-fixture"),
    }
}

fn ready_report() -> QualityReport {
    QualityReport {
        dataset_id: domain::DatasetId::parse("kr-etf-daily").expect("valid id"),
        version: 1,
        state: DataState::Ready,
        issues: vec![],
        exclusions: vec![],
        content_hash: ContentHash::from_bytes(b"ready-report"),
    }
}

fn blocked_report() -> QualityReport {
    QualityReport {
        dataset_id: domain::DatasetId::parse("kr-etf-daily").expect("valid id"),
        version: 1,
        state: DataState::Blocked,
        issues: vec![QualityIssue {
            code: IssueCode::MissingRequiredBar,
            severity: Severity::Blocking,
            instrument: Some(id("069500")),
            date: Some(td(2020, 2, 4)),
            detail: "missing required bar on 2020-02-04".to_owned(),
        }],
        exclusions: vec![],
        content_hash: ContentHash::from_bytes(b"blocked-report"),
    }
}

/// The default MVP spec: momentum 0.7 / volatility 0.3, mandatory 12m
/// momentum, top 7 of 11, cash floor 20%, and a max weight that exactly fits
/// the investable budget (0.8/7) so the default run is never capacity-bound.
fn default_spec() -> SelectionSpec {
    SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
        mandatory(&["return_12m"]),
        7,
        0.8 / 7.0,
        0.2,
        4,
        1e-9,
    )
    .expect("default spec validates")
}

/// The 11-symbol fixture: distinct, fully-determined scores.
fn full_fixture() -> (FactorSnapshot, Vec<FactorFixture>) {
    let values: Vec<FactorFixture> = vec![
        ("069500", Some(0.182), Some(1.5), Some(0.121), Some(-0.4)),
        ("102110", Some(0.121), Some(1.2), Some(0.095), Some(-0.1)),
        ("229200", Some(0.095), Some(0.9), Some(0.141), Some(0.2)),
        ("143850", Some(0.071), Some(0.6), Some(0.110), Some(0.1)),
        ("133690", Some(0.048), Some(0.3), Some(0.132), Some(0.3)),
        ("195930", Some(0.021), Some(0.0), Some(0.104), Some(0.0)),
        ("192090", Some(-0.012), Some(-0.3), Some(0.160), Some(0.5)),
        ("148070", Some(-0.041), Some(-0.6), Some(0.120), Some(-0.2)),
        ("114260", Some(-0.072), Some(-0.9), Some(0.135), Some(0.4)),
        ("153130", Some(-0.101), Some(-1.2), Some(0.098), Some(-0.3)),
        ("132030", Some(-0.135), Some(-1.5), Some(0.115), Some(-0.5)),
    ];
    let factors = fixture_factors(td(2020, 1, 31), &values);
    (factors, values)
}

fn select(
    spec: &SelectionSpec,
    universe: &selector::publish::PublishedSnapshot,
    factors: &FactorSnapshot,
) -> Result<TargetPortfolio, SelectorError> {
    select_targets(spec, &ready_report(), universe, factors)
}

// ---------------------------------------------------------------------------
// (a) Determinism: identical inputs -> identical ordering / reasons / weights
// ---------------------------------------------------------------------------

#[test]
fn identical_inputs_produce_byte_identical_portfolio() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = default_spec();

    let a = select(&spec, &universe, &factors).expect("first selection succeeds");
    let b = select(&spec, &universe, &factors).expect("second selection succeeds");

    // Struct-level identity.
    assert_eq!(a, b, "identical inputs must yield identical portfolios");
    // Byte-level identity of the canonical JSON.
    assert_eq!(
        serde_json::to_vec(&a).expect("serializes A"),
        serde_json::to_vec(&b).expect("serializes B"),
        "serialized portfolios must be byte-identical"
    );
    // Snapshot ids are deterministic too.
    assert_eq!(
        a.portfolio_snapshot_id, b.portfolio_snapshot_id,
        "portfolio snapshot ids must match"
    );
    assert!(a.portfolio_snapshot_id.starts_with("sha256:"));
}

#[test]
fn ranking_is_monotone_contiguous_and_deterministic() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");

    let ranks: Vec<usize> = portfolio.targets.iter().map(|t| t.rank).collect();
    let expected: Vec<usize> = (1..=V1_SYMBOLS.len()).collect();
    assert_eq!(
        ranks, expected,
        "all eligible instruments carry contiguous ranks 1..=N"
    );
    // Scores are non-increasing down the rank order.
    let scores: Vec<f64> = portfolio.targets.iter().map(|t| t.score).collect();
    for pair in scores.windows(2) {
        assert!(pair[0] >= pair[1], "scores must be non-increasing by rank");
    }
}

// ---------------------------------------------------------------------------
// (b)+(c) Weight invariants: finite, nonnegative, sum <= 1 within tolerance,
// cash floor and per-instrument max weight hold
// ---------------------------------------------------------------------------

#[test]
fn weights_finite_nonnegative_and_sum_le_one_within_tolerance() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);

    for top_n in [1usize, 3, 7, 11] {
        for (max_weight, cash_floor) in [(1.0, 0.2), (0.15, 0.0), (0.1, 0.3)] {
            let spec = SelectionSpec::new(
                "relative_momentum",
                "1.0.0",
                weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
                mandatory(&["return_12m"]),
                top_n,
                max_weight,
                cash_floor,
                4,
                1e-9,
            )
            .expect("spec validates");
            // Skip impossible combos: max_weight * selected <= investable.
            let investable = 1.0 - cash_floor;
            let selected = top_n.min(V1_SYMBOLS.len());
            if max_weight * selected as f64 > investable + spec.tolerance {
                continue;
            }

            let portfolio = select(&spec, &universe, &factors).expect("selection succeeds");
            let mut sum: f64 = 0.0;
            for t in &portfolio.targets {
                assert!(t.target_weight.is_finite(), "weight must be finite");
                assert!(
                    t.target_weight >= 0.0,
                    "weight must be nonnegative ({} was {})",
                    t.instrument_id,
                    t.target_weight
                );
                sum += t.target_weight;
            }
            assert!(
                sum <= 1.0 + spec.tolerance,
                "target weight sum {sum} must be <= 1 within tolerance {}",
                spec.tolerance
            );
            assert!(
                portfolio.cash_weight.is_finite() && portfolio.cash_weight >= 0.0,
                "cash weight must be finite and nonnegative"
            );
            // Sum of targets plus cash is exactly the full budget.
            assert!(
                (sum + portfolio.cash_weight - 1.0).abs() <= spec.tolerance,
                "targets ({sum}) + cash ({}) must equal 1 within tolerance",
                portfolio.cash_weight
            );
        }
    }
}

#[test]
fn cash_floor_and_max_weight_constraints_hold() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);

    // Capped case: top 3, max 0.2, floor 0.2 -> every target capped at 0.2.
    let spec = SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
        mandatory(&["return_12m"]),
        3,
        0.2,
        0.2,
        4,
        1e-9,
    )
    .expect("spec validates");
    let portfolio = select(&spec, &universe, &factors).expect("selection succeeds");
    for t in &portfolio.targets {
        assert!(
            t.target_weight <= spec.max_weight + spec.tolerance,
            "target {} weight {} must not exceed max {}",
            t.instrument_id,
            t.target_weight,
            spec.max_weight
        );
        assert!(
            portfolio.cash_weight >= spec.cash_floor - spec.tolerance,
            "cash {} must not fall below floor {}",
            portfolio.cash_weight,
            spec.cash_floor
        );
    }
    // Capped exactness: 3 targets x 0.2 = 0.6; the 0.2 residue lands in cash.
    assert_eq!(portfolio.targets[0].target_weight, 0.2);
    assert_eq!(portfolio.cash_weight, 0.4);
    // The cap is explained on every SELECTED (capped) target.
    for t in portfolio.targets.iter().filter(|t| t.rank <= 3) {
        assert!(
            t.reasons
                .iter()
                .any(|r| r.code == ReasonCode::WeightCappedAtMax),
            "capped target {} must carry a WEIGHT_CAPPED_AT_MAX reason",
            t.instrument_id
        );
    }
}

// ---------------------------------------------------------------------------
// (d) Structured evidence: reason code + localized text on every item
// ---------------------------------------------------------------------------

#[test]
fn every_selected_and_excluded_item_has_structured_evidence() {
    let universe = fixture_universe(&V1_SYMBOLS);
    // Exclude 229200.KRX via a NULL mandatory factor while everything else is
    // healthy.
    let mut values = full_fixture().1;
    values[2].1 = None;
    values[2].2 = None;
    let factors = fixture_factors(td(2020, 1, 31), &values);
    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");

    for t in &portfolio.targets {
        assert!(
            !t.reasons.is_empty(),
            "{} must carry reasons",
            t.instrument_id
        );
        for r in &t.reasons {
            assert!(!r.text_ko.is_empty(), "reason {r:?} must have ko text");
            assert!(!r.text_en.is_empty(), "reason {r:?} must have en text");
        }
    }
    assert_eq!(
        portfolio.exclusions.len(),
        1,
        "exactly one exclusion expected"
    );
    let ex = &portfolio.exclusions[0];
    assert_eq!(ex.instrument, id("229200"));
    assert_eq!(ex.reason.code, ReasonCode::ExcludedMandatoryFactorNull);
    assert!(!ex.reason.text_ko.is_empty() && !ex.reason.text_en.is_empty());
    assert_eq!(ex.missing_factors, vec!["return_12m".to_owned()]);
    // Excluded instruments never appear among targets.
    assert!(
        portfolio
            .targets
            .iter()
            .all(|t| t.instrument_id != ex.instrument),
        "excluded instrument must not be a target"
    );
    // Unselected instruments still carry an explanation.
    for t in portfolio.targets.iter().filter(|t| t.rank > 7) {
        assert!(
            t.reasons
                .iter()
                .any(|r| r.code == ReasonCode::NotSelectedBeyondTopN),
            "rank > top_n must be explained ({} rank {})",
            t.instrument_id,
            t.rank
        );
        assert_eq!(t.target_weight, 0.0);
    }
}

// ---------------------------------------------------------------------------
// (e) Tie scores break deterministically by canonical InstrumentId
// ---------------------------------------------------------------------------

#[test]
fn tie_scores_break_by_canonical_instrument_id() {
    // Three instruments with IDENTICAL factor values -> identical scores.
    let symbols = ["069500", "102110", "229200"];
    let values: Vec<FactorFixture> = vec![
        ("069500", Some(0.10), Some(0.5), Some(0.05), Some(0.5)),
        ("102110", Some(0.10), Some(0.5), Some(0.05), Some(0.5)),
        ("229200", Some(0.10), Some(0.5), Some(0.05), Some(0.5)),
    ];
    let factors = fixture_factors(td(2020, 1, 31), &values);
    let universe = fixture_universe(&symbols);
    let spec = SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
        mandatory(&["return_12m"]),
        3,
        1.0 / 3.0,
        0.0,
        4,
        1e-9,
    )
    .expect("spec validates");

    let portfolio = select(&spec, &universe, &factors).expect("selection succeeds");
    let ordered: Vec<String> = portfolio
        .targets
        .iter()
        .map(|t| t.instrument_id.as_str())
        .collect();
    // Canonical id order breaks the tie: 069500 < 102110 < 229200.
    assert_eq!(ordered, vec!["069500.KRX", "102110.KRX", "229200.KRX"]);
    let scores: Vec<f64> = portfolio.targets.iter().map(|t| t.score).collect();
    assert!(
        scores.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-12),
        "all tied scores must be equal"
    );
    // Rerun: identical tie-break.
    let again = select(&spec, &universe, &factors).expect("rerun succeeds");
    assert_eq!(
        again.targets[0].instrument_id,
        portfolio.targets[0].instrument_id
    );
}

// ---------------------------------------------------------------------------
// (f) NULL mandatory factor -> typed exclusion (also covers the all-ineligible
// universe class in its full form below)
// ---------------------------------------------------------------------------

#[test]
fn null_mandatory_factor_excludes_with_reason() {
    let universe = fixture_universe(&V1_SYMBOLS);
    // Make the mandatory factor NULL for 102110.KRX only.
    let mut values = full_fixture().1;
    values[1].1 = None;
    values[1].2 = None;
    let factors = fixture_factors(td(2020, 1, 31), &values);

    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");
    assert!(
        portfolio
            .targets
            .iter()
            .all(|t| t.instrument_id != id("102110")),
        "NULL mandatory factor must exclude 102110.KRX"
    );
    let ex = portfolio
        .exclusions
        .iter()
        .find(|e| e.instrument == id("102110"))
        .expect("exclusion record exists");
    assert_eq!(ex.reason.code, ReasonCode::ExcludedMandatoryFactorNull);
    assert_eq!(ex.missing_factors, vec!["return_12m".to_owned()]);
}

// ---------------------------------------------------------------------------
// (g) BLOCKED dataset -> typed DATA_BLOCKED denial, no targets emitted
// ---------------------------------------------------------------------------

#[test]
fn blocked_dataset_yields_typed_denial_with_no_output() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = default_spec();

    let err = select_targets(&spec, &blocked_report(), &universe, &factors)
        .expect_err("BLOCKED dataset must deny selection");
    assert_eq!(err.code(), "DATA_BLOCKED", "typed dataset-state denial");
    match &err {
        SelectorError::DataBlocked {
            dataset_id,
            state,
            blocking_issues,
        } => {
            assert_eq!(dataset_id, "kr-etf-daily");
            assert_eq!(state, "blocked");
            assert!(
                blocking_issues.contains("MISSING_REQUIRED_BAR"),
                "{blocking_issues}"
            );
        }
        other => panic!("expected DataBlocked, got {other:?}"),
    }
    // The API is all-or-nothing: an Err carries no portfolio.
    // (compile-time boundary: select_targets returns Result<TargetPortfolio, _>)
}

// ---------------------------------------------------------------------------
// (h) All-ineligible universe -> deterministic all-cash outcome
// ---------------------------------------------------------------------------

#[test]
fn all_ineligible_universe_yields_deterministic_all_cash() {
    let all_null: Vec<FactorFixture> = V1_SYMBOLS
        .iter()
        .map(|s| (*s, None, None, Some(0.1), Some(0.0)))
        .collect();
    let factors = fixture_factors(td(2020, 1, 31), &all_null);
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = default_spec();

    let a = select(&spec, &universe, &factors).expect("all-cash outcome succeeds");
    let b = select(&spec, &universe, &factors).expect("rerun succeeds");
    assert_eq!(a, b, "all-ineligible outcome must be deterministic");
    assert!(
        a.targets.iter().all(|t| t.target_weight == 0.0),
        "no eligible instrument may receive weight"
    );
    assert_eq!(a.cash_weight, 1.0, "portfolio is fully cash");
    assert!(
        a.portfolio_reasons
            .iter()
            .any(|r| r.code == ReasonCode::AllCashNoEligible),
        "all-cash outcome must be explained"
    );
    assert_eq!(
        a.exclusions.len(),
        V1_SYMBOLS.len(),
        "every member is excluded"
    );
    for ex in &a.exclusions {
        assert_eq!(ex.reason.code, ReasonCode::ExcludedMandatoryFactorNull);
    }
}

// ---------------------------------------------------------------------------
// (i) Impossible constraints -> typed error, no partial output
// ---------------------------------------------------------------------------

#[test]
fn impossible_constraints_yield_typed_error() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);

    // cash_floor 0.2 + 5 targets at max 0.2 needs 1.0 weight > 0.8 available.
    let impossible = SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
        mandatory(&["return_12m"]),
        5,
        0.2,
        0.2,
        4,
        1e-9,
    )
    .expect("spec validates");
    let err =
        select(&impossible, &universe, &factors).expect_err("impossible constraints must error");
    assert_eq!(err.code(), "CONSTRAINTS_IMPOSSIBLE");
    match &err {
        SelectorError::ImpossibleConstraints { detail } => {
            assert!(
                detail.contains("0.2"),
                "detail must name the max weight: {detail}"
            );
        }
        other => panic!("expected ImpossibleConstraints, got {other:?}"),
    }

    // Boundary: 8 targets at max 0.1 with floor 0.2 uses exactly 0.8 -> OK.
    let boundary = SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("return_12m", 0.7), ("vol_20d", 0.3)]),
        mandatory(&["return_12m"]),
        8,
        0.1,
        0.2,
        4,
        1e-9,
    )
    .expect("spec validates");
    let portfolio = select(&boundary, &universe, &factors).expect("boundary fits exactly");
    let sum: f64 = portfolio.targets.iter().map(|t| t.target_weight).sum();
    assert!(
        (sum - 0.8).abs() <= 1e-9,
        "boundary invests exactly 0.8, got {sum}"
    );
}

// ---------------------------------------------------------------------------
// (j) Weight-rounding residue -> deterministic cash allocation
// ---------------------------------------------------------------------------

#[test]
fn weight_rounding_residue_goes_to_cash_deterministically() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    // 7 targets over 0.8 investable: 0.8/7 = 0.1142857... truncates to 0.1142
    // per target (7 x 0.1142 = 0.7994); the 0.0006 residue lands in cash.
    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");

    let selected: Vec<&selector::target::TargetRow> =
        portfolio.targets.iter().filter(|t| t.rank <= 7).collect();
    assert_eq!(selected.len(), 7);
    for t in &selected {
        assert_eq!(
            t.target_weight, 0.1142,
            "{} must carry the truncated weight 0.1142",
            t.instrument_id
        );
    }
    let sum: f64 = selected.iter().map(|t| t.target_weight).sum();
    assert!((sum - 0.7994).abs() < 1e-12, "sum was {sum}");
    assert_eq!(
        portfolio.cash_weight, 0.2006,
        "residue + floor lands in cash"
    );
    assert!(
        (sum + portfolio.cash_weight - 1.0).abs() < 1e-12,
        "targets + cash must still equal 1"
    );
    assert!(
        portfolio
            .portfolio_reasons
            .iter()
            .any(|r| r.code == ReasonCode::WeightRoundingResidueToCash),
        "rounding residue must be explained, never silently dropped"
    );
    // Determinism of the residue allocation.
    let again = select(&default_spec(), &universe, &factors).expect("rerun succeeds");
    assert_eq!(
        again
            .targets
            .iter()
            .map(|t| t.target_weight)
            .collect::<Vec<_>>(),
        portfolio
            .targets
            .iter()
            .map(|t| t.target_weight)
            .collect::<Vec<_>>()
    );
    assert_eq!(again.cash_weight, portfolio.cash_weight);
}

// ---------------------------------------------------------------------------
// Stale state: snapshot / provenance ids must line up
// ---------------------------------------------------------------------------

#[test]
fn factor_snapshot_universe_mismatch_is_typed_error() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = default_spec();

    let mut mismatched = factors.clone();
    mismatched.universe_snapshot_id = ContentHash::from_bytes(b"other-universe")
        .as_str()
        .to_owned();
    let err = select(&spec, &universe, &mismatched).expect_err("mismatch must error");
    assert_eq!(err.code(), "UNIVERSE_MISMATCH");
    match &err {
        SelectorError::UniverseMismatch { .. } => {}
        other => panic!("expected UniverseMismatch, got {other:?}"),
    }
}

#[test]
fn as_of_outside_universe_window_is_typed_error() {
    let (_, values) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = default_spec();

    // Late as-of inside the open-ended window, with rows dated consistently.
    let late = fixture_factors(td(2021, 6, 30), &values);
    assert!(select(&spec, &universe, &late).is_ok());

    let early = fixture_factors(td(2019, 12, 31), &values);
    let err = select(&spec, &universe, &early).expect_err("window violation must error");
    assert_eq!(err.code(), "AS_OF_OUTSIDE_WINDOW");
}

// ---------------------------------------------------------------------------
// Malformed input: never a panic, always a typed error
// ---------------------------------------------------------------------------

#[test]
fn unknown_factor_in_spec_is_typed_error() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let spec = SelectionSpec::new(
        "relative_momentum",
        "1.0.0",
        weights(&[("bogus_factor", 1.0)]),
        mandatory(&["return_12m"]),
        3,
        1.0,
        0.0,
        4,
        1e-9,
    )
    .expect("spec validates");
    let err = select(&spec, &universe, &factors).expect_err("unknown factor must error");
    assert_eq!(err.code(), "UNKNOWN_FACTOR");
}

#[test]
fn universe_member_missing_snapshot_row_is_typed_error() {
    // 10 of 11 instruments have rows; 132030.KRX has none on as_of.
    let values: Vec<FactorFixture> = full_fixture()
        .1
        .into_iter()
        .filter(|(s, ..)| *s != "132030")
        .collect();
    let factors = fixture_factors(td(2020, 1, 31), &values);
    let universe = fixture_universe(&V1_SYMBOLS);
    let err = select(&default_spec(), &universe, &factors).expect_err("missing row must error");
    assert_eq!(err.code(), "MISSING_FACTOR_ROW");
    match &err {
        SelectorError::MissingFactorRow { instrument, .. } => {
            assert_eq!(instrument, "132030.KRX");
        }
        other => panic!("expected MissingFactorRow, got {other:?}"),
    }
}

#[test]
fn snapshot_unknown_instrument_is_typed_error() {
    let mut values: Vec<FactorFixture> = full_fixture().1;
    values.push(("999999", Some(0.1), Some(0.5), Some(0.05), Some(0.5)));
    let mut symbols = V1_SYMBOLS.to_vec();
    symbols.push("999999"); // in the snapshot but NOT in the published universe
    let factors = fixture_factors(td(2020, 1, 31), &values);
    let universe = fixture_universe(&V1_SYMBOLS);
    let err =
        select(&default_spec(), &universe, &factors).expect_err("unknown instrument must error");
    assert_eq!(err.code(), "UNKNOWN_SNAPSHOT_INSTRUMENT");
}

#[test]
fn invalid_spec_is_typed_error_never_panic() {
    let cases: Vec<(&str, SelectorError)> = vec![
        (
            "cash floor at 1.0 leaves no room",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", 1.0)]),
                BTreeSet::new(),
                1,
                1.0,
                1.0,
                4,
                1e-9,
            )
            .expect_err("floor 1.0 is invalid"),
        ),
        (
            "max weight above 1",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", 1.0)]),
                BTreeSet::new(),
                1,
                1.5,
                0.0,
                4,
                1e-9,
            )
            .expect_err("max weight 1.5 is invalid"),
        ),
        (
            "empty factor weights",
            SelectionSpec::new(
                "s",
                "1.0.0",
                BTreeMap::new(),
                BTreeSet::new(),
                1,
                1.0,
                0.0,
                4,
                1e-9,
            )
            .expect_err("empty weights are invalid"),
        ),
        (
            "top_n zero",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", 1.0)]),
                BTreeSet::new(),
                0,
                1.0,
                0.0,
                4,
                1e-9,
            )
            .expect_err("top_n 0 is invalid"),
        ),
        (
            "weight scale beyond 6",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", 1.0)]),
                BTreeSet::new(),
                1,
                1.0,
                0.0,
                9,
                1e-9,
            )
            .expect_err("scale 9 is invalid"),
        ),
        (
            "NaN factor weight",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", f64::NAN)]),
                BTreeSet::new(),
                1,
                1.0,
                0.0,
                4,
                1e-9,
            )
            .expect_err("NaN weight is invalid"),
        ),
        (
            "negative cash floor",
            SelectionSpec::new(
                "s",
                "1.0.0",
                weights(&[("return_12m", 1.0)]),
                BTreeSet::new(),
                1,
                1.0,
                -0.1,
                4,
                1e-9,
            )
            .expect_err("negative floor is invalid"),
        ),
    ];
    for (label, err) in cases {
        assert_eq!(err.code(), "SPEC_INVALID", "{label}");
        match err {
            SelectorError::InvalidSpec { .. } => {}
            other => panic!("{label}: expected InvalidSpec, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Typed boundary: the selector emits targets only, never orders
// ---------------------------------------------------------------------------

#[test]
fn portfolio_output_contains_no_order_vocabulary() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");
    let json = serde_json::to_string(&portfolio).expect("serializes");
    for forbidden in [
        "\"order\"",
        "order_id",
        "\"side\"",
        "\"quantity\"",
        "\"qty\"",
        "\"price\"",
    ] {
        assert!(
            !json.contains(forbidden),
            "target portfolio must never carry order vocabulary, found {forbidden} in {json}"
        );
    }
}

// ---------------------------------------------------------------------------
// Provenance: snapshot / provenance ids carried through
// ---------------------------------------------------------------------------

#[test]
fn snapshot_and_provenance_ids_are_carried_through() {
    let (factors, _) = full_fixture();
    let universe = fixture_universe(&V1_SYMBOLS);
    let portfolio = select(&default_spec(), &universe, &factors).expect("selection succeeds");

    assert_eq!(portfolio.universe_snapshot_id, universe_hash());
    assert_eq!(portfolio.factor_snapshot_hash, factors.hash.as_str());
    assert_eq!(portfolio.dataset_id, "kr-etf-daily");
    assert_eq!(portfolio.dataset_version, 1);
    assert_eq!(portfolio.as_of, td(2020, 1, 31));
    assert_eq!(portfolio.strategy_version, "relative_momentum@1.0.0");
    assert_eq!(portfolio.constraints.top_n, 7);
    assert!((portfolio.constraints.max_weight - 0.8 / 7.0).abs() < 1e-12);
    assert_eq!(portfolio.constraints.cash_floor, 0.2);
    // Factor raw/normalized values ride along for explainability (FR-SEL-005).
    let top = &portfolio.targets[0];
    assert_eq!(top.instrument_id, id("069500"));
    assert_eq!(top.factors["return_12m"].raw, Some(0.182));
    assert_eq!(top.factors["return_12m"].normalized, Some(1.5));
    assert_eq!(top.factors["vol_20d"].raw, Some(0.121));
    assert_eq!(top.factors["vol_20d"].normalized, Some(-0.4));
}

/// `Exclusion` and `ReasonCode` are part of the public API surface used above;
/// this helper only exists to silence unused-import warnings when a future
/// refactor stops using one of them.
#[allow(dead_code)]
fn _api_surface(_: &Exclusion, _: ReasonCode) {}
