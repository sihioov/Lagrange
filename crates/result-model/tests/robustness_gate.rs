//! Todo 21 RED tests: the Phase 1 core release gate.
//!
//! The gate verifies the committed golden artifacts for all five strategies
//! plus the AT-03/04/05/06 core evidence, and REFUSES:
//!   - an unapproved golden delta (hash mismatch, quoted field-level diff);
//!   - an incomplete golden set;
//!   - a NaN raw statistic (typed rejection at the boundary, and a blocked
//!     verdict on the JSON path — never a panic);
//!   - missing AT evidence.
//!
//! The gate is deterministic and machine-readable (APPROVED/BLOCKED items).

mod common;

use sha2::{Digest, Sha256};

use result_model::robustness::{
    CoreEvidenceBundle, CoreReleaseVerdict, GoldenManifestEntry, GoldenSet, RobustnessError,
    evaluate_core_release, evaluate_core_release_json,
};

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

/// A synthetic golden set covering the five strategies with the seven
/// canonical artifacts each (recommendation/orders/fills/equity/fees/
/// metrics/provenance).
fn five_strategy_golden_set() -> (GoldenSet, Vec<(String, Vec<u8>)>) {
    let strategies = [
        "buy_and_hold",
        "trend_following",
        "relative_momentum",
        "dual_momentum",
        "inverse_volatility",
    ];
    let artifacts = [
        "recommendation",
        "orders",
        "fills",
        "equity",
        "fees",
        "metrics",
        "provenance",
    ];
    let mut entries = Vec::new();
    let mut bytes_map = Vec::new();
    for strategy in strategies {
        for artifact in artifacts {
            let id = format!("{strategy}/{artifact}");
            let content =
                format!("{{{{\"strategy\":\"{strategy}\",\"artifact\":\"{artifact}\",\"v\":1}}}}");
            let bytes = content.into_bytes();
            entries.push(GoldenManifestEntry {
                id: id.clone(),
                path: format!("strategies/{strategy}/outputs/{artifact}.json"),
                sha256: sha256_hex(&bytes),
            });
            bytes_map.push((id, bytes));
        }
    }
    let set = GoldenSet {
        golden_id: "kr-etf-five-strategies-v1".to_owned(),
        versions: serde_json::json!({
            "engine": {"name": "lagrange-golden-sim", "version": "1.0.0"},
            "timezone": "Asia/Seoul",
        }),
        artifacts: entries,
    };
    (set, bytes_map)
}

fn full_evidence() -> CoreEvidenceBundle {
    let (golden_set, artifacts) = five_strategy_golden_set();
    CoreEvidenceBundle {
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
    }
}

#[test]
fn robustness_full_evidence_approves_the_release() {
    let bundle = full_evidence();
    let verdict = evaluate_core_release(&bundle);
    let failed: Vec<&str> = verdict
        .items
        .iter()
        .filter(|item| !item.passed)
        .map(|item| item.id.as_str())
        .collect();
    assert!(
        verdict.approved,
        "full evidence must APPROVE; failed: {failed:?}"
    );
    // Every artifact hash was checked individually.
    assert!(
        verdict
            .items
            .iter()
            .any(|item| item.id.starts_with("golden_artifact:"))
    );
}

#[test]
fn robustness_unapproved_golden_delta_is_blocked_with_quoted_diff() {
    let mut bundle = full_evidence();
    // Mutate ONE artifact byte: an unapproved golden delta.
    let target = bundle
        .artifacts
        .iter_mut()
        .find(|(id, _)| id == "buy_and_hold/fills")
        .unwrap();
    target.1.push(b'X');
    let verdict = evaluate_core_release(&bundle);
    assert!(
        !verdict.approved,
        "unapproved golden delta must BLOCK the gate"
    );
    let item = verdict
        .items
        .iter()
        .find(|item| item.id == "golden_artifact:buy_and_hold/fills")
        .expect("the mutated artifact must be reported");
    assert!(!item.passed);
    assert!(
        item.detail.contains(&sha256_hex(&[])[..8]) || item.detail.contains("expected"),
        "the failing item must carry the field-level diff: {}",
        item.detail
    );
    assert!(
        item.detail.contains("got"),
        "detail must show the actual hash"
    );
}

#[test]
fn robustness_missing_evidence_item_blocks_the_gate() {
    let mut bundle = full_evidence();
    bundle.at06_worker_kill_one_orphan_max_one_retry = false;
    let verdict = evaluate_core_release(&bundle);
    assert!(!verdict.approved);
    let item = verdict
        .items
        .iter()
        .find(|item| item.id == "at06_worker_kill")
        .expect("the failed evidence item must be reported by name");
    assert!(!item.passed);
}

#[test]
fn robustness_nan_raw_stat_is_rejected_at_the_boundary() {
    let mut bundle = full_evidence();
    let error = bundle
        .with_raw_stat("concentration_ratio", f64::NAN)
        .expect_err("NaN must be rejected as a typed error at the boundary");
    assert!(matches!(error, RobustnessError::NonFinite { .. }));
    // The gate still evaluates cleanly for the untouched bundle.
    assert!(evaluate_core_release(&bundle).approved);
}

#[test]
fn robustness_nan_in_evidence_json_blocks_the_gate_without_panicking() {
    let bundle = full_evidence();
    let json = serde_json::to_string(&bundle).unwrap();
    // Inject a NaN raw statistic into the evidence JSON.
    let poisoned = json.replace(
        "\"artifacts\":",
        "\"raw_stats\":[{\"field\":\"x\",\"value\":NaN}],\"artifacts\":",
    );
    let verdict = evaluate_core_release_json(&poisoned);
    assert!(
        !verdict.approved,
        "a NaN in the evidence must BLOCK the gate"
    );
    assert!(
        verdict
            .items
            .iter()
            .any(|item| item.id == "malformed_evidence"),
        "the blocked item must be named malformed_evidence"
    );
}

#[test]
fn robustness_incomplete_golden_set_blocks() {
    let mut bundle = full_evidence();
    bundle
        .golden_set
        .artifacts
        .retain(|entry| !entry.id.starts_with("inverse_volatility/"));
    bundle
        .artifacts
        .retain(|(id, _)| !id.starts_with("inverse_volatility/"));
    let verdict = evaluate_core_release(&bundle);
    assert!(
        !verdict.approved,
        "four strategies must not pass the five-strategy gate"
    );
    let item = verdict
        .items
        .iter()
        .find(|item| item.id == "golden_set_complete")
        .expect("the incomplete set must be reported");
    assert!(!item.passed);
}

#[test]
fn robustness_verdict_is_deterministic() {
    let bundle = full_evidence();
    let a = evaluate_core_release(&bundle);
    let b = evaluate_core_release(&bundle);
    assert_eq!(a, b);
}

#[test]
fn robustness_verdict_is_machine_readable() {
    let verdict = evaluate_core_release(&full_evidence());
    let json = serde_json::to_string(&verdict).unwrap();
    assert!(json.contains("\"approved\":true"));
    assert!(json.contains("\"items\":"));
    // Round-trips.
    let back: CoreReleaseVerdict = serde_json::from_str(&json).unwrap();
    assert_eq!(back, verdict);
}
