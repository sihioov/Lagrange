//! Todo 29: bounded robustness-suite planning and promotion-evidence
//! assembly (plan acceptance: "bounded fan-out, one-axis child configs,
//! cancellation, holdout non-access, version pinning, and promotion refusal
//! without all required evidence").
//!
//! Cancellation is job-queue's contract (`job_queue::batch::cancel_batch`,
//! tested in that crate); this file proves the result-model side: a suite
//! never registers more than the bounded grid, every child pins exactly one
//! axis and the parent's strategy/data/engine version, holdout dates are
//! never reachable through a planned child, and promotion evidence can only
//! be assembled from a suite where every child actually succeeded.

mod common;

use std::collections::BTreeSet;

use domain::ReportedStat;
use uuid::Uuid;

use result_model::robustness::{
    DerivedAxis, EvidenceManifests, HoldoutBarrier, LineageRegistry, MAX_SUITE_CHILDREN,
    PeriodSplit, PlannedChild, RobustnessError, StabilityEvidence, SuiteChildOutcome, SuiteRequest,
    analyze_stability, assemble_evidence_bundle, plan_suite,
};

fn stat(v: f64) -> ReportedStat {
    ReportedStat::from_f64(v).unwrap()
}

fn cost_stress_child(profile_id: &str) -> PlannedChild {
    PlannedChild {
        axes: vec![DerivedAxis::CostStress {
            profile_id: profile_id.to_owned(),
            profile_version: 1,
        }],
        provenance: common::derived_provenance(),
    }
}

fn split() -> PeriodSplit {
    PeriodSplit {
        train_end: "2024-06-30".to_owned(),
        validation_end: "2024-12-31".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Bounded fan-out (parameter-grid limits)
// ---------------------------------------------------------------------------

#[test]
fn robustness_orchestration_rejects_oversized_grid_before_registering_anything() {
    let mut registry = LineageRegistry::new();
    let children: Vec<PlannedChild> = (0..=MAX_SUITE_CHILDREN)
        .map(|i| cost_stress_child(&format!("profile-{i}")))
        .collect();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children,
    };

    let err =
        plan_suite(&mut registry, None, request).expect_err("oversized grid must be rejected");
    assert!(matches!(
        err,
        RobustnessError::GridTooLarge { requested, max }
            if requested == MAX_SUITE_CHILDREN + 1 && max == MAX_SUITE_CHILDREN
    ));
    assert!(
        registry.is_empty(),
        "a rejected suite must register NOTHING, not a truncated prefix"
    );
}

#[test]
fn robustness_orchestration_accepts_a_grid_at_exactly_the_limit() {
    let mut registry = LineageRegistry::new();
    let children: Vec<PlannedChild> = (0..MAX_SUITE_CHILDREN)
        .map(|i| cost_stress_child(&format!("profile-{i}")))
        .collect();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children,
    };

    let plan = plan_suite(&mut registry, None, request).expect("exactly-at-limit grid is allowed");
    assert_eq!(plan.items.len(), MAX_SUITE_CHILDREN);
    assert_eq!(registry.len(), MAX_SUITE_CHILDREN);
}

// ---------------------------------------------------------------------------
// One-axis child configs
// ---------------------------------------------------------------------------

#[test]
fn robustness_orchestration_plans_one_axis_per_child_with_deterministic_idempotency_keys() {
    let mut registry = LineageRegistry::new();
    let parent_run_id = Uuid::new_v4();
    let request = SuiteRequest {
        parent_run_id,
        parent: common::provenance(),
        children: vec![
            PlannedChild {
                axes: vec![DerivedAxis::CostStress {
                    profile_id: "adverse".to_owned(),
                    profile_version: 2,
                }],
                provenance: common::derived_provenance(),
            },
            PlannedChild {
                axes: vec![DerivedAxis::ExecutionDelay { delay_sessions: 1 }],
                provenance: common::derived_provenance(),
            },
            PlannedChild {
                axes: vec![DerivedAxis::BenchmarkComparison {
                    benchmark_id: "069500.KRX".to_owned(),
                }],
                provenance: common::derived_provenance(),
            },
        ],
    };

    let plan = plan_suite(&mut registry, None, request).expect("valid one-axis children plan");
    assert_eq!(plan.parent_run_id, parent_run_id);
    assert_eq!(plan.items.len(), 3);

    // Each lineage row pins the SAME parent and changes EXACTLY one axis;
    // idempotency keys are derived from the lineage run id and therefore
    // unique per child.
    let mut keys = BTreeSet::new();
    for item in &plan.items {
        assert_eq!(item.lineage.parent_run_id, parent_run_id);
        assert_eq!(
            item.idempotency_key,
            format!("robustness-child:{}", item.lineage.run_id)
        );
        keys.insert(item.idempotency_key.clone());
    }
    assert_eq!(keys.len(), 3, "idempotency keys must be pairwise distinct");

    // Re-planning the identical suite is idempotent at the lineage level:
    // the SAME run ids come back (crash-safe suite re-planning).
    let request_again = SuiteRequest {
        parent_run_id,
        parent: common::provenance(),
        children: vec![
            PlannedChild {
                axes: vec![DerivedAxis::CostStress {
                    profile_id: "adverse".to_owned(),
                    profile_version: 2,
                }],
                provenance: common::derived_provenance(),
            },
            PlannedChild {
                axes: vec![DerivedAxis::ExecutionDelay { delay_sessions: 1 }],
                provenance: common::derived_provenance(),
            },
            PlannedChild {
                axes: vec![DerivedAxis::BenchmarkComparison {
                    benchmark_id: "069500.KRX".to_owned(),
                }],
                provenance: common::derived_provenance(),
            },
        ],
    };
    let plan_again = plan_suite(&mut registry, None, request_again).expect("re-plan succeeds");
    let ids: BTreeSet<Uuid> = plan.items.iter().map(|i| i.lineage.run_id).collect();
    let ids_again: BTreeSet<Uuid> = plan_again.items.iter().map(|i| i.lineage.run_id).collect();
    assert_eq!(
        ids, ids_again,
        "re-planning must resolve to the SAME children"
    );
    assert_eq!(
        registry.len(),
        3,
        "re-planning must never duplicate lineage rows"
    );
}

