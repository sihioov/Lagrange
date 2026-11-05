//! Bounded async actions that retain the actual Paper runtime and Current
//! binding while API or worker SQL work is in progress.
//!
//! This scope is a process/currentness boundary, not a SQL permission or
//! transaction protocol. A callback must revalidate immediately before each
//! economic query/commit and enforce bounded SQL timeouts, explicit rollback,
//! and its own rights/epoch/claim fences. An unknown commit requires readback;
//! this module never retries a callback.

use std::{
    convert::Infallible,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use market_data::{
    checked_close::{Attempt, Gap},
    paper_source_evidence::{
        CheckedCurrentFailure, ProtectedCurrentBinding, ProtectedExecutionContext,
        UntrustedCurrentEnvelope,
    },
    raw_custody::RawInvocation,
};
use serde_json::Value;
use tokio::sync::watch;

use crate::paper_io::{BlockingIoError, run_bounded_blocking};

use super::runtime::{PaperProcessRole, PaperRuntimeError, TrustedPaperRuntime};

/// The callback future is deliberately local and may be `!Send`.
pub type PaperActionFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + 'a>>;

/// Async callback accepted by [`run_bound_paper_action`]. Its closure is sent
/// to the bounded blocking worker, then invoked there with a borrowed,
/// non-Send context. The returned future is never moved to another thread.
pub trait PaperActionWork<T: 'static, E: 'static>: Send + 'static {
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, T, E>;
}

impl<T: 'static, E: 'static, F> PaperActionWork<T, E> for F
where
    F: for<'ctx, 'scope, 'attempt> FnOnce(
            &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
        ) -> PaperActionFuture<'ctx, T, E>
        + Send
        + 'static,
{
    fn run<'ctx, 'scope, 'attempt>(
        self: Box<Self>,
        context: &'ctx PaperActionContext<'ctx, 'scope, 'attempt>,
    ) -> PaperActionFuture<'ctx, T, E> {
        (*self)(context)
    }
}

/// Static, value-free refusal reasons for acquiring or revalidating scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperActionRefusal {
    RuntimeUnbound,
    RuntimeRejected,
    CurrentUnavailable,
    CurrentNotCurrent,
    BindingMismatch,
    Canceled,
    DeadlineExceeded,
    ExecutorUnavailable,
    CallbackPanicked,
}

impl PaperActionRefusal {
    pub const fn code(self) -> &'static str {
        match self {
            Self::RuntimeUnbound => "PAPER_ACTION_RUNTIME_UNBOUND",
            Self::RuntimeRejected => "PAPER_ACTION_RUNTIME_REJECTED",
            Self::CurrentUnavailable => "PAPER_ACTION_CURRENT_UNAVAILABLE",
            Self::CurrentNotCurrent => "PAPER_ACTION_CURRENT_NOT_CURRENT",
            Self::BindingMismatch => "PAPER_ACTION_BINDING_MISMATCH",
            Self::Canceled => "PAPER_ACTION_CANCELED",
            Self::DeadlineExceeded => "PAPER_ACTION_DEADLINE_EXCEEDED",
            Self::ExecutorUnavailable => "PAPER_ACTION_EXECUTOR_UNAVAILABLE",
            Self::CallbackPanicked => "PAPER_ACTION_CALLBACK_PANICKED",
        }
    }
}

impl fmt::Display for PaperActionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Result class; `CommitUnknown` always requires a state/readback check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundPaperActionErrorKind {
    Refused,
    ActionFailed,
    CompletionUnclean,
    CommitUnknown,
}

/// Which actual cleanup/completion result failed. Values and provider messages
/// are intentionally omitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperActionCompletionFailure {
    CurrentOpen,
    CurrentFinish,
    ExpectedScope,
    CapturedScope,
    AttemptFinish,
    PostActionRevalidation,
}

