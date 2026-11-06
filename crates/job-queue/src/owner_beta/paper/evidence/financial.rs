//! Bounded, read-only financial inputs captured by the Paper evidence checker.
//!
//! These values are descriptive data. They carry no permission, process or
//! runtime proof. A consumer must independently hold protected current/runtime
//! evidence, rerun eligibility immediately before economics, and use SQL fences.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    rc::Rc,
};

use crate::owner_beta::{
    daily_input::OwnerBetaDailyRecommendationInputV1, daily_target::OwnerBetaDailyTargetSnapshotV1,
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{TradingDate, UtcTimestamp};
use market_data::daily_price_only::calendar::CalendarRangeProof;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use super::super::action::PaperActionContext;
pub(super) use super::super::calculation::{PaperPriceSnapshotV1, PriceAvailabilityObservation};
use super::super::{
    contract::{PaperEligibilityDecision, PaperEligibilityReport, UntrustedPaperContractV1},
    input::UntrustedPaperSessionReadbackV1,
};
use super::{EvaluationClock, Reason};

/// Private erasure of the actual borrowed context, not an authorization value.
/// The cheap boundary stays separate from the original full IO validation.
pub(super) trait ActionContextGuard {
    fn revalidate(&mut self) -> Result<(), PaperInputActionError>;
    fn checkpoint(&mut self) -> Result<(), PaperInputActionError>;
}

impl ActionContextGuard for &PaperActionContext<'_, '_, '_> {
    fn revalidate(&mut self) -> Result<(), PaperInputActionError> {
        PaperActionContext::revalidate(self).map_err(|_| PaperInputActionError::Runtime)
    }

    fn checkpoint(&mut self) -> Result<(), PaperInputActionError> {
        PaperActionContext::checkpoint_boundary(self).map_err(|_| PaperInputActionError::Runtime)
    }
}

/// Exact financial action selector. These values choose inputs; they carry no
/// authority and are always checked against retained evidence and SQL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaperActionInputRequest {
    LatestSignal,
    Preview {
        signal: TradingDate,
        effective: TradingDate,
        run_id: uuid::Uuid,
    },
    Settle {
        signal: TradingDate,
        effective: TradingDate,
        run_id: uuid::Uuid,
    },
    Value {
        signal: TradingDate,
        effective: TradingDate,
        run_id: uuid::Uuid,
    },
}

impl PaperActionInputRequest {
    pub(super) fn calculation_request(&self) -> PaperCalculationInputRequest {
        match self {
            Self::LatestSignal => PaperCalculationInputRequest::new([], None),
            Self::Preview {
                signal,
                effective,
                run_id,
            } => PaperCalculationInputRequest::for_action(
                [signal.as_naive_date()],
                [],
                Some((*signal, *effective)),
                *signal,
                *run_id,
            ),
            Self::Settle {
                signal,
                effective,
                run_id,
            }
            | Self::Value {
                signal,
                effective,
                run_id,
            } => PaperCalculationInputRequest::for_action(
                [signal.as_naive_date(), effective.as_naive_date()],
                [effective.as_naive_date()],
                Some((*signal, *effective)),
                *signal,
                *run_id,
            ),
        }
    }

    pub(super) fn signal_date(&self) -> Option<TradingDate> {
        match self {
            Self::LatestSignal => None,
            Self::Preview { signal, .. }
            | Self::Settle { signal, .. }
            | Self::Value { signal, .. } => Some(*signal),
        }
    }

    pub(super) fn execution_date(&self) -> Option<TradingDate> {
        match self {
            Self::Settle { effective, .. } | Self::Value { effective, .. } => Some(*effective),
            Self::LatestSignal | Self::Preview { .. } => None,
        }
    }
}

pub type LocalInputFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Local transaction fence for the preview's exact queue claim. Checkpoints
/// retain the configured lease during validation; settlement is deferred until
/// all caller/input checks pass. Futures deliberately have no Send bound.
pub trait PaperActionCommitFence {
    fn checkpoint<'a>(
        &'a mut self,
        transaction: &'a mut Transaction<'static, Postgres>,
    ) -> LocalInputFuture<'a, Result<(), crate::QueueError>>;

    fn settle<'a>(
        &'a mut self,
        transaction: &'a mut Transaction<'static, Postgres>,
    ) -> LocalInputFuture<'a, Result<crate::SettleResult, crate::QueueError>>;

    fn after_settlement<'a>(
        &'a mut self,
        transaction: &'a mut Transaction<'static, Postgres>,
    ) -> LocalInputFuture<'a, Result<(), crate::QueueError>>;
}

/// Local, deliberately non-Send consumer for one retained checked action.
pub trait PaperFinancialConsumer<T: 'static, E: 'static> {
    fn run<'a, 'hold>(
        self,
        action: &'a mut CheckedPaperAction<'hold>,
    ) -> LocalInputFuture<'a, Result<T, E>>
    where
        Self: 'a,
        'hold: 'a;
}

pub(super) trait LocalActionConsumer {
    fn invoke<'a>(
        &'a mut self,
        action: &'a mut CheckedPaperAction<'_>,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>>;
}

pub(super) struct ConsumerResult<'c, T, E> {
    consumer: Option<Box<dyn LocalActionConsumer + 'c>>,
    result: Rc<RefCell<Option<Result<T, E>>>>,
}

impl<'c, T: 'static, E: 'static> ConsumerResult<'c, T, E> {
    pub(super) fn new<C: PaperFinancialConsumer<T, E> + 'c>(consumer: C) -> Self {
        let result = Rc::new(RefCell::new(None));
        let adapter = StoredConsumer {
            consumer: Some(consumer),
            result: Rc::clone(&result),
        };
        Self {
            consumer: Some(Box::new(adapter)),
            result,
        }
    }

    pub(super) fn consumer(&mut self) -> Option<&mut (dyn LocalActionConsumer + 'c)> {
        self.consumer.as_deref_mut()
    }

    pub(super) fn take_result(&self) -> Option<Result<T, E>> {
        self.result.borrow_mut().take()
    }
}

