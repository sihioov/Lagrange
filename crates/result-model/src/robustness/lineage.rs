//! Parent/derived run lineage (design §9.5, plan Todo 21).
//!
//! A robustness suite is a parent run plus derived runs. Every derived run
//! pins the parent's strategy/data/engine context and changes EXACTLY ONE
//! declared axis (design: "모든 파생 실행은 부모의 전략·데이터 버전을 고정하고
//! 하나의 변수만 변경한다"). [`DerivedAxis`] enumerates the six documented
//! derived-run types: parameter neighborhood, cost stress, period split,
//! walk-forward, execution delay, and benchmark comparison.
//!
//! Lineage is deterministic and idempotent: the derived run id is
//! `uuid5(LINEAGE_NAMESPACE, parent_run_id | axis | pinned)` so the same
//! request always resolves to the same run (FR-BT-008/AT-03 semantics at the
//! lineage level) and re-registration is a no-op.

use std::collections::BTreeMap;

use domain::provenance::{Engine, RunProvenance};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::robustness::RobustnessError;

/// Namespace of the deterministic derived-run ids ("LAGRANGELINEAGE").
pub const LINEAGE_NAMESPACE: Uuid = Uuid::from_u128(0x4c414752_414e4745_4c494e45_41474500);

/// The six documented derived-run axes (design §9.5).
///
/// A derived run changes exactly ONE of these; the variant payload is the
/// declared axis change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "axis", rename_all = "snake_case")]
pub enum DerivedAxis {
    /// Parameter-neighborhood run: one strategy parameter moved by `delta`
    /// (FR-ROB-002).
    ParameterNeighborhood {
        parameter: String,
        delta: serde_json::Value,
    },
    /// Cost-stress run: a versioned cost profile (FR-ROB-003).
    CostStress {
        profile_id: String,
        profile_version: u32,
    },
    /// Train/validation/test split boundaries (FR-ROB-001).
    PeriodSplit {
        train_end: String,
        validation_end: String,
    },
    /// Walk-forward plan: `window_sessions` train, `step_sessions` advance
    /// (FR-ROB-004).
    WalkForward {
        window_sessions: u32,
        step_sessions: u32,
    },
    /// Execution-delay run: fills shifted `delay_sessions` sessions later.
    ExecutionDelay { delay_sessions: u32 },
    /// Benchmark-comparison run: compare against the named benchmark series.
    BenchmarkComparison { benchmark_id: String },
}

impl DerivedAxis {
    /// The stable machine code of the axis (`parameter_neighborhood`,
    /// `cost_stress`, `period_split`, `walk_forward`, `execution_delay`,
    /// `benchmark_comparison`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::ParameterNeighborhood { .. } => "parameter_neighborhood",
            Self::CostStress { .. } => "cost_stress",
            Self::PeriodSplit { .. } => "period_split",
            Self::WalkForward { .. } => "walk_forward",
            Self::ExecutionDelay { .. } => "execution_delay",
            Self::BenchmarkComparison { .. } => "benchmark_comparison",
        }
    }

    /// A canonical serialization of the axis (recursively sorted keys), used
    /// in the deterministic run-id derivation.
    pub fn canonical(&self) -> String {
        let value = serde_json::to_value(self).expect("axis serializes to JSON");
        let sorted = canonical_value(&value);
        serde_json::to_string(&sorted).expect("canonical axis serialization")
    }
}

/// The context every derived run pins from its parent (design §9.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedContext {
    pub strategy_id: String,
    pub strategy_version: String,
    pub dataset_version: String,
    pub engine: String,
    pub engine_version: String,
}

impl PinnedContext {
    /// Extracts the pinned context from a run's provenance.
    pub fn from_provenance(provenance: &RunProvenance) -> Self {
        Self {
            strategy_id: provenance.strategy_id.to_string(),
            strategy_version: provenance.strategy_version.to_string(),
            dataset_version: provenance.dataset_version.to_string(),
            engine: match provenance.engine {
                Engine::NautilusTrader => "nautilustrader",
            }
            .to_owned(),
            engine_version: provenance.engine_version.to_string(),
        }
    }

    /// Canonical serialization used in the deterministic run-id derivation.
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("pinned context serializes")
    }
}