/// Typed action error which can retain both the callback's domain error and
/// all cleanup failures without formatting the domain error or its payload.
pub struct BoundPaperActionError<E> {
    kind: BoundPaperActionErrorKind,
    refusal: Option<PaperActionRefusal>,
    work_error: Option<E>,
    completion_failures: Vec<PaperActionCompletionFailure>,
}

impl<E> BoundPaperActionError<E> {
    pub fn kind(&self) -> BoundPaperActionErrorKind {
        self.kind
    }

    pub fn refusal(&self) -> Option<PaperActionRefusal> {
        self.refusal
    }

    pub fn work_error(&self) -> Option<&E> {
        self.work_error.as_ref()
    }

    pub fn completion_failures(&self) -> &[PaperActionCompletionFailure] {
        &self.completion_failures
    }
}

impl<E> fmt::Debug for BoundPaperActionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundPaperActionError")
            .field("kind", &self.kind)
            .field("refusal", &self.refusal)
            .field("work_error_present", &self.work_error.is_some())
            .field("completion_failures", &self.completion_failures)
            .finish()
    }
}

impl<E> fmt::Display for BoundPaperActionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self.kind {
            BoundPaperActionErrorKind::Refused => "PAPER_ACTION_REFUSED",
            BoundPaperActionErrorKind::ActionFailed => "PAPER_ACTION_FAILED",
            BoundPaperActionErrorKind::CompletionUnclean => "PAPER_ACTION_COMPLETION_UNCLEAN",
            BoundPaperActionErrorKind::CommitUnknown => {
                "PAPER_ACTION_COMMIT_UNKNOWN_READBACK_REQUIRED"
            }
        };
        f.write_str(code)
    }
}

impl<E: 'static> std::error::Error for BoundPaperActionError<E> {}

/// Borrowed evidence available to one API or worker action. It exposes the
/// actual read-only runtime and the snapshot selector, never the mutable
/// protected Current holder or a transferable permit.
pub struct PaperActionContext<'ctx, 'scope, 'attempt> {
    runtime: &'ctx TrustedPaperRuntime,
    current: &'ctx ProtectedExecutionContext<'scope, 'attempt>,
    binding: &'ctx Value,
    canceled: &'ctx AtomicBool,
    started: &'ctx AtomicBool,
    deadline: Instant,
    attempt: &'ctx Attempt,
}

impl PaperActionContext<'_, '_, '_> {
    pub fn runtime(&self) -> &TrustedPaperRuntime {
        self.runtime
    }

    /// The real binding selector cloned from a protected CURRENT snapshot.
    /// It remains selector data and does not grant action authority.
    pub fn binding(&self) -> &Value {
        self.binding
    }

    /// Recheck cancellation/deadline, the real fixed-path Current snapshot,
    /// and the protected config/executable before continuing an action.
    pub fn revalidate(&self) -> Result<(), PaperActionRefusal> {
        self.checkpoint_boundary()?;

        let result = self.current.with_snapshot(|snapshot| {
            let actual = current_selector(snapshot)?;
            if actual != *self.binding {
                return Err(PaperActionRefusal::BindingMismatch);
            }
            self.runtime
                .validate_current_binding(&actual)
                .map_err(map_runtime_error)
        });
        match result {
            Ok(inner) => inner,
            Err(_) => {
                if self.started.load(Ordering::Acquire) {
                    self.attempt.gap(Gap::Abandoned);
                }
                Err(PaperActionRefusal::CurrentUnavailable)
            }
        }
    }

    /// Borrowed cancellation/deadline check only. No Current/file/runtime IO;
    /// callers still perform full revalidation before their final lease seal.
    pub(super) fn checkpoint_boundary(&self) -> Result<(), PaperActionRefusal> {
        checkpoint_action_boundary(self.canceled, self.started, self.deadline, self.attempt)
    }
}

