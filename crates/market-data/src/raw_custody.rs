//! Borrows the terminal consumer's custody attempt without granting Raw authority.
//! Local settlement preserves live owners and never finishes the terminal attempt.

use crate::checked_close::{Attempt, Failure, Gap};
use crate::paper_source_evidence::ExecutionGateFailure;

/// Private bridge implemented only by the checked protected-current context.
/// The public constructor below never installs one.
pub(crate) trait NativeAdmissionContext {
    fn compare_scopes(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
    ) -> Result<(), ExecutionGateFailure>;
    fn admit(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
    ) -> Result<(), ExecutionGateFailure>;
}

// Retain both completed checks in operation order. The join owns the later
// typed error even when the original result already failed.
pub(crate) fn join_native_checks<E, X>(
    original: Result<(), E>,
    execution: Result<(), X>,
    join: impl FnOnce(Option<E>, X) -> E,
) -> Result<(), E> {
    match (original, execution) {
        (original, Ok(())) => original,
        (Err(first), Err(later)) => Err(join(Some(first), later)),
        (Ok(()), Err(later)) => Err(join(None, later)),
    }
}

/// Borrows the terminal consumer's ONE Attempt. No global/thread-local scope.
/// The terminal consumer may finish that Attempt only after every lease settles.
pub struct RawInvocation<'attempt> {
    attempt: &'attempt Attempt,
    context: Option<&'attempt dyn NativeAdmissionContext>,
}

impl<'attempt> RawInvocation<'attempt> {
    pub fn new(attempt: &'attempt Attempt) -> Self {
        Self {
            attempt,
            context: None,
        }
    }
    pub(crate) fn with_native_context(
        attempt: &'attempt Attempt,
        context: &'attempt dyn NativeAdmissionContext,
    ) -> Self {
        Self {
            attempt,
            context: Some(context),
        }
    }
    pub(crate) fn attempt(&self) -> &'attempt Attempt {
        self.attempt
    }
    /// Makes an unbound bootstrap view over the same ledger without exposing
    /// the terminal Attempt or carrying this invocation's private context.
    pub fn unbound_bootstrap(&self) -> Self {
        Self::new(self.attempt)
    }
    pub(crate) fn native_context(&self) -> Option<&dyn NativeAdmissionContext> {
        self.context
    }
    pub(crate) fn compare_native_context_scopes(
        &self,
        expected: &Attempt,
    ) -> Result<(), ExecutionGateFailure> {
        match self.context {
            Some(context) => context.compare_scopes(self, expected),
            None => Ok(()),
        }
    }
    pub(crate) fn refused<E>(&self, gap: Gap) -> Failure<E> {
        self.attempt.gap(gap);
        self.attempt.failure(None)
    }
    /// Local settlement result, NOT terminal Attempt::finish or authority.
    pub(crate) fn settled<T, E>(&self, body: Result<T, E>) -> Result<T, Failure<E>> {
        match body {
            Err(body) => {
                self.attempt.gap(Gap::Abandoned);
                Err(self.attempt.failure(Some(body)))
            }
            Ok(value) if self.attempt.check() => Ok(value),
            Ok(_) => Err(self.refused(Gap::Reuse)),
        }
    }
}

impl<'attempt> RawInvocation<'attempt> {
    /// Settles a local operation into the caller's real Attempt without
    /// finishing it. A foreign expected scope poisons both actual ledgers.
    pub fn settle_local_in<T, E>(
        &self,
        expected: &Attempt,
        body: Result<T, E>,
    ) -> Result<T, Failure<E>> {
        if !self.attempt.same_scope(expected) {
            self.attempt.gap(Gap::ScopeMismatch);
            expected.gap(Gap::ScopeMismatch);
        }
        self.settled(body)
    }
}

#[cfg(test)]
mod tests {
    use super::RawInvocation;
    use crate::checked_close::{Attempt, Close, Errno, Gap, Issue, Role};
    use std::{cell::Cell, convert::Infallible, rc::Rc};