/// One lineage row: the derived run, its parent, and the single changed axis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLineage {
    /// The parent run's id.
    pub parent_run_id: Uuid,
    /// The derived run's deterministic id (`uuid5` over parent|axis|pinned).
    pub run_id: Uuid,
    /// The pinned strategy/data/engine context inherited from the parent.
    pub pinned: PinnedContext,
    /// The single axis this derived run changes.
    pub changed_axis: DerivedAxis,
    /// The derived run's configuration content hash (verbatim provenance).
    pub derived_config_hash: String,
}

/// A derived-run registration request.
#[derive(Debug, Clone)]
pub struct DerivedRunRequest {
    pub parent_run_id: Uuid,
    pub parent: RunProvenance,
    /// The declared axis changes; exactly one is allowed.
    pub changes: Vec<DerivedAxis>,
    /// The actual provenance of the derived run (its pinning is validated).
    pub derived_provenance: RunProvenance,
}

/// Checks that `derived` pins every field of `parent` (design §9.5). Shared
/// by [`LineageRegistry::register_derived`] and suite planning (`suite.rs`)
/// so both reject a mismatch with the same field-precise error.
pub fn check_pin_match(
    parent: &PinnedContext,
    derived: &PinnedContext,
) -> Result<(), RobustnessError> {
    let pin_checks: [(&'static str, &str, &str); 5] = [
        ("strategy_id", &parent.strategy_id, &derived.strategy_id),
        (
            "strategy_version",
            &parent.strategy_version,
            &derived.strategy_version,
        ),
        (
            "dataset_version",
            &parent.dataset_version,
            &derived.dataset_version,
        ),
        ("engine", &parent.engine, &derived.engine),
        (
            "engine_version",
            &parent.engine_version,
            &derived.engine_version,
        ),
    ];
    for (field, parent_value, derived_value) in pin_checks {
        if parent_value != derived_value {
            return Err(RobustnessError::PinMismatch {
                field,
                parent: parent_value.to_owned(),
                derived: derived_value.to_owned(),
            });
        }
    }
    Ok(())
}

/// In-memory lineage registry with deterministic, idempotent registration.
#[derive(Debug, Clone, Default)]
pub struct LineageRegistry {
    runs: BTreeMap<Uuid, RunLineage>,
    children: BTreeMap<Uuid, Vec<Uuid>>,
}

impl LineageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a derived run. Rejects when the run changes more than one
    /// axis ([`RobustnessError::MultiAxisChange`]) or when it does not pin
    /// the parent's strategy/data/engine ([`RobustnessError::PinMismatch`]).
    /// Identical requests return the identical deterministic row.
    pub fn register_derived(
        &mut self,
        request: DerivedRunRequest,
    ) -> Result<RunLineage, RobustnessError> {
        if request.changes.len() != 1 {
            return Err(RobustnessError::MultiAxisChange {
                count: request.changes.len(),
            });
        }
        let axis = request
            .changes
            .into_iter()
            .next()
            .expect("exactly one axis was validated");
        let parent_pinned = PinnedContext::from_provenance(&request.parent);
        let pinned = PinnedContext::from_provenance(&request.derived_provenance);
        check_pin_match(&parent_pinned, &pinned)?;

        let run_id = Uuid::new_v5(
            &LINEAGE_NAMESPACE,
            format!(
                "{}|{}|{}",
                request.parent_run_id,
                axis.canonical(),
                pinned.canonical()
            )
            .as_bytes(),
        );
        let lineage = RunLineage {
            parent_run_id: request.parent_run_id,
            run_id,
            pinned,
            changed_axis: axis,
            derived_config_hash: request.derived_provenance.config_hash.to_string(),
        };

        self.runs.insert(run_id, lineage.clone());
        let siblings = self.children.entry(lineage.parent_run_id).or_default();
        siblings.push(run_id);
        siblings.sort_unstable();
        siblings.dedup();
        Ok(lineage)
    }

    /// The lineage row of a run, if registered.
    pub fn get(&self, run_id: Uuid) -> Option<&RunLineage> {
        self.runs.get(&run_id)
    }

    /// All derived runs of a parent, deterministic (sorted by run id).
    pub fn children_of(&self, parent_run_id: Uuid) -> Vec<&RunLineage> {
        self.children
            .get(&parent_run_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.runs.get(id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Number of registered lineage rows.
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    /// Whether no lineage row is registered.
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// Recursively sorts object keys so the canonical axis serialization is
/// independent of input key order.
fn canonical_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical_value(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_value).collect())
        }
        other => other.clone(),
    }
}