pub(super) fn checkpoint_action_boundary(
    canceled: &AtomicBool,
    started: &AtomicBool,
    deadline: Instant,
    attempt: &Attempt,
) -> Result<(), PaperActionRefusal> {
    if let Some(reason) =
        boundary_refusal(canceled.load(Ordering::Acquire), Instant::now() >= deadline)
    {
        if started.load(Ordering::Acquire) {
            attempt.gap(Gap::Abandoned);
        }
        return Err(reason);
    }
    Ok(())
}

/// Run one local async callback while the actual Current owner and trusted
/// process runtime remain alive. The outer future is `Send`; the callback's
/// future may be `!Send` because `block_on` polls it on the owned blocking
/// caller; the private runtime worker supports synchronous nested IO adapters.
pub async fn run_bound_paper_action<T, E, F>(
    role: PaperProcessRole,
    timeout: Duration,
    shutdown: Option<watch::Receiver<bool>>,
    work: F,
) -> Result<T, BoundPaperActionError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: PaperActionWork<T, E>,
{
    let now = Instant::now();
    let Some(deadline) = now.checked_add(timeout) else {
        return Err(action_error(
            BoundPaperActionErrorKind::Refused,
            Some(PaperActionRefusal::DeadlineExceeded),
            None,
            Vec::new(),
        ));
    };
    let started = Arc::new(AtomicBool::new(false));
    let started_by_work = started.clone();
    let result = run_bounded_blocking(timeout, shutdown, move |canceled| {
        Ok::<_, Infallible>(run_local_action(
            role,
            deadline,
            canceled,
            started_by_work,
            work,
        ))
    })
    .await;

    match result {
        Ok(completion) => classify_completion(completion),
        Err(BlockingIoError::Canceled) => Err(blocking_refusal(
            started.load(Ordering::Acquire),
            PaperActionRefusal::Canceled,
        )),
        Err(BlockingIoError::TimedOut) => Err(blocking_refusal(
            started.load(Ordering::Acquire),
            PaperActionRefusal::DeadlineExceeded,
        )),
        Err(BlockingIoError::Failed(never)) => match never {},
    }
}

fn current_selector(snapshot: &UntrustedCurrentEnvelope) -> Result<Value, PaperActionRefusal> {
    if snapshot.status().ok() != Some("CURRENT") {
        return Err(PaperActionRefusal::CurrentNotCurrent);
    }
    snapshot
        .value()
        .get("binding")
        .filter(|binding| !binding.is_null())
        .cloned()
        .ok_or(PaperActionRefusal::CurrentNotCurrent)
}

fn boundary_refusal(canceled: bool, expired: bool) -> Option<PaperActionRefusal> {
    if canceled {
        Some(PaperActionRefusal::Canceled)
    } else if expired {
        Some(PaperActionRefusal::DeadlineExceeded)
    } else {
        None
    }
}

fn map_runtime_error(error: PaperRuntimeError) -> PaperActionRefusal {
    match error {
        PaperRuntimeError::Unbound => PaperActionRefusal::RuntimeUnbound,
        PaperRuntimeError::CurrentBindingMismatch => PaperActionRefusal::BindingMismatch,
        _ => PaperActionRefusal::RuntimeRejected,
    }
}

fn blocking_refusal<E>(started: bool, reason: PaperActionRefusal) -> BoundPaperActionError<E> {
    let kind = if started {
        BoundPaperActionErrorKind::CommitUnknown
    } else {
        BoundPaperActionErrorKind::Refused
    };
    action_error(kind, Some(reason), None, Vec::new())
}

struct ActionCompletion<T, E> {
    setup_refusal: Option<PaperActionRefusal>,
    operation: Option<Result<T, E>>,
    callback_panicked: bool,
    post_refusal: Option<PaperActionRefusal>,
    current_open_failed: bool,
    current_finish_clean: Option<bool>,
    expected_scope_clean: bool,
    captured_scope_clean: bool,
    attempt_finish_clean: bool,
}

