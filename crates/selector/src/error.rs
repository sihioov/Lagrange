//! Typed selector failures (design §6.6). Every failure mode is one of these
//! variants — the selector never panics on malformed input and never emits a
//! partial portfolio: `select_targets` returns `Err` or a complete
//! `TargetPortfolio`, never both.

/// A typed selector failure. Each variant carries the exact offending id /
/// dataset / date plus the reason; [`SelectorError::code`] gives the stable
/// machine-readable tag.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SelectorError {
    /// The required dataset is `BLOCKED` (Todo 11 quality gate): the
    /// documented fail-closed denial for recommendation use.
    #[error("dataset {dataset_id} is {state}: recommendation blocked (DATA_BLOCKED; {blocking_issues})")]
    DataBlocked {
        dataset_id: String,
        state: String,
        blocking_issues: String,
    },
    /// The factor-snapshot as-of date falls outside the universe effective
    /// window. `until` is pre-formatted ("open" for open-ended windows).
    #[error("as-of {as_of} is outside the universe effective window [{from}, {until})")]
    AsOfOutsideWindow {
        as_of: String,
        from: String,
        until: String,
    },
    /// Stale-state guard: the factor snapshot was frozen over a different
    /// universe than the published snapshot being selected on.
    #[error("factor snapshot universe {snapshot_universe} does not match published universe {published_universe}")]
    UniverseMismatch {
        snapshot_universe: String,
        published_universe: String,
    },
    /// A universe member has no factor snapshot row on the as-of date.
    #[error("universe member {instrument} has no factor snapshot row on {date}")]
    MissingFactorRow { instrument: String, date: String },
    /// A factor snapshot row on the as-of date names an instrument outside
    /// the published universe.
    #[error("factor snapshot row on {date} for {instrument}, which is not a member of the published universe")]
    UnknownSnapshotInstrument { instrument: String, date: String },
    /// The spec references a factor the snapshot does not carry.
    #[error("selection spec references unknown factor {factor} (snapshot carries {known})")]
    UnknownFactor { factor: String, known: String },
    /// The selection spec violates its documented contract.
    #[error("invalid selection spec: {detail}")]
    InvalidSpec { detail: String },
    /// The documented constraints cannot be satisfied simultaneously (e.g.
    /// cash floor + max weight leave no room for the selected targets).
    #[error("impossible constraints: {detail}")]
    ImpossibleConstraints { detail: String },
    /// An invariant that cannot arise from user input.
    #[error("internal selector error: {detail}")]
    Internal { detail: String },
}

impl SelectorError {
    /// The stable machine-readable code of the failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DataBlocked { .. } => "DATA_BLOCKED",
            Self::AsOfOutsideWindow { .. } => "AS_OF_OUTSIDE_WINDOW",
            Self::UniverseMismatch { .. } => "UNIVERSE_MISMATCH",
            Self::MissingFactorRow { .. } => "MISSING_FACTOR_ROW",
            Self::UnknownSnapshotInstrument { .. } => "UNKNOWN_SNAPSHOT_INSTRUMENT",
            Self::UnknownFactor { .. } => "UNKNOWN_FACTOR",
            Self::InvalidSpec { .. } => "SPEC_INVALID",
            Self::ImpossibleConstraints { .. } => "CONSTRAINTS_IMPOSSIBLE",
            Self::Internal { .. } => "INTERNAL",
        }
    }
}