struct StoredConsumer<C, T, E> {
    consumer: Option<C>,
    result: Rc<RefCell<Option<Result<T, E>>>>,
}

impl<C, T: 'static, E: 'static> LocalActionConsumer for StoredConsumer<C, T, E>
where
    C: PaperFinancialConsumer<T, E>,
{
    fn invoke<'a>(
        &'a mut self,
        action: &'a mut CheckedPaperAction<'_>,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
        Box::pin(async move {
            let Some(consumer) = self.consumer.take() else {
                return Err(PaperInputActionError::TransactionState);
            };
            let result = consumer.run(action).await;
            *self.result.borrow_mut() = Some(result);
            Ok(())
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperInputActionError {
    #[error("PAPER_ACTION_INPUT_REFUSED: {0:?}")]
    Input(Reason),
    #[error("PAPER_ACTION_RUNTIME_REFUSED")]
    Runtime,
    #[error("PAPER_ACTION_SQL_FAILED")]
    Sql,
    #[error("PAPER_ACTION_TRANSACTION_STATE")]
    TransactionState,
    #[error("PAPER_ACTION_COMMIT_UNKNOWN")]
    CommitUnknown,
    #[error("PAPER_ACTION_CLEANUP_FAILED")]
    Cleanup,
}

pub(super) trait ActionInputDriver {
    fn revalidate_files(&mut self) -> Result<(), PaperInputActionError>;
    fn retain_value(&mut self, value: Value);
    fn retain_sql_error(&mut self, error: sqlx::Error);
    fn retain_queue_error(&mut self, error: crate::QueueError);
    fn commit_started(&self) -> bool;
    fn begin_transaction<'a>(
        &'a mut self,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>>;
    fn transaction(&mut self)
    -> Result<&mut Transaction<'static, Postgres>, PaperInputActionError>;
    fn revalidate_before_commit<'a>(
        &'a mut self,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>>;
    fn commit<'a>(
        &'a mut self,
        final_check: &'a mut dyn FnMut() -> Result<(), PaperInputActionError>,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>>;
    // An unsupported driver must refuse a requested fence, never fall back to
    // its legacy commit implementation or fabricate an active transaction.
    fn commit_fenced<'a>(
        &'a mut self,
        _final_check: &'a mut dyn FnMut() -> Result<(), PaperInputActionError>,
        _fence: &'a mut dyn PaperActionCommitFence,
    ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
        Box::pin(async { Err(PaperInputActionError::TransactionState) })
    }
    fn rollback<'a>(&'a mut self) -> LocalInputFuture<'a, Result<(), PaperInputActionError>>;
}

pub(super) enum RetainedActionValue {
    SqlValue(
        #[expect(
            dead_code,
            reason = "Whole returned values remain owned, without public payload access or formatting."
        )]
        Value,
    ),
    SqlError(
        #[expect(
            dead_code,
            reason = "Whole raw errors remain owned, without public payload access or formatting."
        )]
        sqlx::Error,
    ),
    QueueError(
        #[expect(
            dead_code,
            reason = "Whole queue errors stay owned until final outcome drop."
        )]
        crate::QueueError,
    ),
    QueueSettlement(
        #[cfg_attr(
            not(test),
            expect(
                dead_code,
                reason = "The actual settlement result/error remains private and whole."
            )
        )]
        Box<Result<crate::SettleResult, crate::QueueError>>,
    ),
    // Native lower evidence has no public extraction, serialization or capability API.
    #[cfg(target_os = "linux")]
    LowerEvidence {
        _outcome: Box<super::checker_custody::Outcome>,
    },
}

pub(super) fn retain_settlement(
    retained: &mut Vec<RetainedActionValue>,
    result: Result<crate::SettleResult, crate::QueueError>,
) -> Result<(), PaperInputActionError> {
    let committed = matches!(&result, Ok(crate::SettleResult::Committed(_)));
    retained.push(RetainedActionValue::QueueSettlement(Box::new(result)));
    if committed {
        Ok(())
    } else {
        Err(PaperInputActionError::Runtime)
    }
}

/// Read-only view into the observations whose real owners remain held by the
/// surrounding checked action scope.
///
/// A holder cannot be reconstructed from caller data:
///
/// ```compile_fail
/// use job_queue::owner_beta::paper::evidence::financial::PaperActionInputs;
/// let _ = PaperActionInputs { current_binding: &serde_json::Value::Null,
///     signal: todo!(), execution: None, calendar_range_proof: None };
/// ```
///
/// Nor can it be cloned or serialized into a capability:
///
/// ```compile_fail
/// use job_queue::owner_beta::paper::evidence::financial::PaperActionInputs;
/// fn clone_bound<T: Clone>() {}
/// clone_bound::<PaperActionInputs<'static>>();
/// ```
///
/// ```compile_fail
/// use job_queue::owner_beta::paper::evidence::financial::PaperActionInputs;
/// fn serialize_bound<T: serde::Serialize>() {}
/// serialize_bound::<PaperActionInputs<'static>>();
/// ```
pub struct PaperActionInputs<'a> {
    current_binding: &'a Value,
    signal: PaperSignalObservation<'a>,
    execution: Option<PaperAdmittedPriceObservation<'a>>,
    calendar_range_proof: Option<&'a CalendarRangeProof>,
    native_enable_sessions: Vec<Value>,
    evaluated_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl PaperActionInputs<'_> {
    pub fn current_binding(&self) -> &Value {
        self.current_binding
    }

    pub fn signal(&self) -> &PaperSignalObservation<'_> {
        &self.signal
    }

    pub fn execution(&self) -> Option<&PaperAdmittedPriceObservation<'_>> {
        self.execution.as_ref()
    }

    pub fn calendar_range_proof(&self) -> Option<&CalendarRangeProof> {
        self.calendar_range_proof
    }

    /// Crate-private joined SQL/source facts for the consuming native ENABLE
    /// callback. They are available only while CheckedPaperAction is borrowed.
    pub(crate) fn native_enable_sessions(&self) -> &[Value] {
        &self.native_enable_sessions
    }

    pub(crate) fn evaluated_at(&self) -> DateTime<Utc> {
        self.evaluated_at
    }

    pub(crate) fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl<'a> PaperActionInputs<'a> {
    pub(super) fn new(
        current_binding: &'a Value,
        signal: PaperSignalObservation<'a>,
        execution: Option<PaperAdmittedPriceObservation<'a>>,
        calendar_range_proof: Option<&'a CalendarRangeProof>,
        native_enable_sessions: Vec<Value>,
        evaluated_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            current_binding,
            signal,
            execution,
            calendar_range_proof,
            native_enable_sessions,
            evaluated_at,
            expires_at,
        }
    }
}

