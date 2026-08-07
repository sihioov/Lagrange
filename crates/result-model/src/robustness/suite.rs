//! Bounded robustness-suite planning and promotion-evidence assembly (plan
//! Todo 29, design §9.5-9.6).
//!
//! A suite plans N one-axis children under a single parent (design:
//! `RobustnessSuite` -> `ParameterNeighborhood|CostStress|PeriodSplit|
//! WalkForward|ExecutionDelay|BenchmarkComparison`). Planning is atomic:
//! every child is validated — grid size, exactly one axis, holdout safety,
//! parent pinning — BEFORE any lineage row is registered, so a single bad
//! child never leaves a half-registered suite behind. This module knows
//! nothing about the job queue; [`SuitePlanItem::idempotency_key`] is the
//! value a caller hands to `job_queue::batch::submit_batch` so crashed
//! re-planning resolves to the identical children instead of duplicates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use domain::provenance::RunProvenance;

use crate::robustness::RobustnessError;
use crate::robustness::holdout::HoldoutBarrier;
use crate::robustness::lineage::{
    DerivedAxis, DerivedRunRequest, LineageRegistry, PinnedContext, RunLineage, check_pin_match,
};
use crate::robustness::stability::StabilityScore;

/// Hard cap on children per suite (parameter-grid limit, design §9.5's
/// bounded-batch orchestration). A caller requesting more gets a typed
/// rejection before any lineage row is registered — never a silently
/// truncated grid.
pub const MAX_SUITE_CHILDREN: usize = 25;

/// One requested child: its axis change(s) and the provenance it claims.
///
/// `axes` mirrors [`DerivedRunRequest::changes`]'s shape (a `Vec`) so a
/// caller who mistakenly asks for more than one axis change on a single
/// child gets the same typed [`RobustnessError::MultiAxisChange`] the
/// lineage layer already defines, rather than a different error at a
/// different layer for the same mistake.
#[derive(Debug, Clone)]
pub struct PlannedChild {
    pub axes: Vec<DerivedAxis>,
    pub provenance: RunProvenance,
}

/// A suite orchestration request: one parent, N candidate children.
#[derive(Debug, Clone)]
pub struct SuiteRequest {
    pub parent_run_id: Uuid,
    pub parent: RunProvenance,
    pub children: Vec<PlannedChild>,
}

/// One planned, lineage-registered suite child ready for job submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuitePlanItem {
    pub lineage: RunLineage,
    /// Deterministic key for `job_queue::batch::submit_batch`: derived from
    /// the lineage run id, so re-submitting the same plan after a crash
    /// resolves to the same child jobs (AT-03 semantics at suite level).
    pub idempotency_key: String,
}

/// A fully planned suite: bounded, one-axis-per-child, holdout-safe, and
/// pinned to the parent's strategy/data/engine version.
#[derive(Debug, Clone)]
pub struct SuitePlan {
    pub parent_run_id: Uuid,
    pub items: Vec<SuitePlanItem>,
}

/// Guards a [`DerivedAxis`]'s own date fields against the holdout barrier.
/// Only [`DerivedAxis::PeriodSplit`] carries explicit dates; the other axes
/// have nothing here to guard (walk-forward folds are session counts, not
/// dates, and are guarded when the fold is actually computed).
fn guard_axis_dates(barrier: &HoldoutBarrier, axis: &DerivedAxis) -> Result<(), RobustnessError> {
    if let DerivedAxis::PeriodSplit {
        train_end,
        validation_end,
    } = axis
    {
        barrier.guard(train_end)?;
        barrier.guard(validation_end)?;
    }
    Ok(())
}