impl<T, E> ActionCompletion<T, E> {
    fn failures(&self) -> Vec<PaperActionCompletionFailure> {
        let mut failures = Vec::new();
        if self.current_open_failed {
            failures.push(PaperActionCompletionFailure::CurrentOpen);
        }
        if self.current_finish_clean == Some(false) {
            failures.push(PaperActionCompletionFailure::CurrentFinish);
        }
        if !self.expected_scope_clean {
            failures.push(PaperActionCompletionFailure::ExpectedScope);
        }
        if !self.captured_scope_clean {
            failures.push(PaperActionCompletionFailure::CapturedScope);
        }
        if !self.attempt_finish_clean {
            failures.push(PaperActionCompletionFailure::AttemptFinish);
        }
        if self.post_refusal.is_some() {
            failures.push(PaperActionCompletionFailure::PostActionRevalidation);
        }
        failures
    }
}

fn classify_completion<T, E>(
    completion: ActionCompletion<T, E>,
) -> Result<T, BoundPaperActionError<E>> {
    let failures = completion.failures();
    if let Some(refusal) = completion.setup_refusal {
        let kind = if failures.is_empty() {
            BoundPaperActionErrorKind::Refused
        } else {
            BoundPaperActionErrorKind::CompletionUnclean
        };
        return Err(action_error(kind, Some(refusal), None, failures));
    }
    if completion.callback_panicked {
        return Err(action_error(
            BoundPaperActionErrorKind::CommitUnknown,
            Some(PaperActionRefusal::CallbackPanicked),
            None,
            failures,
        ));
    }
    let Some(operation) = completion.operation else {
        return Err(action_error(
            BoundPaperActionErrorKind::CompletionUnclean,
            Some(PaperActionRefusal::ExecutorUnavailable),
            None,
            failures,
        ));
    };
    if !failures.is_empty() {
        return Err(action_error(
            BoundPaperActionErrorKind::CommitUnknown,
            completion.post_refusal,
            operation.err(),
            failures,
        ));
    }
    match operation {
        Ok(value) => Ok(value),
        Err(error) => Err(action_error(
            BoundPaperActionErrorKind::ActionFailed,
            None,
            Some(error),
            Vec::new(),
        )),
    }
}

fn action_error<E>(
    kind: BoundPaperActionErrorKind,
    refusal: Option<PaperActionRefusal>,
    work_error: Option<E>,
    completion_failures: Vec<PaperActionCompletionFailure>,
) -> BoundPaperActionError<E> {
    BoundPaperActionError {
        kind,
        refusal,
        work_error,
        completion_failures,
    }
}

fn build_action_executor() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}