/// Signal observation is recommendation-backed and has a checked T run.
pub struct PaperSignalObservation<'a> {
    snapshot: &'a PaperPriceSnapshotV1,
    readback: &'a UntrustedPaperSessionReadbackV1,
    prices: PaperAdmittedPriceObservation<'a>,
    availability: PriceAvailabilityObservation,
    observed_at: DateTime<Utc>,
    input: &'a OwnerBetaDailyRecommendationInputV1,
    target: &'a OwnerBetaDailyTargetSnapshotV1,
}

impl PaperSignalObservation<'_> {
    pub fn prices(&self) -> &PaperAdmittedPriceObservation<'_> {
        &self.prices
    }

    pub fn input(&self) -> &OwnerBetaDailyRecommendationInputV1 {
        self.input
    }

    pub fn target(&self) -> &OwnerBetaDailyTargetSnapshotV1 {
        self.target
    }

    pub fn snapshot(&self) -> &PaperPriceSnapshotV1 {
        self.snapshot
    }

    pub fn readback(&self) -> &UntrustedPaperSessionReadbackV1 {
        self.readback
    }

    pub fn availability(&self) -> PriceAvailabilityObservation {
        self.availability
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}

impl<'a> PaperSignalObservation<'a> {
    pub(super) fn new(
        snapshot: &'a PaperPriceSnapshotV1,
        readback: &'a UntrustedPaperSessionReadbackV1,
        prices: PaperAdmittedPriceObservation<'a>,
        availability: PriceAvailabilityObservation,
        observed_at: DateTime<Utc>,
        input: &'a OwnerBetaDailyRecommendationInputV1,
        target: &'a OwnerBetaDailyTargetSnapshotV1,
    ) -> Self {
        Self {
            snapshot,
            readback,
            prices,
            availability,
            observed_at,
            input,
            target,
        }
    }
}

/// Execution observation carries only the checked admitted Daily price chain.
pub struct PaperAdmittedPriceObservation<'a> {
    snapshot: &'a PaperPriceSnapshotV1,
    readback: &'a UntrustedPaperSessionReadbackV1,
    availability: PriceAvailabilityObservation,
    observed_at: DateTime<Utc>,
}

impl PaperAdmittedPriceObservation<'_> {
    pub fn snapshot(&self) -> &PaperPriceSnapshotV1 {
        self.snapshot
    }

    pub fn readback(&self) -> &UntrustedPaperSessionReadbackV1 {
        self.readback
    }

    pub fn availability(&self) -> PriceAvailabilityObservation {
        self.availability
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}

impl<'a> PaperAdmittedPriceObservation<'a> {
    pub(super) fn new(
        snapshot: &'a PaperPriceSnapshotV1,
        readback: &'a UntrustedPaperSessionReadbackV1,
        availability: PriceAvailabilityObservation,
        observed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            snapshot,
            readback,
            availability,
            observed_at,
        }
    }
}

/// Action inputs are selected only after the actual checked capture succeeds.
/// The callback cannot return a borrow from the checked holder:
///
/// ```compile_fail
/// use job_queue::owner_beta::paper::evidence::financial::{
///     CheckedPaperAction, PaperActionInputs,
/// };
/// fn escape(action: &CheckedPaperAction<'_>) -> &'static PaperActionInputs<'static> {
///     action.inputs()
/// }
/// ```
///
/// The local consumer future must stay in the existing local executor:
///
/// ```compile_fail
/// use job_queue::owner_beta::paper::evidence::financial::CheckedPaperAction;
/// fn spawn(action: &'static mut CheckedPaperAction<'static>) {
///     tokio::spawn(async move {
///         action.begin_transaction().await
///     });
/// }
/// ```
pub struct CheckedPaperAction<'hold> {
    inputs: &'hold PaperActionInputs<'hold>,
    driver: &'hold mut dyn ActionInputDriver,
}

impl<'hold> CheckedPaperAction<'hold> {
    pub(super) fn new(
        inputs: &'hold PaperActionInputs<'hold>,
        driver: &'hold mut dyn ActionInputDriver,
    ) -> Self {
        CheckedPaperAction { inputs, driver }
    }