    struct Counted {
        calls: Rc<Cell<usize>>,
        result: Result<(), Errno>,
    }

    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.calls.set(self.calls.get() + 1);
            self.result
        }
    }

    fn counted(calls: &Rc<Cell<usize>>, result: Result<(), Errno>) -> Counted {
        Counted {
            calls: Rc::clone(calls),
            result,
        }
    }

    struct Body(Box<str>);

    #[test]
    fn local_settlement_keeps_owner_live_until_explicit_cleanup() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(counted(&calls, Ok(())), Role::Ancestor);
        assert!(invocation.attempt().same_scope(&attempt));
        let value = invocation
            .settle_local_in(&attempt, Ok::<_, Body>("local value"))
            .unwrap();
        assert_eq!(value, "local value");
        assert!(attempt.check());
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(attempt.report().attempted(), 0);
        assert_eq!(calls.get(), 0);

        assert!(owner.finish());
        assert_eq!(calls.get(), 1);
        assert_eq!(attempt.report().closed(), 1);
        assert!(attempt.finish(Ok::<_, Infallible>(())).is_ok());
    }

    #[test]
    fn foreign_expected_poison_is_unconditional_and_keeps_whole_body() {
        let actual = Attempt::default();
        let expected = Attempt::default();
        let actual_calls = Rc::new(Cell::new(0));
        let expected_calls = Rc::new(Cell::new(0));
        let actual_owner = actual.adopt(counted(&actual_calls, Ok(())), Role::Ancestor);
        let expected_owner = expected.adopt(
            counted(&expected_calls, Err(Errno::BadDescriptor)),
            Role::Source,
        );
        actual.gap(Gap::Unbound);
        let invocation = RawInvocation::new(&actual);
        let body = Body("original operation body".to_owned().into_boxed_str());
        let body_address = body.0.as_ptr();
        let failure = match invocation.settle_local_in(&expected, Err::<(), _>(body)) {
            Err(failure) => failure,
            Ok(()) => panic!("foreign scope must never return success"),
        };
        assert_eq!(failure.body().unwrap().0.as_ptr(), body_address);
        assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::Unbound)));
        assert_eq!(
            failure.report().secondary(),
            &[Issue::Gap(Gap::ScopeMismatch), Issue::Gap(Gap::Abandoned)]
        );
        assert_eq!(failure.report().live(), 1);
        assert_eq!(failure.report().attempted(), 0);
        let expected_before_cleanup = expected.report();
        assert_eq!(
            expected_before_cleanup.first(),
            Some(Issue::Gap(Gap::ScopeMismatch))
        );
        assert_eq!(expected_before_cleanup.live(), 1);
        assert_eq!(expected_before_cleanup.attempted(), 0);
        assert!(!actual.check());
        assert!(!expected.check());

        assert!(!actual_owner.finish());
        assert!(!expected_owner.finish());
        assert_eq!(actual_calls.get(), 1);
        assert_eq!(expected_calls.get(), 1);
        assert_eq!(actual.report().closed(), 1);
        assert_eq!(actual.report().unknown(), 0);
        assert_eq!(expected.report().closed(), 0);
        assert_eq!(expected.report().unknown(), 1);
        assert_eq!(actual.report().live(), 0);
        assert_eq!(expected.report().live(), 0);
        assert_eq!(failure.report().live(), 1);
        assert_eq!(failure.report().attempted(), 0);
        assert_eq!(expected_before_cleanup.attempted(), 0);
    }

    #[test]
    fn dirty_same_scope_success_stays_bodyless_and_refuses_value() {
        let attempt = Attempt::default();
        attempt.gap(Gap::TerminalPublication);
        let invocation = RawInvocation::new(&attempt);
        let failure = match invocation.settle_local_in(&attempt, Ok::<_, Body>("suppressed value"))
        {
            Err(failure) => failure,
            Ok(_) => panic!("a dirty ledger cannot release a value"),
        };
        assert!(failure.body().is_none());
        assert_eq!(
            failure.report().first(),
            Some(Issue::Gap(Gap::TerminalPublication))
        );
        assert_eq!(failure.report().secondary(), &[Issue::Gap(Gap::Reuse)]);
        assert_eq!(failure.report().attempted(), 0);
        assert_eq!(failure.report().live(), 0);
        assert!(!attempt.check());
    }

    #[test]
    fn unbound_bootstrap_keeps_the_same_attempt_without_context() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let bootstrap = invocation.unbound_bootstrap();

        assert!(std::ptr::eq(invocation.attempt(), bootstrap.attempt()));
        assert!(bootstrap.native_context().is_none());
        assert!(attempt.check());
    }
}

#[cfg(test)]
mod native_join_repair_tests {
    use super::join_native_checks;
    use crate::checked_close::{Attempt, Failure, Gap};

    struct Body(Box<str>);
    enum Source {
        Original(Failure<Body>),
        Joined {
            first: Option<Box<Source>>,
            later: Body,
        },
    }
    #[test]
    fn earlier_some_and_none_keep_whole_later_nonclone_error() {
        for has_body in [true, false] {
            let attempt = Attempt::default();
            attempt.gap(Gap::Abandoned);
            let body = has_body.then(|| Body("original".to_owned().into_boxed_str()));
            let pointer = body.as_ref().map(|b| b.0.as_ptr());
            let first = Source::Original(attempt.failure(body));
            let later = Body("later".to_owned().into_boxed_str());
            let later_pointer = later.0.as_ptr();
            let joined =
                join_native_checks(Err::<(), _>(first), Err::<(), _>(later), |first, later| {
                    Source::Joined {
                        first: first.map(Box::new),
                        later,
                    }
                });
            let Source::Joined {
                first: Some(first),
                later,
            } = joined.err().unwrap()
            else {
                panic!("both actual errors retained")
            };
            let Source::Original(first) = *first else {
                panic!("original stays primary")
            };
            assert_eq!(first.body().map(|b| b.0.as_ptr()), pointer);
            assert_eq!(later.0.as_ptr(), later_pointer);
            assert!(first.report().first().is_some());
        }
    }
}
