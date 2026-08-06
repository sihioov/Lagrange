//! Typed failures of the robustness layer (plan Todo 21).
//!
//! Every rejection is a typed [`RobustnessError`]; the layer never panics on
//! malformed input (QA adversarial class: NaN/missing-data/axis-mutation are
//! typed and rejected).

/// A typed failure of a robustness operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RobustnessError {
    /// A derived run changed more than one declared axis (design §9.5:
    /// "모든 파생 실행은 ... 하나의 변수만 변경한다").
    #[error("derived run changes {count} axes; exactly one axis is allowed")]
    MultiAxisChange { count: usize },

    /// A derived run did not pin a parent context field (strategy/data/engine
    /// must be identical to the parent — design §9.5 "부모의 전략·데이터 버전을
    /// 고정").
    #[error("derived run does not pin parent {field}: parent {parent}, derived {derived}")]
    PinMismatch {
        field: &'static str,
        parent: String,
        derived: String,
    },

    /// The final test period was read during parameter selection
    /// (FR-ROB-001: "최종 테스트 구간은 파라미터 선택 단계에서 사용되지 않는다").
    #[error("holdout violation: test-period date {date} was read during selection")]
    HoldoutViolation { date: String },

    /// A series required by the computation was empty.
    #[error("empty series: {what}")]
    EmptySeries { what: String },

    /// A cost-stress profile is out of the documented range.
    #[error("invalid cost profile: {detail}")]
    InvalidCostProfile { detail: String },

    /// Execution delay pushes a fill beyond the session calendar.
    #[error("execution delay out of range: {detail}")]
    DelayOutOfRange { detail: String },

    /// The dataset/backtest is blocked by the missing-data policy (AT-05;
    /// mirrors the queue's `DataBlocked` class: never retried).
    #[error("data blocked: {detail}")]
    DataBlocked { detail: String },

    /// The stability score is a reference indicator and can never approve an
    /// investment (design §9.6: "초기 점수는 참고 지표이며 절대적 승인 기준으로
    /// 사용하지 않는다").
    #[error("stability score is a reference indicator, not an investment approval")]
    StabilityScoreNotApproval,

    /// A non-finite value reached a place that requires finite numbers.
    #[error("non-finite value at {field}")]
    NonFinite { field: String },

    /// The deterministic fill→ledger→equity replay failed an invariant.
    #[error("replay failure: {detail}")]
    Replay { detail: String },

    /// A benchmark series was required but is missing.
    #[error("benchmark series missing or empty for benchmark {benchmark_id}")]
    NoBenchmarkData { benchmark_id: String },

    /// Not enough sessions/points for the requested window or step.
    #[error("insufficient data: {what} (need {need}, have {have})")]
    InsufficientData {
        what: &'static str,
        need: usize,
        have: usize,
    },
}