    pub fn inputs(&self) -> &PaperActionInputs<'_> {
        self.inputs
    }

    pub fn revalidate_files(&mut self) -> Result<(), PaperInputActionError> {
        self.driver.revalidate_files()
    }

    /// Retain the whole lower SQL response before decoding it.
    pub fn retain_value(&mut self, value: Value) {
        self.driver.retain_value(value)
    }

    /// Retain the actual SQL error before mapping it to the payload-free API code.
    pub fn retain_sql_error(&mut self, error: sqlx::Error) {
        self.driver.retain_sql_error(error)
    }

    /// Keep the actual queue failure before mapping it to a closed API code.
    pub fn retain_queue_error(&mut self, error: crate::QueueError) {
        self.driver.retain_queue_error(error)
    }

    pub fn commit_started(&self) -> bool {
        self.driver.commit_started()
    }

    pub async fn begin_transaction(&mut self) -> Result<(), PaperInputActionError> {
        self.driver.begin_transaction().await
    }

    pub fn transaction(
        &mut self,
    ) -> Result<&mut Transaction<'static, Postgres>, PaperInputActionError> {
        self.driver.transaction()
    }

    pub async fn revalidate_before_commit(&mut self) -> Result<(), PaperInputActionError> {
        self.driver.revalidate_before_commit().await
    }

    pub async fn commit(
        &mut self,
        final_check: &mut dyn FnMut() -> Result<(), PaperInputActionError>,
    ) -> Result<(), PaperInputActionError> {
        self.driver.commit(final_check).await
    }

    pub async fn commit_fenced(
        &mut self,
        final_check: &mut dyn FnMut() -> Result<(), PaperInputActionError>,
        fence: &mut dyn PaperActionCommitFence,
    ) -> Result<(), PaperInputActionError> {
        self.driver.commit_fenced(final_check, fence).await
    }

    pub async fn rollback(&mut self) -> Result<(), PaperInputActionError> {
        self.driver.rollback().await
    }
}

pub struct CheckedPaperActionOutcome<T, E> {
    original: Option<Result<T, E>>,
    first: Option<PaperInputActionError>,
    secondary: Vec<PaperInputActionError>,
    committed: bool,
    commit_started: bool,
    retained: Vec<RetainedActionValue>,
}

impl<T, E> CheckedPaperActionOutcome<T, E> {
    pub(super) fn new(
        original: Option<Result<T, E>>,
        first: Option<PaperInputActionError>,
        secondary: Vec<PaperInputActionError>,
        committed: bool,
        commit_started: bool,
        retained: Vec<RetainedActionValue>,
    ) -> Self {
        Self {
            original,
            first,
            secondary,
            committed,
            commit_started,
            retained,
        }
    }

    pub fn completion_count(&self) -> usize {
        self.secondary.len() + self.retained.len()
    }

    pub fn committed(&self) -> bool {
        self.committed
    }

    pub fn commit_started(&self) -> bool {
        self.commit_started
    }

    pub fn into_result(self) -> Result<T, CheckedPaperActionFailure<T, E>> {
        match (self.first, self.original) {
            (None, Some(Ok(value))) if self.secondary.is_empty() => Ok(value),
            (first, original) => Err(CheckedPaperActionFailure {
                original,
                first,
                secondary: self.secondary,
                committed: self.committed,
                commit_started: self.commit_started,
                retained: self.retained,
            }),
        }
    }
}

pub struct CheckedPaperActionFailure<T, E> {
    original: Option<Result<T, E>>,
    first: Option<PaperInputActionError>,
    secondary: Vec<PaperInputActionError>,
    committed: bool,
    commit_started: bool,
    pub(super) retained: Vec<RetainedActionValue>,
}

impl<T, E> CheckedPaperActionFailure<T, E> {
    pub fn original(&self) -> Option<&Result<T, E>> {
        self.original.as_ref()
    }

    pub fn first_error(&self) -> Option<PaperInputActionError> {
        self.first
    }

    pub fn secondary_error_count(&self) -> usize {
        self.secondary.len()
    }

    pub fn committed(&self) -> bool {
        self.committed
    }

    pub fn commit_started(&self) -> bool {
        self.commit_started
    }

    pub fn into_original(self) -> Option<Result<T, E>> {
        self.original
    }

    pub fn retained_outcome_count(&self) -> usize {
        self.retained.len()
    }
}

/// Requested additional price dates and an optional witnessed calendar range.
/// Repeated dates are deduplicated when the request is constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperCalculationInputRequest {
    price_dates: BTreeSet<NaiveDate>,
    price_only_dates: BTreeSet<NaiveDate>,
    calendar_range: Option<(TradingDate, TradingDate)>,
    signal_date: Option<NaiveDate>,
    signal_run_id: Option<uuid::Uuid>,
}

impl PaperCalculationInputRequest {
    pub fn new(
        price_dates: impl IntoIterator<Item = NaiveDate>,
        calendar_range: Option<(TradingDate, TradingDate)>,
    ) -> Self {
        Self {
            price_dates: price_dates.into_iter().collect(),
            price_only_dates: BTreeSet::new(),
            calendar_range,
            signal_date: None,
            signal_run_id: None,
        }
    }

    /// Requests admitted price evidence without requiring a recommendation
    /// run for those dates. Used only for persisted execution-session prices.
    pub fn with_price_only_dates(
        price_dates: impl IntoIterator<Item = NaiveDate>,
        price_only_dates: impl IntoIterator<Item = NaiveDate>,
        calendar_range: Option<(TradingDate, TradingDate)>,
    ) -> Self {
        Self {
            price_dates: price_dates.into_iter().collect(),
            price_only_dates: price_only_dates.into_iter().collect(),
            calendar_range,
            signal_date: None,
            signal_run_id: None,
        }
    }

    fn for_action(
        price_dates: impl IntoIterator<Item = NaiveDate>,
        price_only_dates: impl IntoIterator<Item = NaiveDate>,
        calendar_range: Option<(TradingDate, TradingDate)>,
        signal_date: TradingDate,
        signal_run_id: uuid::Uuid,
    ) -> Self {
        Self {
            price_dates: price_dates.into_iter().collect(),
            price_only_dates: price_only_dates.into_iter().collect(),
            calendar_range,
            signal_date: Some(signal_date.as_naive_date()),
            signal_run_id: Some(signal_run_id),
        }
    }