fn run_local_action<T, E, F>(
    role: PaperProcessRole,
    deadline: Instant,
    canceled: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    work: F,
) -> ActionCompletion<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
    F: PaperActionWork<T, E>,
{
    let attempt = Attempt::default();
    let captured = attempt.clone();
    let bootstrap = RawInvocation::new(&attempt);
    let mut runtime_hold = None;
    let mut current_failure_hold: Option<CheckedCurrentFailure> = None;
    let mut setup_refusal = None;
    let mut operation = None;
    let mut callback_panicked = false;
    let mut post_refusal = None;
    let mut current_open_failed = false;
    let mut current_finish_clean = None;

    match TrustedPaperRuntime::load(role) {
        Err(error) => setup_refusal = Some(map_runtime_error(error)),
        Ok(runtime) => {
            runtime_hold = Some(runtime);
            match ProtectedCurrentBinding::open_checked_in(&bootstrap, &attempt, &captured) {
                Err(error) => {
                    current_open_failed = true;
                    current_failure_hold = Some(error);
                    setup_refusal = Some(PaperActionRefusal::CurrentUnavailable);
                }
                Ok(current) => {
                    let current = current.into_execution_context();
                    let binding_result = current.with_snapshot(current_selector);
                    match binding_result {
                        Err(_) => setup_refusal = Some(PaperActionRefusal::CurrentUnavailable),
                        Ok(Err(reason)) => setup_refusal = Some(reason),
                        Ok(Ok(binding)) => {
                            let runtime = runtime_hold.as_ref().expect("loaded runtime");
                            let context = PaperActionContext {
                                runtime,
                                current: &current,
                                binding: &binding,
                                canceled: &canceled,
                                started: &started,
                                deadline,
                                attempt: &attempt,
                            };
                            if let Err(reason) = context.revalidate() {
                                setup_refusal = Some(reason);
                            } else {
                                match build_action_executor() {
                                    Err(_) => {
                                        setup_refusal =
                                            Some(PaperActionRefusal::ExecutorUnavailable)
                                    }
                                    Ok(local_runtime) => {
                                        // Recheck immediately before entering callback code. The
                                        // callback must revalidate before every economic boundary.
                                        if let Err(reason) = context.revalidate() {
                                            setup_refusal = Some(reason);
                                        } else {
                                            started.store(true, Ordering::Release);
                                            // Close the small cancellation/deadline race between
                                            // the preceding check and callback entry.
                                            if let Err(reason) = context.revalidate() {
                                                setup_refusal = Some(reason);
                                            } else {
                                                let invoked = std::panic::catch_unwind(
                                                    std::panic::AssertUnwindSafe(|| {
                                                        local_runtime
                                                            .block_on(Box::new(work).run(&context))
                                                    }),
                                                );
                                                match invoked {
                                                    Ok(result) => operation = Some(result),
                                                    Err(_) => callback_panicked = true,
                                                }
                                                post_refusal = context.revalidate().err();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    current_finish_clean = Some(current.finish_local().is_clean());
                }
            }
        }
    }

    // Keep the actual failed Current result alive through scope settlement and
    // Attempt::finish; its diagnostics are intentionally not string-rendered.
    let current_failure_retained = current_failure_hold.is_some();
    let expected_scope_clean = bootstrap
        .settle_local_in(&attempt, Ok::<(), Infallible>(()))
        .is_ok();
    let captured_scope_clean = bootstrap
        .settle_local_in(&captured, Ok::<(), Infallible>(()))
        .is_ok();
    let attempt_finish_clean = attempt.finish(Ok::<(), Infallible>(())).is_ok();
    let _runtime_remained_alive_through_completion = runtime_hold.is_some();
    ActionCompletion {
        setup_refusal,
        operation,
        callback_panicked,
        post_refusal,
        current_open_failed: current_failure_retained || current_open_failed,
        current_finish_clean,
        expected_scope_clean,
        captured_scope_clean,
        attempt_finish_clean,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_executor_preserves_non_send_callback_at_polars_block_in_place_seam() {
        // The pinned Polars 0.54.4 seam is block_in_place(|| io_rt.block_on(f)).
        // This ordinary regression needs no Current/base/DB authority.
        let executor = build_action_executor().expect("private action executor builds");
        let io_executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("ordinary nested IO executor builds");
        let caller = std::thread::current().id();
        let result = executor.block_on(async {
            let local = std::rc::Rc::new(41_u8);
            tokio::task::yield_now().await;
            assert_eq!(std::thread::current().id(), caller);
            let result = tokio::task::block_in_place(|| {
                assert_eq!(std::thread::current().id(), caller);
                io_executor.block_on(async { *local + 1 })
            });
            tokio::task::yield_now().await;
            assert_eq!(std::thread::current().id(), caller);
            assert_eq!(*local, 41);
            result
        });
        assert_eq!(result, 42);
    }

    #[test]
    fn cancellation_and_deadline_refuse_before_action() {
        assert_eq!(
            boundary_refusal(true, false),
            Some(PaperActionRefusal::Canceled)
        );
        assert_eq!(
            boundary_refusal(false, true),
            Some(PaperActionRefusal::DeadlineExceeded)
        );
        assert_eq!(boundary_refusal(false, false), None);
    }

    #[test]
    fn cheap_checkpoint_preserves_started_abandonment_and_cancellation_priority() {
        for entered in [false, true] {
            let attempt = Attempt::default();
            let canceled = AtomicBool::new(true);
            let started = AtomicBool::new(entered);
            let expired = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
            assert_eq!(
                checkpoint_action_boundary(&canceled, &started, expired, &attempt),
                Err(PaperActionRefusal::Canceled)
            );
            assert_eq!(
                attempt.report().first(),
                entered.then_some(market_data::checked_close::Issue::Gap(Gap::Abandoned))
            );
        }
    }

    #[test]
    fn cheap_checkpoint_keeps_valid_scope_clean_and_records_started_expiry() {
        let attempt = Attempt::default();
        let canceled = AtomicBool::new(false);
        let started = AtomicBool::new(true);
        assert_eq!(
            checkpoint_action_boundary(
                &canceled,
                &started,
                Instant::now() + Duration::from_secs(60),
                &attempt
            ),
            Ok(())
        );
        assert!(attempt.check());
        assert_eq!(
            checkpoint_action_boundary(&canceled, &started, Instant::now(), &attempt),
            Err(PaperActionRefusal::DeadlineExceeded)
        );
        assert_eq!(
            attempt.report().first(),
            Some(market_data::checked_close::Issue::Gap(Gap::Abandoned))
        );
    }

    #[test]
    fn outer_timeout_is_unknown_only_after_callback_entry() {
        let before = blocking_refusal::<()>(false, PaperActionRefusal::DeadlineExceeded);
        assert_eq!(before.kind(), BoundPaperActionErrorKind::Refused);
        let during = blocking_refusal::<()>(true, PaperActionRefusal::DeadlineExceeded);
        assert_eq!(during.kind(), BoundPaperActionErrorKind::CommitUnknown);
        assert_eq!(during.refusal(), Some(PaperActionRefusal::DeadlineExceeded));
    }

    #[test]
    fn callback_and_cleanup_errors_are_both_retained_without_debugging_values() {
        let completion: ActionCompletion<(), String> = ActionCompletion {
            setup_refusal: None,
            operation: Some(Err(String::from("sensitive domain detail"))),
            callback_panicked: false,
            post_refusal: None,
            current_open_failed: false,
            current_finish_clean: Some(false),
            expected_scope_clean: true,
            captured_scope_clean: false,
            attempt_finish_clean: false,
        };
        let Err(error) = classify_completion(completion) else {
            panic!("unclean callback result must be commit-unknown")
        };
        assert_eq!(error.kind(), BoundPaperActionErrorKind::CommitUnknown);
        assert_eq!(
            error.work_error().map(String::as_str),
            Some("sensitive domain detail")
        );
        assert_eq!(
            error.completion_failures(),
            &[
                PaperActionCompletionFailure::CurrentFinish,
                PaperActionCompletionFailure::CapturedScope,
                PaperActionCompletionFailure::AttemptFinish,
            ]
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("sensitive domain detail"));
    }

    #[test]
    fn clean_domain_failure_preserves_typed_error_without_claiming_commit_unknown() {
        let completion: ActionCompletion<(), u8> = ActionCompletion {
            setup_refusal: None,
            operation: Some(Err(17_u8)),
            callback_panicked: false,
            post_refusal: None,
            current_open_failed: false,
            current_finish_clean: Some(true),
            expected_scope_clean: true,
            captured_scope_clean: true,
            attempt_finish_clean: true,
        };
        let Err(error) = classify_completion(completion) else {
            panic!("domain error must be returned")
        };
        assert_eq!(error.kind(), BoundPaperActionErrorKind::ActionFailed);
        assert_eq!(error.work_error(), Some(&17));
        assert!(error.completion_failures().is_empty());
    }
}
