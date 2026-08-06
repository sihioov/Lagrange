//! Todo 21 RED tests: reference stability score (design §9.6).
//!
//! The score is composed of the seven documented weighted components
//! (25/20/15/15/10/10/5), always carries its raw evidence, and — the
//! critical guard — is NEVER an investment-approval signal
//! ("초기 점수는 참고 지표이며 절대적 승인 기준으로 사용하지 않는다"):
//! [`approve_investment`] always refuses with
//! [`RobustnessError::StabilityScoreNotApproval`].

mod common;

use domain::ReportedStat;

use result_model::robustness::{
    RobustnessError, StabilityEvidence, StabilityScore, analyze_stability, approve_investment,
};

fn stat(value: f64) -> ReportedStat {
    ReportedStat::from_f64(value).unwrap()
}

fn strong_evidence() -> StabilityEvidence {
    StabilityEvidence {
        validation_monthly_excess: vec![stat(0.02), stat(0.01), stat(0.015), stat(0.005)],
        neighborhood_returns: vec![stat(0.10), stat(0.11), stat(0.105), stat(0.10)],
        parent_return: stat(0.10),
        cost_stress_final_returns: vec![stat(0.08), stat(0.06), stat(0.04)],
        max_drawdown: stat(-0.08),
        volatility: stat(0.12),
        top_trade_share: stat(0.20),
        year_max_share: stat(0.25),
        recent_excess: stat(0.03),
        turnover: stat(1.2),
    }
}

fn weak_evidence() -> StabilityEvidence {
    StabilityEvidence {
        validation_monthly_excess: vec![stat(-0.02), stat(0.01), stat(-0.03), stat(-0.005)],
        neighborhood_returns: vec![stat(0.20), stat(-0.10), stat(0.05), stat(-0.25)],
        parent_return: stat(0.05),
        cost_stress_final_returns: vec![stat(-0.08), stat(-0.12), stat(-0.06)],
        max_drawdown: stat(-0.45),
        volatility: stat(0.55),
        top_trade_share: stat(0.90),
        year_max_share: stat(0.85),
        recent_excess: stat(-0.04),
        turnover: stat(6.0),
    }
}

#[test]
fn robustness_components_sum_to_total_within_their_weights() {
    let score = analyze_stability(&strong_evidence()).expect("analysis succeeds");
    let mut sum = 0.0f64;
    for component in &score.components {
        assert!(
            component.score >= 0.0 && component.score <= f64::from(component.weight),
            "{}: score {} outside [0, {}]",
            component.code,
            component.score,
            component.weight
        );
        sum += component.score;
    }
    assert!(
        (sum - score.total).abs() < 1e-9,
        "total must equal the sum of components"
    );
    assert!(score.total <= 100.0);
}

#[test]
fn robustness_stability_score_is_never_an_investment_approval() {
    let score = analyze_stability(&strong_evidence()).expect("analysis succeeds");
    let error = approve_investment(&score)
        .expect_err("the stability score must NEVER approve an investment (design 9.6)");
    assert!(matches!(error, RobustnessError::StabilityScoreNotApproval));
    // Even a perfect score cannot approve: the guard is structural.
    assert!(score.reference_only);
    let mut perfect = strong_evidence();
    perfect.validation_monthly_excess = vec![stat(0.05); 12];
    perfect.neighborhood_returns = vec![stat(0.10); 4];
    perfect.max_drawdown = stat(-0.02);
    perfect.volatility = stat(0.05);
    perfect.recent_excess = stat(0.05);
    let perfect_score = analyze_stability(&perfect).unwrap();
    assert!(perfect_score.total > score.total);
    assert!(matches!(
        approve_investment(&perfect_score),
        Err(RobustnessError::StabilityScoreNotApproval)
    ));
}

#[test]
fn robustness_stability_score_carries_raw_evidence() {
    let evidence = strong_evidence();
    let score = analyze_stability(&evidence).unwrap();
    let raw = &score.raw_evidence;
    // Every raw input is echoed in the evidence payload.
    assert_eq!(
        raw["validation_monthly_excess"].as_array().unwrap().len(),
        4
    );
    assert_eq!(raw["neighborhood_returns"].as_array().unwrap().len(), 4);
    assert_eq!(
        raw["cost_stress_final_returns"].as_array().unwrap().len(),
        3
    );
    assert!(raw["max_drawdown"].as_f64().is_some());
    assert!(raw["volatility"].as_f64().is_some());
    assert!(raw["top_trade_share"].as_f64().is_some());
    assert!(raw["year_max_share"].as_f64().is_some());
    assert!(raw["recent_excess"].as_f64().is_some());
    assert!(raw["turnover"].as_f64().is_some());
    // Each component echoes its own raw inputs.
    for component in &score.components {
        assert!(
            component.raw_evidence.is_object(),
            "component {} must carry raw evidence",
            component.code
        );
    }
    // The documented seven components exist.
    let codes: Vec<&str> = score.components.iter().map(|c| c.code.as_str()).collect();
    assert_eq!(
        codes,
        vec![
            "validation_excess_persistence",
            "parameter_neighborhood_stability",
            "cost_stress_survival",
            "mdd_volatility",
            "return_concentration",
            "recent_performance",
            "tradability_turnover",
        ]
    );
}

#[test]
fn robustness_strong_evidence_scores_materially_higher_than_weak() {
    let strong = analyze_stability(&strong_evidence()).unwrap();
    let weak = analyze_stability(&weak_evidence()).unwrap();
    assert!(
        strong.total > 70.0,
        "strong evidence should score high (got {})",
        strong.total
    );
    assert!(
        weak.total < 45.0,
        "weak evidence should score low (got {})",
        weak.total
    );
    assert!(strong.total > weak.total);
}

#[test]
fn robustness_stability_score_is_deterministic() {
    let a = analyze_stability(&strong_evidence()).unwrap();
    let b = analyze_stability(&strong_evidence()).unwrap();
    assert_eq!(a.total, b.total);
    assert_eq!(a.components, b.components);
}

#[test]
fn robustness_neighborhood_dispersion_moves_the_component_score() {
    let mut tight = strong_evidence();
    tight.neighborhood_returns = vec![stat(0.10), stat(0.101), stat(0.1005), stat(0.10)];
    let mut scattered = strong_evidence();
    scattered.neighborhood_returns = vec![stat(0.10), stat(0.20), stat(0.0), stat(-0.05)];
    let tight_score = analyze_stability(&tight).unwrap();
    let scattered_score = analyze_stability(&scattered).unwrap();
    let component = |score: &StabilityScore, code: &str| {
        score
            .components
            .iter()
            .find(|c| c.code == code)
            .expect("component exists")
            .score
    };
    assert!(
        component(&tight_score, "parameter_neighborhood_stability")
            > component(&scattered_score, "parameter_neighborhood_stability"),
        "tighter neighborhoods must score higher on stability"
    );
}