/// Plans a bounded robustness suite.
///
/// Validation runs in two passes so the suite is atomic: pass one checks
/// every child (grid size, exactly one axis, holdout dates, parent pinning)
/// without touching `registry`; only if every child passes does pass two
/// register the lineage rows. A single bad child therefore never leaves a
/// half-registered suite in `registry`.
///
/// `holdout` is `None` when the parent declares no train/validation split
/// (nothing to guard); `Some` enforces FR-ROB-001 against every
/// [`DerivedAxis::PeriodSplit`] child.
pub fn plan_suite(
    registry: &mut LineageRegistry,
    holdout: Option<&HoldoutBarrier>,
    request: SuiteRequest,
) -> Result<SuitePlan, RobustnessError> {
    if request.children.is_empty() {
        return Err(RobustnessError::EmptySeries {
            what: "robustness suite children".to_owned(),
        });
    }
    if request.children.len() > MAX_SUITE_CHILDREN {
        return Err(RobustnessError::GridTooLarge {
            requested: request.children.len(),
            max: MAX_SUITE_CHILDREN,
        });
    }

    let parent_pinned = PinnedContext::from_provenance(&request.parent);
    let mut validated = Vec::with_capacity(request.children.len());
    for child in &request.children {
        if child.axes.len() != 1 {
            return Err(RobustnessError::MultiAxisChange {
                count: child.axes.len(),
            });
        }
        if let Some(barrier) = holdout {
            guard_axis_dates(barrier, &child.axes[0])?;
        }
        let derived_pinned = PinnedContext::from_provenance(&child.provenance);
        check_pin_match(&parent_pinned, &derived_pinned)?;
        validated.push(DerivedRunRequest {
            parent_run_id: request.parent_run_id,
            parent: request.parent.clone(),
            changes: child.axes.clone(),
            derived_provenance: child.provenance.clone(),
        });
    }

    let mut items = Vec::with_capacity(validated.len());
    for derived_request in validated {
        let lineage = registry.register_derived(derived_request)?;
        let idempotency_key = format!("robustness-child:{}", lineage.run_id);
        items.push(SuitePlanItem {
            lineage,
            idempotency_key,
        });
    }
    Ok(SuitePlan {
        parent_run_id: request.parent_run_id,
        items,
    })
}

/// One planned child's terminal outcome, as it feeds evidence assembly.
#[derive(Debug, Clone, Copy)]
pub struct SuiteChildOutcome {
    pub run_id: Uuid,
    pub succeeded: bool,
}

/// The manifest references a `Draft -> Validated` promotion cites (mirrors
/// `selector::registry::PromotionEvidence::Golden`'s three hashes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceManifests {
    pub golden_manifest_hash: String,
    pub holdout_manifest_hash: String,
    pub cost_manifest_hash: String,
}

/// The suite-level evidence bundle a promotion decision may cite. Building
/// this bundle NEVER approves anything by itself (design §9.6): `stability`
/// stays a reference score, and nothing in this module or its return value
/// can turn it into an approval — only [`crate::robustness::stability::approve_investment`]
/// speaks to approval, and it always refuses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteEvidenceBundle {
    pub parent_run_id: Uuid,
    pub manifests: EvidenceManifests,
    pub stability: StabilityScore,
}

/// Assembles the suite-level evidence bundle. Refuses with
/// [`RobustnessError::IncompleteSuite`] unless EVERY planned child
/// succeeded — evidence must be complete, never partial (plan Todo 29:
/// "promotion refusal without all required evidence").
pub fn assemble_evidence_bundle(
    plan: &SuitePlan,
    outcomes: &[SuiteChildOutcome],
    manifests: EvidenceManifests,
    stability: StabilityScore,
) -> Result<SuiteEvidenceBundle, RobustnessError> {
    let succeeded_ids: std::collections::BTreeSet<Uuid> = outcomes
        .iter()
        .filter(|o| o.succeeded)
        .map(|o| o.run_id)
        .collect();
    let have = plan
        .items
        .iter()
        .filter(|item| succeeded_ids.contains(&item.lineage.run_id))
        .count();
    if have != plan.items.len() {
        return Err(RobustnessError::IncompleteSuite {
            expected: plan.items.len(),
            have,
        });
    }
    Ok(SuiteEvidenceBundle {
        parent_run_id: plan.parent_run_id,
        manifests,
        stability,
    })
}
