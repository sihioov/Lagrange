//! Phase 1 core release gate (plan Todo 21, requirements §12.2).
//!
//! The gate is the machine-readable APPROVED/BLOCKED decision over:
//!   - the committed golden artifacts for ALL FIVE baseline strategies
//!     (recommendation/orders/fills/equity/fees/metrics/provenance per
//!     §12.2), verified by SHA-256 — an unapproved golden delta BLOCKS the
//!     gate with the quoted expected-vs-actual diff;
//!   - the AT-03/04/05/06 core evidence (duplicate request → prior run,
//!     deterministic rerun, higher cost ends lower, reconciled fees,
//!     missing-data policy, worker-kill one ORPHANED attempt / max one
//!     retry), holdout non-access, and the stability score's reference-only
//!     guard;
//!   - finiteness: a NaN raw statistic is a typed rejection at the boundary
//!     ([`CoreEvidenceBundle::with_raw_stat`]) and a `malformed_evidence`
//!     block on the JSON path — the gate never panics.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::robustness::RobustnessError;

/// The five baseline strategy ids (plan Todo 17 registry).
pub const FIVE_STRATEGIES: [&str; 5] = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
];

/// The seven canonical golden artifacts of §12.2.
pub const CANONICAL_ARTIFACTS: [&str; 7] = [
    "recommendation",
    "orders",
    "fills",
    "equity",
    "fees",
    "metrics",
    "provenance",
];

/// One golden artifact reference (id, relative path, committed hash).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GoldenManifestEntry {
    /// e.g. `buy_and_hold/recommendation`.
    pub id: String,
    /// Path relative to the golden base directory.
    pub path: String,
    /// Committed content hash (`sha256:<hex>`).
    pub sha256: String,
}

/// The committed golden manifest for the five strategies (§12.2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GoldenSet {
    pub golden_id: String,
    pub versions: serde_json::Value,
    pub artifacts: Vec<GoldenManifestEntry>,
}

/// The core release evidence (golden artifacts + AT-03/04/05/06 facts).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CoreEvidenceBundle {
    pub golden_set: GoldenSet,
    /// (entry id, artifact bytes) produced by the run under evaluation.
    pub artifacts: Vec<(String, Vec<u8>)>,
    /// Extra raw statistics that MUST be finite (NaN → gate block).
    #[serde(default)]
    pub raw_stats: Vec<(String, f64)>,
    pub at03_duplicate_request_returns_prior_run: bool,
    pub at03_deterministic_rerun_identical: bool,
    pub at04_higher_cost_ends_lower: bool,
    pub at04_fees_reconciled: bool,
    pub at05_missing_data_policy_obeyed: bool,
    pub at06_worker_kill_one_orphan_max_one_retry: bool,
    pub holdout_not_read_during_selection: bool,
    pub stability_score_reference_only: bool,
}

impl CoreEvidenceBundle {
    /// Records a raw statistic, rejecting non-finite values as a typed
    /// [`RobustnessError::NonFinite`] at the boundary (never a panic).
    pub fn with_raw_stat(&mut self, field: &str, value: f64) -> Result<(), RobustnessError> {
        if !value.is_finite() {
            return Err(RobustnessError::NonFinite {
                field: field.to_owned(),
            });
        }
        self.raw_stats.push((field.to_owned(), value));
        Ok(())
    }
}

/// One gate item: a named assertion plus its outcome.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GateItem {
    pub id: String,
    pub passed: bool,
    pub detail: String,
}

/// The machine-readable gate verdict.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CoreReleaseVerdict {
    pub approved: bool,
    pub items: Vec<GateItem>,
}

impl CoreReleaseVerdict {
    /// `true` only when EVERY item passed.
    pub fn is_approved(&self) -> bool {
        self.approved
    }