#[test]
fn robustness_orchestration_rejects_a_child_that_changes_two_axes() {
    let mut registry = LineageRegistry::new();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![PlannedChild {
            axes: vec![
                DerivedAxis::CostStress {
                    profile_id: "adverse".to_owned(),
                    profile_version: 1,
                },
                DerivedAxis::ExecutionDelay { delay_sessions: 1 },
            ],
            provenance: common::derived_provenance(),
        }],
    };

    let err =
        plan_suite(&mut registry, None, request).expect_err("two-axis child must be rejected");
    assert!(matches!(err, RobustnessError::MultiAxisChange { count: 2 }));
    assert!(
        registry.is_empty(),
        "a rejected multi-axis child must not partially register the suite"
    );
}

// ---------------------------------------------------------------------------
// Version pinning
// ---------------------------------------------------------------------------

#[test]
fn robustness_orchestration_rejects_a_child_that_does_not_pin_the_parent_strategy_version() {
    let mut registry = LineageRegistry::new();
    let mut mismatched = common::derived_provenance();
    mismatched.strategy_version = domain::version::StrategyVersion::parse("9.9.9").unwrap();

    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![
            cost_stress_child("adverse"),
            PlannedChild {
                axes: vec![DerivedAxis::ExecutionDelay { delay_sessions: 1 }],
                provenance: mismatched,
            },
        ],
    };

    let err = plan_suite(&mut registry, None, request)
        .expect_err("a child that does not pin the parent's strategy version must be rejected");
    assert!(matches!(
        err,
        RobustnessError::PinMismatch {
            field: "strategy_version",
            ..
        }
    ));
    assert!(
        registry.is_empty(),
        "the earlier, correctly-pinned child must NOT have been committed either -- \
         planning is all-or-nothing"
    );
}

// ---------------------------------------------------------------------------
// Holdout non-access
// ---------------------------------------------------------------------------

#[test]
fn robustness_orchestration_rejects_a_period_split_child_that_reads_the_holdout() {
    let mut registry = LineageRegistry::new();
    let barrier = HoldoutBarrier::new(&split());
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![
            cost_stress_child("adverse"),
            PlannedChild {
                // validation_end reaches past the barrier's 2024-12-31.
                axes: vec![DerivedAxis::PeriodSplit {
                    train_end: "2024-06-30".to_owned(),
                    validation_end: "2025-03-31".to_owned(),
                }],
                provenance: common::derived_provenance(),
            },
        ],
    };

    let err = plan_suite(&mut registry, Some(&barrier), request)
        .expect_err("a period-split child that reads past the holdout must be rejected");
    assert!(matches!(
        err,
        RobustnessError::HoldoutViolation { date } if date == "2025-03-31"
    ));
    assert!(
        registry.is_empty(),
        "planning is all-or-nothing: the sibling cost-stress child must not land either"
    );
}

#[test]
fn robustness_orchestration_allows_a_period_split_child_within_the_holdout() {
    let mut registry = LineageRegistry::new();
    let barrier = HoldoutBarrier::new(&split());
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![PlannedChild {
            axes: vec![DerivedAxis::PeriodSplit {
                train_end: "2024-03-31".to_owned(),
                validation_end: "2024-09-30".to_owned(),
            }],
            provenance: common::derived_provenance(),
        }],
    };

    let plan = plan_suite(&mut registry, Some(&barrier), request)
        .expect("a period split entirely within train+validation is allowed");
    assert_eq!(plan.items.len(), 1);
}