    pub(super) fn expected_run_id(&self, date: NaiveDate) -> Option<uuid::Uuid> {
        if self.signal_date == Some(date) {
            self.signal_run_id
        } else {
            None
        }
    }

    pub fn price_dates(&self) -> impl ExactSizeIterator<Item = NaiveDate> + '_ {
        self.price_dates.iter().copied()
    }

    pub fn calendar_range(&self) -> Option<(TradingDate, TradingDate)> {
        self.calendar_range
    }

    pub fn requires_recommendation(&self, date: NaiveDate) -> bool {
        self.price_dates.contains(&date) && !self.price_only_dates.contains(&date)
    }

    pub fn is_price_only(&self, date: NaiveDate) -> bool {
        self.price_only_dates.contains(&date)
    }

    fn validate(&self, now: DateTime<Utc>) -> Result<(), PaperCalculationInputError> {
        if self.price_dates.len() > 2
            || !self.price_only_dates.is_subset(&self.price_dates)
            || self.signal_date.is_some() != self.signal_run_id.is_some()
            || self
                .signal_date
                .is_some_and(|date| !self.price_dates.contains(&date))
            || self.signal_run_id.is_some_and(|run_id| run_id.is_nil())
        {
            return Err(PaperCalculationInputError::InvalidRequest);
        }
        let eligible = market_data::daily_price_only::calendar::eligible_calendar_date(
            UtcTimestamp::from_datetime(now),
        )
        .map_err(|_| PaperCalculationInputError::InvalidRequest)?
        .as_naive_date();
        if self.price_dates.iter().any(|date| *date > eligible) {
            return Err(PaperCalculationInputError::FuturePriceDate);
        }
        if let Some((start, end)) = self.calendar_range {
            let span = end
                .as_naive_date()
                .signed_duration_since(start.as_naive_date())
                .num_days();
            if start >= end || span < 1 || span as u64 > super::MAX_CALENDAR_DAYS {
                return Err(PaperCalculationInputError::InvalidRequest);
            }
        }
        Ok(())
    }
}

/// Payload-free result errors. Eligibility failures retain the checker's
/// stable reason code; no SQL/provider details cross this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaperCalculationInputError {
    #[error("PAPER_CALCULATION_INPUT_INVALID_REQUEST")]
    InvalidRequest,
    #[error("PAPER_CALCULATION_INPUT_FUTURE_PRICE_DATE")]
    FuturePriceDate,
    #[error("PAPER_CALCULATION_INPUT_ELIGIBILITY_REJECTED: {0:?}")]
    Eligibility(super::super::contract::PaperEligibilityReasonCode),
    #[error("PAPER_CALCULATION_INPUT_INELIGIBLE")]
    Ineligible,
    #[error("PAPER_CALCULATION_INPUT_CAPTURE_INCOMPLETE")]
    CaptureIncomplete,
}

/// Actual checked values retained after the complete checker has published an
/// eligible result and every owned resource has finished cleanly.
pub struct ObservedPaperCalculationInputs {
    report: PaperEligibilityReport,
    current_binding: Value,
    sessions: BTreeMap<NaiveDate, PaperSessionObservation>,
    calendar_range_proof: Option<CalendarRangeProof>,
}

impl std::fmt::Debug for ObservedPaperCalculationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedPaperCalculationInputs")
            .field("report", &self.report)
            .field("session_count", &self.sessions.len())
            .field(
                "calendar_range_proof_present",
                &self.calendar_range_proof.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl ObservedPaperCalculationInputs {
    pub fn report(&self) -> &PaperEligibilityReport {
        &self.report
    }

    /// Clone of the selector from the actual protected CURRENT snapshot that
    /// was checked against the caller's untrusted selector.
    pub fn current_binding(&self) -> &Value {
        &self.current_binding
    }

    pub fn sessions(&self) -> &BTreeMap<NaiveDate, PaperSessionObservation> {
        &self.sessions
    }

    pub fn calendar_range_proof(&self) -> Option<&CalendarRangeProof> {
        self.calendar_range_proof.as_ref()
    }
}

/// Checked session data extracted at the genuine checked sealed-view point.
pub struct PaperSessionObservation {
    snapshot: PaperPriceSnapshotV1,
    readback: UntrustedPaperSessionReadbackV1,
    observed_at: DateTime<Utc>,
    availability: PriceAvailabilityObservation,
    input: Option<OwnerBetaDailyRecommendationInputV1>,
    target: Option<OwnerBetaDailyTargetSnapshotV1>,
}

impl std::fmt::Debug for PaperSessionObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperSessionObservation")
            .field("as_of", &self.snapshot.as_of())
            .field("observed_at", &self.observed_at)
            .finish_non_exhaustive()
    }
}

impl PaperSessionObservation {
    pub fn snapshot(&self) -> &PaperPriceSnapshotV1 {
        &self.snapshot
    }

    pub fn readback(&self) -> &UntrustedPaperSessionReadbackV1 {
        &self.readback
    }

    /// Safe clone of the complete validated untrusted SQL value.
    pub fn readback_value(&self) -> Value {
        self.readback.as_untrusted_value().clone()
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }

    /// Original Raw retrieval and SQL admission times, with actual local observation.
    pub fn availability(&self) -> PriceAvailabilityObservation {
        self.availability
    }

    pub fn input(&self) -> &OwnerBetaDailyRecommendationInputV1 {
        self.input
            .as_ref()
            .expect("recommendation-backed T session")
    }

    /// Recomputed from the genuine checked input and matched to persisted F/W.
    pub fn target(&self) -> &OwnerBetaDailyTargetSnapshotV1 {
        self.target
            .as_ref()
            .expect("recommendation-backed T session")
    }