    /// The failing items (deterministic order).
    pub fn failed_items(&self) -> Vec<&GateItem> {
        self.items.iter().filter(|item| !item.passed).collect()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn item(passed: bool, id: &str, detail: String) -> GateItem {
    GateItem {
        id: id.to_owned(),
        passed,
        detail,
    }
}

/// Evaluates the core release evidence (deterministic, machine-readable).
pub fn evaluate_core_release(bundle: &CoreEvidenceBundle) -> CoreReleaseVerdict {
    let mut items: Vec<GateItem> = Vec::new();

    // 1. The golden set must cover all five strategies with the seven
    //    canonical artifacts.
    let entry_ids: std::collections::BTreeSet<&str> =
        bundle.golden_set.artifacts.iter().map(|e| e.id.as_str()).collect();
    let mut missing = Vec::new();
    for strategy in FIVE_STRATEGIES {
        for artifact in CANONICAL_ARTIFACTS {
            let id = format!("{strategy}/{artifact}");
            if !entry_ids.contains(id.as_str()) {
                missing.push(id);
            }
        }
    }
    if missing.is_empty() {
        items.push(item(
            true,
            "golden_set_complete",
            format!("all {} strategies x {} artifacts present", FIVE_STRATEGIES.len(), CANONICAL_ARTIFACTS.len()),
        ));
    } else {
        items.push(item(
            false,
            "golden_set_complete",
            format!("golden set is incomplete; missing: {}", missing.join(", ")),
        ));
    }

    // 2. Every artifact must match its committed hash.
    for entry in &bundle.golden_set.artifacts {
        let actual = bundle
            .artifacts
            .iter()
            .find(|(id, _)| id == &entry.id)
            .map(|(_, bytes)| sha256_hex(bytes))
            .unwrap_or_else(|| "sha256:<artifact-not-provided>".to_owned());
        let id = format!("golden_artifact:{}", entry.id);
        if actual == entry.sha256 {
            items.push(item(
                true,
                &id,
                format!("{} matches committed {}", entry.path, entry.sha256),
            ));
        } else {
            items.push(item(
                false,
                &id,
                format!(
                    "unapproved golden delta at {}: expected {} got {}",
                    entry.path, entry.sha256, actual
                ),
            ));
        }
    }

    // 3. Finiteness of every raw statistic.
    let non_finite: Vec<&str> = bundle
        .raw_stats
        .iter()
        .filter(|(_, value)| !value.is_finite())
        .map(|(field, _)| field.as_str())
        .collect();
    if non_finite.is_empty() {
        items.push(item(
            true,
            "evidence_finite",
            format!("{} raw statistics are finite", bundle.raw_stats.len()),
        ));
    } else {
        items.push(item(
            false,
            "malformed_evidence",
            format!("non-finite raw statistics: {}", non_finite.join(", ")),
        ));
    }

    // 4. AT-03/04/05/06 + holdout + stability guard evidence.
    let evidence: Vec<(&'static str, bool, String)> = vec![
        (
            "at03_duplicate_request",
            bundle.at03_duplicate_request_returns_prior_run,
            "duplicate request returns the prior run (FR-BT-008 / AT-03)".to_owned(),
        ),
        (
            "at03_deterministic_rerun",
            bundle.at03_deterministic_rerun_identical,
            "identical input twice yields identical trades/equity/metrics (AT-03)".to_owned(),
        ),
        (
            "at04_higher_cost_ends_lower",
            bundle.at04_higher_cost_ends_lower,
            "higher fees/slippage lower the final equity (AT-04)".to_owned(),
        ),
        (
            "at04_fees_reconciled",
            bundle.at04_fees_reconciled,
            "cost totals reconcile with the trade records (AT-04)".to_owned(),
        ),
        (
            "at05_missing_data_policy",
            bundle.at05_missing_data_policy_obeyed,
            "missing data obeys the declared policy (AT-05)".to_owned(),
        ),
        (
            "at06_worker_kill",
            bundle.at06_worker_kill_one_orphan_max_one_retry,
            "worker kill yields one ORPHANED attempt and at most one retry (AT-06)".to_owned(),
        ),
        (
            "holdout_not_read",
            bundle.holdout_not_read_during_selection,
            "the final test period is never read during selection (FR-ROB-001)".to_owned(),
        ),
        (
            "stability_score_reference_only",
            bundle.stability_score_reference_only,
            "the stability score is a reference indicator, not an approval (design 9.6)".to_owned(),
        ),
    ];
    for (id, passed, detail) in evidence {
        items.push(item(passed, id, detail));
    }

    let approved = items.iter().all(|i| i.passed);
    CoreReleaseVerdict { approved, items }
}

/// Parses the evidence bundle from JSON and evaluates it. Malformed JSON
/// (including non-finite floats) BLOCKS the gate with a `malformed_evidence`
/// item — never a panic.
pub fn evaluate_core_release_json(json: &str) -> CoreReleaseVerdict {
    match serde_json::from_str::<CoreEvidenceBundle>(json) {
        Ok(bundle) => evaluate_core_release(&bundle),
        Err(error) => CoreReleaseVerdict {
            approved: false,
            items: vec![item(
                false,
                "malformed_evidence",
                format!("evidence JSON is malformed: {error}"),
            )],
        },
    }
}

/// Loads a golden set manifest plus its artifact bytes from disk.
///
/// `manifest_path` is the committed `golden-set.json`; artifact paths in the
/// manifest resolve relative to `base_dir`.
pub fn load_golden_set(
    base_dir: &Path,
    manifest_path: &Path,
) -> Result<(GoldenSet, Vec<(String, Vec<u8>)>), RobustnessError> {
    let text = std::fs::read_to_string(manifest_path).map_err(|e| RobustnessError::Replay {
        detail: format!("read golden manifest {}: {e}", manifest_path.display()),
    })?;
    let golden_set: GoldenSet =
        serde_json::from_str(&text).map_err(|e| RobustnessError::Replay {
            detail: format!("parse golden manifest: {e}"),
        })?;
    let mut artifacts = Vec::new();
    for entry in &golden_set.artifacts {
        let path = base_dir.join(&entry.path);
        let bytes = std::fs::read(&path).map_err(|e| RobustnessError::Replay {
            detail: format!("read golden artifact {}: {e}", path.display()),
        })?;
        let actual = sha256_hex(&bytes);
        if actual != entry.sha256 {
            return Err(RobustnessError::Replay {
                detail: format!(
                    "committed golden artifact {} does not match its manifest: expected {} got {}",
                    entry.path, entry.sha256, actual
                ),
            });
        }
        artifacts.push((entry.id.clone(), bytes));
    }
    Ok((golden_set, artifacts))
}