// ---------------------------------------------------------------------------
// Promotion refusal without all required evidence
// ---------------------------------------------------------------------------

fn stability_evidence() -> StabilityEvidence {
    StabilityEvidence {
        validation_monthly_excess: vec![stat(0.01), stat(0.02), stat(-0.01)],
        neighborhood_returns: vec![stat(0.10), stat(0.11), stat(0.09)],
        parent_return: stat(0.10),
        cost_stress_final_returns: vec![stat(0.08), stat(0.07)],
        max_drawdown: stat(-0.12),
        volatility: stat(0.15),
        top_trade_share: stat(0.2),
        year_max_share: stat(0.25),
        recent_excess: stat(0.01),
        turnover: stat(1.1),
    }
}

fn manifests() -> EvidenceManifests {
    EvidenceManifests {
        golden_manifest_hash: "golden-hash".to_owned(),
        holdout_manifest_hash: "holdout-hash".to_owned(),
        cost_manifest_hash: "cost-hash".to_owned(),
    }
}

#[test]
fn robustness_orchestration_refuses_promotion_evidence_when_one_child_is_missing() {
    let mut registry = LineageRegistry::new();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![cost_stress_child("adverse"), cost_stress_child("extreme")],
    };
    let plan = plan_suite(&mut registry, None, request).unwrap();
    let stability = analyze_stability(&stability_evidence()).unwrap();

    // Only ONE of the two children actually succeeded.
    let outcomes = vec![SuiteChildOutcome {
        run_id: plan.items[0].lineage.run_id,
        succeeded: true,
    }];

    let err = assemble_evidence_bundle(&plan, &outcomes, manifests(), stability)
        .expect_err("incomplete suite evidence must be refused");
    assert!(matches!(
        err,
        RobustnessError::IncompleteSuite {
            expected: 2,
            have: 1
        }
    ));
}

#[test]
fn robustness_orchestration_refuses_promotion_evidence_when_a_child_failed() {
    let mut registry = LineageRegistry::new();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![cost_stress_child("adverse"), cost_stress_child("extreme")],
    };
    let plan = plan_suite(&mut registry, None, request).unwrap();
    let stability = analyze_stability(&stability_evidence()).unwrap();

    let outcomes = vec![
        SuiteChildOutcome {
            run_id: plan.items[0].lineage.run_id,
            succeeded: true,
        },
        SuiteChildOutcome {
            run_id: plan.items[1].lineage.run_id,
            succeeded: false,
        },
    ];

    let err = assemble_evidence_bundle(&plan, &outcomes, manifests(), stability)
        .expect_err("a FAILED child must refuse promotion evidence, not just a missing one");
    assert!(matches!(
        err,
        RobustnessError::IncompleteSuite {
            expected: 2,
            have: 1
        }
    ));
}

#[test]
fn robustness_orchestration_assembles_evidence_when_every_child_succeeded() {
    let mut registry = LineageRegistry::new();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![cost_stress_child("adverse"), cost_stress_child("extreme")],
    };
    let plan = plan_suite(&mut registry, None, request).unwrap();
    let stability = analyze_stability(&stability_evidence()).unwrap();

    let outcomes: Vec<SuiteChildOutcome> = plan
        .items
        .iter()
        .map(|item| SuiteChildOutcome {
            run_id: item.lineage.run_id,
            succeeded: true,
        })
        .collect();

    let bundle = assemble_evidence_bundle(&plan, &outcomes, manifests(), stability)
        .expect("a fully-succeeded suite must assemble evidence");
    assert_eq!(bundle.parent_run_id, plan.parent_run_id);
    assert!(bundle.stability.reference_only);
}

#[test]
fn robustness_orchestration_score_alone_can_never_approve_a_promotion() {
    // Even a COMPLETE, successfully-assembled bundle's score cannot itself
    // become an approval (design §9.6): the score is advisory evidence
    // INSIDE the bundle, never a substitute for the bundle's manifests.
    let mut registry = LineageRegistry::new();
    let request = SuiteRequest {
        parent_run_id: Uuid::new_v4(),
        parent: common::provenance(),
        children: vec![cost_stress_child("adverse")],
    };
    let plan = plan_suite(&mut registry, None, request).unwrap();
    let stability = analyze_stability(&stability_evidence()).unwrap();
    let outcomes = vec![SuiteChildOutcome {
        run_id: plan.items[0].lineage.run_id,
        succeeded: true,
    }];
    let bundle = assemble_evidence_bundle(&plan, &outcomes, manifests(), stability).unwrap();

    let approval = result_model::robustness::approve_investment(&bundle.stability);
    assert!(matches!(
        approval,
        Err(RobustnessError::StabilityScoreNotApproval)
    ));
}