    pub fn recommendation_input(&self) -> Option<&OwnerBetaDailyRecommendationInputV1> {
        self.input.as_ref()
    }

    pub fn recommendation_target(&self) -> Option<&OwnerBetaDailyTargetSnapshotV1> {
        self.target.as_ref()
    }
}

/// Opens the existing checker path with bounded data capture. Invalid requests
/// are rejected before opening protected files or starting a database session.
pub async fn read_paper_calculation_inputs(
    pool: &PgPool,
    selector: &UntrustedPaperContractV1,
    request: &PaperCalculationInputRequest,
) -> Result<ObservedPaperCalculationInputs, PaperCalculationInputError> {
    request.validate(Utc::now())?;
    let mut clock = EvaluationClock::start().map_err(PaperCalculationInputError::Eligibility)?;
    super::read_paper_calculation_inputs_at(pool, selector, request, &mut clock).await
}

/// Capture one exact signal/execution input set and run its economic consumer
/// before any retained evidence holder is retired.
pub async fn with_checked_paper_action<T, E, C>(
    pool: &PgPool,
    context: &PaperActionContext<'_, '_, '_>,
    request: &PaperActionInputRequest,
    consumer: C,
) -> CheckedPaperActionOutcome<T, E>
where
    T: 'static,
    E: 'static,
    C: PaperFinancialConsumer<T, E>,
{
    let request_inputs = request.calculation_request();
    if request_inputs.validate(Utc::now()).is_err() {
        return CheckedPaperActionOutcome::new(
            None,
            Some(PaperInputActionError::Runtime),
            Vec::new(),
            false,
            false,
            Vec::new(),
        );
    }
    let binding = match serde_json::to_vec(context.binding())
        .ok()
        .and_then(|bytes| UntrustedPaperContractV1::decode(&bytes).ok())
    {
        Some(binding) => binding,
        None => {
            return CheckedPaperActionOutcome::new(
                None,
                Some(PaperInputActionError::Runtime),
                Vec::new(),
                false,
                false,
                Vec::new(),
            );
        }
    };
    let mut result = ConsumerResult::new(consumer);
    let mut outer_guard = context;
    let outcome = match result.consumer() {
        Some(consumer) => {
            super::read_checked_paper_action_inputs_at(
                pool,
                &binding,
                &request_inputs,
                request,
                &mut outer_guard,
                consumer,
            )
            .await
        }
        None => super::ActionCapturePublication {
            first: Some(PaperInputActionError::TransactionState),
            secondary: Vec::new(),
            committed: false,
            commit_started: false,
            retained: Vec::new(),
        },
    };
    CheckedPaperActionOutcome::new(
        result.take_result(),
        outcome.first,
        outcome.secondary,
        outcome.committed,
        outcome.commit_started,
        outcome.retained,
    )
}

pub(super) struct CapturedSession {
    pub(super) date: NaiveDate,
    pub(super) snapshot: PaperPriceSnapshotV1,
    pub(super) readback: UntrustedPaperSessionReadbackV1,
    pub(super) observed_at: DateTime<Utc>,
    pub(super) availability: PriceAvailabilityObservation,
    pub(super) input: Option<OwnerBetaDailyRecommendationInputV1>,
    pub(super) target: Option<OwnerBetaDailyTargetSnapshotV1>,
}

pub(super) fn publish_observed(
    report: PaperEligibilityReport,
    current_binding: Value,
    captures: Vec<CapturedSession>,
    calendar_range_proof: Option<CalendarRangeProof>,
    requested_dates: &BTreeSet<NaiveDate>,
    expects_calendar_proof: bool,
) -> Result<ObservedPaperCalculationInputs, PaperCalculationInputError> {
    validate_report_gate(&report)?;
    if expects_calendar_proof != calendar_range_proof.is_some() {
        return Err(PaperCalculationInputError::CaptureIncomplete);
    }
    let mut sessions = BTreeMap::new();
    for capture in captures {
        if sessions
            .insert(
                capture.date,
                PaperSessionObservation {
                    snapshot: capture.snapshot,
                    readback: capture.readback,
                    observed_at: capture.observed_at,
                    availability: capture.availability,
                    input: capture.input,
                    target: capture.target,
                },
            )
            .is_some()
        {
            return Err(PaperCalculationInputError::CaptureIncomplete);
        }
    }
    let mut required_dates = requested_dates.clone();
    for date in report.sessions() {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| PaperCalculationInputError::CaptureIncomplete)?;
        required_dates.insert(date);
    }
    if !capture_coverage_complete(&required_dates, sessions.keys().copied()) {
        return Err(PaperCalculationInputError::CaptureIncomplete);
    }
    Ok(ObservedPaperCalculationInputs {
        report,
        current_binding,
        sessions,
        calendar_range_proof,
    })
}

fn validate_report_gate(report: &PaperEligibilityReport) -> Result<(), PaperCalculationInputError> {
    if report.decision() != PaperEligibilityDecision::Eligible {
        return Err(PaperCalculationInputError::Ineligible);
    }
    if report.expires_at() <= Utc::now() {
        return Err(PaperCalculationInputError::Eligibility(
            Reason::EligibilityExpired,
        ));
    }
    Ok(())
}

