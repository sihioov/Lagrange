//! Todo 21 RED tests: parent/derived run lineage (design §9.5).
//!
//! Every derived run pins the parent's strategy/data/engine and changes
//! EXACTLY ONE declared axis:
//!   - one-axis changes register with a deterministic lineage row;
//!   - mutating TWO axes is rejected with `MultiAxisChange`;
//!   - pinning violations (strategy/data/engine drift) are rejected with
//!     `PinMismatch` naming the drifted field;
//!   - lineage is deterministic: identical requests yield identical run ids
//!     across registries, and distinct axes yield distinct run ids.

mod common;

use domain::version::{SemVer, StrategyVersion};
use domain::DatasetVersionId;
use serde_json::json;
use uuid::Uuid;

use result_model::robustness::{
    DerivedAxis, DerivedRunRequest, LineageRegistry, PinnedContext, RobustnessError, RunLineage,
};

fn parameter_axis() -> DerivedAxis {
    DerivedAxis::ParameterNeighborhood {
        parameter: "fast_ma".to_owned(),
        delta: json!({"period": 20}),
    }
}

#[test]
fn derived_run_pins_strategy_data_engine_and_changes_one_axis() {
    let parent = common::provenance();
    let derived = common::derived_provenance();
    let mut registry = LineageRegistry::new();

    let lineage = registry
        .register_derived(DerivedRunRequest {
            parent_run_id: Uuid::new_v4(),
            parent: parent.clone(),
            changes: vec![parameter_axis()],
            derived_provenance: derived.clone(),
        })
        .expect("one-axis change with pinned context must register");

    assert_eq!(
        lineage.pinned,
        PinnedContext::from_provenance(&parent),
        "derived run must pin the parent's strategy/data/engine"
    );
    assert_eq!(lineage.changed_axis.code(), "parameter_neighborhood");
    assert_eq!(lineage.pinned.engine, "nautilustrader");
    assert_eq!(lineage.pinned.engine_version, "1.231.0");
    assert_eq!(lineage.pinned.strategy_version, "1.2.0");
    assert_eq!(lineage.pinned.dataset_version, "kr-etf-daily-20260804.1");
    // The derived config hash is recorded verbatim for reproducibility.
    assert_eq!(lineage.derived_config_hash, derived.config_hash.to_string());
    assert_eq!(registry.len(), 1);
}

#[test]
fn two_axis_mutation_is_rejected() {
    let parent = common::provenance();
    let derived = common::derived_provenance();
    let mut registry = LineageRegistry::new();

    let request = DerivedRunRequest {
        parent_run_id: Uuid::new_v4(),
        parent: parent.clone(),
        changes: vec![
            DerivedAxis::CostStress {
                profile_id: "krx_etf_default".to_owned(),
                profile_version: 2,
            },
            DerivedAxis::ExecutionDelay {
                delay_sessions: 1,
            },
        ],
        derived_provenance: derived,
    };
    let error = registry.register_derived(request).expect_err(
        "a derived run that mutates two axes must be rejected (design §9.5 one variable only)",
    );
    assert_eq!(registry.len(), 0, "no lineage row may exist after a rejection");
    match error {
        RobustnessError::MultiAxisChange { count } => assert_eq!(
            count, 2,
            "MultiAxisChange must report the exact axis count (got {count})"
        ),
        other => panic!("expected MultiAxisChange, got: {other:?}"),
    }
}

#[test]
fn pinning_violations_are_rejected_per_field() {
    let parent = common::provenance();
    let mut registry = LineageRegistry::new();

    // strategy version drift
    let mut derived = common::derived_provenance();
    derived.strategy_version = StrategyVersion::parse("2.0.0").unwrap();
    let error = registry
        .register_derived(DerivedRunRequest {
            parent_run_id: Uuid::new_v4(),
            parent: parent.clone(),
            changes: vec![parameter_axis()],
            derived_provenance: derived,
        })
        .expect_err("strategy-version drift must be rejected");
    assert!(matches!(
        error,
        RobustnessError::PinMismatch {
            field: "strategy_version",
            ..
        }
    ));

    // dataset version drift
    let mut derived = common::derived_provenance();
    derived.dataset_version = DatasetVersionId::parse("kr-etf-daily-20260805.1").unwrap();
    let error = registry
        .register_derived(DerivedRunRequest {
            parent_run_id: Uuid::new_v4(),
            parent: parent.clone(),
            changes: vec![parameter_axis()],
            derived_provenance: derived,
        })
        .expect_err("dataset-version drift must be rejected");
    assert!(matches!(
        error,
        RobustnessError::PinMismatch {
            field: "dataset_version",
            ..
        }
    ));

    // engine version drift
    let mut derived = common::derived_provenance();
    derived.engine_version = SemVer::parse("2.0.0").unwrap();
    let error = registry
        .register_derived(DerivedRunRequest {
            parent_run_id: Uuid::new_v4(),
            parent,
            changes: vec![parameter_axis()],
            derived_provenance: derived,
        })
        .expect_err("engine-version drift must be rejected");
    assert!(matches!(
        error,
        RobustnessError::PinMismatch {
            field: "engine_version",
            ..
        }
    ));
}

#[test]
fn lineage_is_deterministic_and_idempotent() {
    let parent = common::provenance();
    let derived = common::derived_provenance();
    let parent_run_id = Uuid::new_v4();

    let request = || DerivedRunRequest {
        parent_run_id,
        parent: parent.clone(),
        changes: vec![parameter_axis()],
        derived_provenance: derived.clone(),
    };

    // Identical requests across FRESH registries produce the same run id
    // (uuid5 over parent|axis|pinned — deterministic, no randomness).
    let mut a = LineageRegistry::new();
    let mut b = LineageRegistry::new();
    let lineage_a = a.register_derived(request()).unwrap();
    let lineage_b = b.register_derived(request()).unwrap();
    assert_eq!(lineage_a.run_id, lineage_b.run_id);
    assert_eq!(lineage_a.parent_run_id, parent_run_id);

    // Re-registering in the same registry is a no-op (same row).
    let again = a.register_derived(request()).unwrap();
    assert_eq!(again.run_id, lineage_a.run_id);
    assert_eq!(a.len(), 1);

    // A DIFFERENT axis yields a different run id.
    let other = a
        .register_derived(DerivedRunRequest {
            parent_run_id,
            parent: parent.clone(),
            changes: vec![DerivedAxis::BenchmarkComparison {
                benchmark_id: "069500.KRX".to_owned(),
            }],
            derived_provenance: common::derived_provenance(),
        })
        .unwrap();
    assert_ne!(other.run_id, lineage_a.run_id);

    // children_of is deterministic (sorted) and complete.
    let children: Vec<&RunLineage> = a.children_of(parent_run_id);
    assert_eq!(children.len(), 2);
    assert!(children[0].run_id < children[1].run_id);
}
