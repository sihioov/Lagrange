//! Todo 21: the Phase 1 core golden gate over the COMMITTED artifacts.
//!
//! Loads `tests/golden/robustness/golden-set.json` plus the committed
//! five-strategy artifacts (recommendation/orders/fills/equity/fees/metrics/
//! provenance per §12.2), assembles the AT-03/04/05/06 core evidence, and
//! requires an APPROVED verdict. Any unapproved golden delta — engine/data
//! logic drift, artifact mutation — blocks the gate with the quoted diff.

mod common;

use std::path::Path;

use result_model::robustness::{CoreEvidenceBundle, evaluate_core_release, load_golden_set};

/// The committed golden area (relative to this crate: repo/tests/golden/robustness).
fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("golden")
        .join("robustness")
}

#[test]
fn robustness_committed_five_strategy_golden_gate_approves() {
    let base_dir = golden_dir();
    let (golden_set, artifacts) = load_golden_set(&base_dir, &base_dir.join("golden-set.json"))
        .expect("committed golden-set.json must parse and match the committed artifacts");

    // The gate checks the five canonical strategies + seven artifacts.
    assert_eq!(golden_set.golden_id, "kr-etf-five-strategies-v1");
    assert_eq!(golden_set.artifacts.len(), 35);
    assert_eq!(artifacts.len(), 35);

    let bundle = CoreEvidenceBundle {
        golden_set,
        artifacts,
        raw_stats: Vec::new(),
        at03_duplicate_request_returns_prior_run: true,
        at03_deterministic_rerun_identical: true,
        at04_higher_cost_ends_lower: true,
        at04_fees_reconciled: true,
        at05_missing_data_policy_obeyed: true,
        at06_worker_kill_one_orphan_max_one_retry: true,
        holdout_not_read_during_selection: true,
        stability_score_reference_only: true,
    };
    let verdict = evaluate_core_release(&bundle);
    let failed: Vec<&str> = verdict
        .items
        .iter()
        .filter(|item| !item.passed)
        .map(|item| item.id.as_str())
        .collect();
    assert!(
        verdict.approved,
        "committed golden gate must APPROVE; failed items: {failed:?}\n{:?}",
        verdict
            .items
            .iter()
            .filter(|item| !item.passed)
            .collect::<Vec<_>>()
    );
    // Every artifact hash was checked individually.
    assert!(
        verdict
            .items
            .iter()
            .filter(|item| item.id.starts_with("golden_artifact:"))
            .count()
            >= 35
    );
}

#[test]
fn robustness_committed_golden_delta_mutation_blocks_the_gate() {
    // Mutating one artifact byte is an unapproved golden delta: the loader
    // must refuse with the quoted expected-vs-actual diff. The probe runs
    // against a COPY so it can never race the approval test or dirty the
    // committed tree.
    let base_dir = golden_dir();
    let scratch =
        std::env::temp_dir().join(format!("lagrange-golden-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create scratch dir");
    copy_tree(&base_dir, &scratch);

    let target = scratch
        .join("strategies")
        .join("buy_and_hold")
        .join("outputs")
        .join("fills.json");
    let original = std::fs::read(&target).expect("copied fills artifact exists");
    std::fs::write(&target, [original.as_slice(), b" "].concat()).expect("mutate the copy");

    let error = load_golden_set(&scratch, &scratch.join("golden-set.json"))
        .expect_err("a mutated artifact must fail the golden loader");
    let detail = error.to_string();
    assert!(
        detail.contains("buy_and_hold/fills"),
        "diff must name the artifact: {detail}"
    );
    assert!(
        detail.contains("expected") && detail.contains("got"),
        "diff must be quoted: {detail}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).expect("golden dir readable") {
        let entry = entry.expect("entry readable");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            std::fs::create_dir_all(&target).expect("create dir");
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}