fn capture_coverage_complete(
    required: &BTreeSet<NaiveDate>,
    captured: impl IntoIterator<Item = NaiveDate>,
) -> bool {
    let captured = captured.into_iter().collect::<BTreeSet<_>>();
    required == &captured && (3..=5).contains(&captured.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::TradingDate;

    struct UnsupportedFenceDriver {
        legacy_commit_called: bool,
    }

    impl ActionInputDriver for UnsupportedFenceDriver {
        fn revalidate_files(&mut self) -> Result<(), PaperInputActionError> {
            panic!("unexpected files")
        }
        fn retain_value(&mut self, _: Value) {
            panic!("unexpected value")
        }
        fn retain_sql_error(&mut self, _: sqlx::Error) {
            panic!("unexpected SQL error")
        }
        fn retain_queue_error(&mut self, _: crate::QueueError) {
            panic!("unexpected queue error")
        }
        fn commit_started(&self) -> bool {
            false
        }
        fn begin_transaction<'a>(
            &'a mut self,
        ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
            panic!("unexpected begin")
        }
        fn transaction(
            &mut self,
        ) -> Result<&mut Transaction<'static, Postgres>, PaperInputActionError> {
            panic!("no fabricated transaction")
        }
        fn revalidate_before_commit<'a>(
            &'a mut self,
        ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
            panic!("unexpected revalidation")
        }
        fn commit<'a>(
            &'a mut self,
            _: &'a mut dyn FnMut() -> Result<(), PaperInputActionError>,
        ) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
            self.legacy_commit_called = true;
            Box::pin(async { Ok(()) })
        }
        fn rollback<'a>(&'a mut self) -> LocalInputFuture<'a, Result<(), PaperInputActionError>> {
            panic!("unexpected rollback")
        }
    }

    struct NeverFence;
    impl PaperActionCommitFence for NeverFence {
        fn checkpoint<'a>(
            &'a mut self,
            _: &'a mut Transaction<'static, Postgres>,
        ) -> LocalInputFuture<'a, Result<(), crate::QueueError>> {
            panic!("unsupported driver called fence")
        }
        fn settle<'a>(
            &'a mut self,
            _: &'a mut Transaction<'static, Postgres>,
        ) -> LocalInputFuture<'a, Result<crate::SettleResult, crate::QueueError>> {
            panic!("unsupported driver settled")
        }
        fn after_settlement<'a>(
            &'a mut self,
            _: &'a mut Transaction<'static, Postgres>,
        ) -> LocalInputFuture<'a, Result<(), crate::QueueError>> {
            panic!("unsupported driver sealed")
        }
    }

    #[tokio::test]
    async fn unsupported_fenced_driver_refuses_without_legacy_commit_or_callbacks() {
        // This calls the actual default, without constructing checked inputs
        // or a pretend SQL transaction.
        let mut driver = UnsupportedFenceDriver {
            legacy_commit_called: false,
        };
        let mut final_called = false;
        let mut final_check = || {
            final_called = true;
            Ok(())
        };
        assert_eq!(
            driver
                .commit_fenced(&mut final_check, &mut NeverFence)
                .await,
            Err(PaperInputActionError::TransactionState)
        );
        assert!(!driver.legacy_commit_called && !final_called && !driver.commit_started());
    }

    #[derive(Debug)]
    struct QueueErrorBody {
        identity: Box<str>,
        drops: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl std::fmt::Display for QueueErrorBody {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("ordinary queue sentinel")
        }
    }
    impl std::error::Error for QueueErrorBody {}
    impl Drop for QueueErrorBody {
        fn drop(&mut self) {
            self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn final_fence_keeps_whole_queue_error_and_original_failure_until_drop() {
        let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let identity = Box::<str>::from("whole lower queue error");
        let address = identity.as_ptr();
        let lower =
            crate::QueueError::Database(sqlx::Error::Io(std::io::Error::other(QueueErrorBody {
                identity,
                drops: drops.clone(),
            })));
        let mut retained = Vec::new();
        assert_eq!(
            retain_settlement(&mut retained, Err(lower)),
            Err(PaperInputActionError::Runtime)
        );
        let original = Box::<str>::from("original consumer refusal");
        let original_address = original.as_ptr();
        let failure = match CheckedPaperActionOutcome::new(
            Some(Err::<(), _>(original)),
            Some(PaperInputActionError::Runtime),
            vec![PaperInputActionError::Cleanup],
            false,
            false,
            retained,
        )
        .into_result()
        {
            Err(failure) => failure,
            Ok(()) => panic!("queue failure cannot become success"),
        };
        let RetainedActionValue::QueueSettlement(result) = &failure.retained[0] else {
            panic!("whole settlement result missing")
        };
        let Err(crate::QueueError::Database(sqlx::Error::Io(error))) = result.as_ref() else {
            panic!("actual typed error missing")
        };
        let body = error
            .get_ref()
            .unwrap()
            .downcast_ref::<QueueErrorBody>()
            .unwrap();
        assert_eq!(body.identity.as_ptr(), address);
        assert_eq!(
            failure.original().unwrap().as_ref().unwrap_err().as_ptr(),
            original_address
        );
        assert_eq!(failure.secondary_error_count(), 1);
        assert!(!failure.commit_started() && !failure.committed());
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(failure);
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn final_fence_refuses_actual_canceled_settlement_variant_without_erasing_it() {
        let job: crate::Job = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::from_u128(1), "owner_user_id": uuid::Uuid::from_u128(2),
            "job_type": "backtest", "status": "CANCELED", "priority": 0, "idempotency_key": null,
            "payload_json": {"sentinel": "whole canceled job"}, "max_attempts": 1, "attempt_count": 1,
            "available_at": "2026-11-05T00:00:00Z", "locked_by": "worker", "locked_at": "2026-11-05T00:00:00Z",
            "started_at": null, "finished_at": null, "error_code": null, "error_message": null,
            "created_at": "2026-11-05T00:00:00Z", "updated_at": "2026-11-05T00:00:00Z"
        })).unwrap();
        let mut retained = Vec::new();
        assert_eq!(
            retain_settlement(
                &mut retained,
                Ok(crate::SettleResult::Canceled(job.clone()))
            ),
            Err(PaperInputActionError::Runtime)
        );
        let RetainedActionValue::QueueSettlement(result) = &retained[0] else {
            panic!("result erased")
        };
        assert!(
            matches!(result.as_ref(), Ok(crate::SettleResult::Canceled(actual)) if actual == &job)
        );
    }

    fn td(value: &str) -> TradingDate {
        TradingDate::parse(value).expect("test date")
    }

    fn prior_dates() -> [NaiveDate; 3] {
        let eligible = market_data::daily_price_only::calendar::eligible_calendar_date(
            UtcTimestamp::from_datetime(Utc::now()),
        )
        .expect("current eligible date")
        .as_naive_date();
        [
            eligible - chrono::Days::new(3),
            eligible - chrono::Days::new(2),
            eligible - chrono::Days::new(1),
        ]
    }

    #[test]
    fn requests_deduplicate_before_the_two_date_bound() {
        let [one, two, three] = prior_dates();
        let request = PaperCalculationInputRequest::new([one, one, two], None);
        assert_eq!(request.price_dates.len(), 2);
        assert!(request.validate(Utc::now()).is_ok());

        let too_many = PaperCalculationInputRequest::new([one, two, three], None);
        assert_eq!(
            too_many.validate(Utc::now()),
            Err(PaperCalculationInputError::InvalidRequest)
        );
    }

    #[test]
    fn execution_price_only_request_keeps_t_recommendation_and_limits_e_to_admission() {
        let [signal, effective, _] = prior_dates();
        let request = PaperCalculationInputRequest::with_price_only_dates(
            [signal, effective],
            [effective],
            Some((
                TradingDate::parse(&signal.to_string()).expect("signal trading date"),
                TradingDate::parse(&effective.to_string()).expect("effective trading date"),
            )),
        );
        assert!(request.validate(Utc::now()).is_ok());
        assert!(request.requires_recommendation(signal));
        assert!(!request.requires_recommendation(effective));
        assert!(request.is_price_only(effective));

        let invalid =
            PaperCalculationInputRequest::with_price_only_dates([signal], [effective], None);
        assert_eq!(
            invalid.validate(Utc::now()),
            Err(PaperCalculationInputError::InvalidRequest)
        );
    }

    #[test]
    fn exact_action_selectors_pin_t_run_and_separate_execution_prices() {
        let signal = td("2026-11-04");
        let effective = td("2026-11-05");
        let run_id = uuid::Uuid::from_u128(23);
        let request = PaperActionInputRequest::Settle {
            signal,
            effective,
            run_id,
        }
        .calculation_request();
        assert_eq!(request.price_dates.len(), 2);
        assert!(
            request
                .price_only_dates
                .contains(&effective.as_naive_date())
        );
        assert!(!request.price_only_dates.contains(&signal.as_naive_date()));
        assert_eq!(
            request.expected_run_id(signal.as_naive_date()),
            Some(run_id)
        );
        assert_eq!(request.expected_run_id(effective.as_naive_date()), None);

        let preview = PaperActionInputRequest::Preview {
            signal,
            effective,
            run_id,
        }
        .calculation_request();
        assert_eq!(preview.price_dates.len(), 1);
        assert!(preview.price_dates.contains(&signal.as_naive_date()));
        assert!(preview.price_only_dates.is_empty());
        assert_eq!(
            preview.expected_run_id(signal.as_naive_date()),
            Some(run_id)
        );
        assert_eq!(preview.calendar_range(), Some((signal, effective)));
    }

    #[test]
    fn request_rejects_future_prices_and_invalid_calendar_ranges() {
        let future = Utc::now().date_naive() + chrono::Days::new(5);
        let request = PaperCalculationInputRequest::new([future], None);
        assert_eq!(
            request.validate(Utc::now()),
            Err(PaperCalculationInputError::FuturePriceDate)
        );
        let start = td("2026-11-04");
        let reversed = PaperCalculationInputRequest::new([], Some((start, start)));
        assert_eq!(
            reversed.validate(Utc::now()),
            Err(PaperCalculationInputError::InvalidRequest)
        );
    }

    #[test]
    fn capture_coverage_does_not_confuse_extra_dates_with_latest_three() {
        let [first, second, third] = prior_dates();
        let latest_three = [first, second, third].into_iter().collect::<BTreeSet<_>>();
        let extra = [first - chrono::Days::new(2)]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let captures = latest_three.union(&extra).copied().collect::<BTreeSet<_>>();
        let required = latest_three.union(&extra).copied().collect::<BTreeSet<_>>();
        assert!(capture_coverage_complete(
            &required,
            captures.iter().copied()
        ));
        assert!(!capture_coverage_complete(
            &required,
            latest_three.iter().copied()
        ));
        assert_eq!(
            latest_three.len(),
            3,
            "extras never replace required sessions"
        );
    }

    #[test]
    fn ineligible_and_expired_reports_cannot_publish_capture() {
        let now = Utc::now();
        let ineligible = PaperEligibilityReport::new(
            PaperEligibilityDecision::Ineligible,
            vec![Reason::CalendarUnproven],
            vec![],
            now,
            now + chrono::Duration::seconds(20),
        );
        assert_eq!(
            validate_report_gate(&ineligible),
            Err(PaperCalculationInputError::Ineligible)
        );

        let expired = PaperEligibilityReport::new(
            PaperEligibilityDecision::Eligible,
            vec![],
            vec![
                "2026-11-03".to_owned(),
                "2026-11-04".to_owned(),
                "2026-11-05".to_owned(),
            ],
            now - chrono::Duration::seconds(10),
            now - chrono::Duration::seconds(1),
        );
        assert_eq!(
            validate_report_gate(&expired),
            Err(PaperCalculationInputError::Eligibility(
                Reason::EligibilityExpired
            ))
        );
    }
}
