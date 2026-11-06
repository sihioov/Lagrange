//! Invocation-local cleanup accounting shared by checked Raw readers.
//! Reports describe custody, not source origin, durability, or publication authority.

use std::sync::{Arc, Mutex, MutexGuard};

const MAX_ISSUES: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Ancestor,
    Child,
    Clone,
    BootId,
    Enumeration,
    Current,
    Source,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errno {
    Interrupted,
    BadDescriptor,
    Io,
    Other(i32),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knowledge {
    Closed,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    Unbound,
    NonLinux,
    RawDailyClose,
    DatabaseClose,
    TerminalPublication,
    Overflow,
    LiveOwners,
    Reuse,
    ScopeMismatch,
    Abandoned,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    Close {
        role: Role,
        errno: Errno,
        state: Knowledge,
    },
    Gap(Gap),
    MutexPoison,
}
#[derive(Clone, Debug)]
pub struct Report {
    first: Option<Issue>,
    secondary: Vec<Issue>,
    omitted: usize,
    attempted: usize,
    closed: usize,
    unknown: usize,
    live: usize,
}
impl Report {
    pub fn first(&self) -> Option<Issue> {
        self.first
    }
    pub fn secondary(&self) -> &[Issue] {
        &self.secondary
    }
    pub fn omitted(&self) -> usize {
        self.omitted
    }
    pub fn attempted(&self) -> usize {
        self.attempted
    }
    pub fn closed(&self) -> usize {
        self.closed
    }
    pub fn unknown(&self) -> usize {
        self.unknown
    }
    pub fn live(&self) -> usize {
        self.live
    }
}

pub struct Failure<E> {
    body: Option<E>,
    report: Report,
}
impl<E> Failure<E> {
    pub fn body(&self) -> Option<&E> {
        self.body.as_ref()
    }
    pub fn report(&self) -> &Report {
        &self.report
    }
    pub fn into_parts(self) -> (Option<E>, Report) {
        (self.body, self.report)
    }
}
// Generic body values may carry provider text. This diagnostic view never
// formats E, and deliberately does not require E: Debug or Display.
impl<E> std::fmt::Debug for Failure<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Failure")
            .field("body_present", &self.body.is_some())
            .field("report", &self.report)
            .finish()
    }
}
// If body is Some it is primary; report first/secondary are cleanup diagnostics.
// Otherwise report.first is primary. No string-dumped OS/provider messages.
#[derive(Default)]
struct State {
    issues: Vec<Issue>,
    omitted: usize,
    attempted: usize,
    closed: usize,
    unknown: usize,
    live: usize,
    finished: bool,
}
#[derive(Clone, Default)]
pub struct Attempt(Arc<Mutex<State>>);
impl Attempt {
    pub fn same_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    fn state(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(s) => s,
            Err(p) => {
                let mut s = p.into_inner();
                Self::push(&mut s, Issue::MutexPoison);
                s
            }
        }
    }
    fn push(s: &mut State, issue: Issue) {
        if s.issues.len() < MAX_ISSUES {
            s.issues.push(issue)
        } else {
            s.omitted = s.omitted.saturating_add(1)
        }
    }
    pub fn check(&self) -> bool {
        let s = self.state();
        !s.finished && s.issues.is_empty() && s.omitted == 0
    }
    pub fn gap(&self, gap: Gap) {
        Self::push(&mut self.state(), Issue::Gap(gap));
    }
    pub fn report(&self) -> Report {
        let s = self.state();
        Report {
            first: s.issues.first().copied(),
            secondary: s.issues.iter().skip(1).copied().collect(),
            omitted: s.omitted,
            attempted: s.attempted,
            closed: s.closed,
            unknown: s.unknown,
            live: s.live,
        }
    }
    /// Records a failure against this actual ledger; callers cannot supply a
    /// replacement Report or reconstruct/clear live state from diagnostics.
    /// A failure cannot be ignored and followed by successful terminal finish.
    pub fn failure<E>(&self, body: Option<E>) -> Failure<E> {
        {
            let mut state = self.state();
            if state.issues.is_empty() && state.omitted == 0 {
                let gap = if state.finished {
                    Gap::Reuse
                } else {
                    Gap::Abandoned
                };
                Self::push(&mut state, Issue::Gap(gap));
            }
        }
        Failure {
            body,
            report: self.report(),
        }
    }
    pub fn adopt<R: Close>(&self, resource: R, role: Role) -> Owner<R> {
        let mut s = self.state();
        if s.finished {
            Self::push(&mut s, Issue::Gap(Gap::Reuse));
        }
        s.live += 1;
        drop(s);
        Owner {
            resource: Some(resource),
            attempt: self.clone(),
            role,
        }
    }
    pub fn finish<T, E>(self, body: Result<T, E>) -> Result<T, Failure<E>> {
        {
            let mut s = self.state();
            if s.finished {
                Self::push(&mut s, Issue::Gap(Gap::Reuse));
            }
            s.finished = true;
            if s.live != 0 {
                Self::push(&mut s, Issue::Gap(Gap::LiveOwners));
            }
        }
        let report = self.report();
        match body {
            Err(body) => Err(Failure {
                body: Some(body),
                report,
            }),
            Ok(value) if report.first.is_none() && report.omitted == 0 && report.live == 0 => {
                Ok(value)
            }
            Ok(_) => Err(Failure { body: None, report }),
        }
    }
}
// Cleanup contract only, never protected source/origin authority. The native
// File implementation belongs here; model resources cannot construct a
// downstream opaque protected reader proof.
pub trait Close {
    fn close(self) -> Result<(), Errno>;
}
pub struct Owner<R: Close> {
    resource: Option<R>,
    attempt: Attempt,
    role: Role,
}
impl<R: Close> Owner<R> {
    pub fn get(&self) -> &R {
        self.resource
            .as_ref()
            .expect("owned until consuming finish")
    }
    pub fn get_mut(&mut self) -> &mut R {
        self.resource
            .as_mut()
            .expect("owned until consuming finish")
    }
    pub fn attempt(&self) -> &Attempt {
        &self.attempt
    }
    fn settle(&mut self) {
        if let Some(resource) = self.resource.take() {
            // DISARM before native close.
            let result = resource.close(); // One attempt, including EINTR. Never retry.
            let mut s = self.attempt.state();
            s.live -= 1;
            s.attempted += 1;
            match result {
                Ok(()) => s.closed += 1,
                Err(errno) => {
                    s.unknown += 1;
                    Attempt::push(
                        &mut s,
                        Issue::Close {
                            role: self.role,
                            errno,
                            state: Knowledge::Unknown,
                        },
                    );
                }
            }
        }
    }
    pub fn finish(mut self) -> bool {
        self.settle();
        self.attempt.check()
    }
}
impl<R: Close> Drop for Owner<R> {
    fn drop(&mut self) {
        self.settle();
    }
}

/// The same File owner used by the private checked directory graph.
#[cfg(target_os = "linux")]
pub type CheckedFile = Owner<std::fs::File>;

#[cfg(target_os = "linux")]
mod native {
    use super::{Close, Errno};
    use std::{fs::File, os::fd::IntoRawFd};

    pub fn errno() -> Errno {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EINTR) => Errno::Interrupted,
            Some(libc::EBADF) => Errno::BadDescriptor,
            Some(libc::EIO) => Errno::Io,
            Some(n) => Errno::Other(n),
            None => Errno::Unknown,
        }
    }
    impl Close for File {
        fn close(self) -> Result<(), Errno> {
            let fd = self.into_raw_fd();
            if unsafe { libc::close(fd) } == 0 {
                Ok(())
            } else {
                Err(errno())
            }
        }
    }
}

impl<E> Failure<E> {
    /// Changes only a present body; the captured canonical report moves intact.
    pub fn map_body<U>(self, map: impl FnOnce(E) -> U) -> Failure<U> {
        Failure {
            body: self.body.map(map),
            report: self.report,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, convert::Infallible, rc::Rc};

    struct Counted {
        calls: Rc<Cell<usize>>,
        result: Result<(), Errno>,
        value: usize,
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
            value: 0,
        }
    }

    struct Body(Box<str>);
    struct MappedBody(Box<str>);

    #[test]
    fn body_error_stays_primary_after_one_unknown_close() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(counted(&calls, Err(Errno::Interrupted)), Role::Source);
        let body = Body("original body".to_owned().into_boxed_str());
        let body_address = body.0.as_ptr();

        assert!(!owner.finish());
        let failure = match attempt.finish(Err::<(), _>(body)) {
            Err(failure) => failure,
            Ok(_) => panic!("the body error must remain a failure"),
        };
        assert_eq!(failure.body().unwrap().0.as_ptr(), body_address);
        assert_eq!(failure.report().attempted(), 1);
        assert_eq!(failure.report().closed(), 0);
        assert_eq!(failure.report().unknown(), 1);
        assert_eq!(failure.report().live(), 0);
        assert_eq!(
            failure.report().first(),
            Some(Issue::Close {
                role: Role::Source,
                errno: Errno::Interrupted,
                state: Knowledge::Unknown,
            })
        );
        drop(failure);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn close_only_failure_has_no_body_and_never_retries() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(counted(&calls, Err(Errno::Io)), Role::Child);
        assert!(!owner.finish());
        let failure = match attempt.finish(Ok::<_, Infallible>("suppressed value")) {
            Err(failure) => failure,
            Ok(_) => panic!("an unknown close must suppress success"),
        };
        assert!(failure.body().is_none());
        assert_eq!(failure.report().attempted(), 1);
        assert_eq!(failure.report().unknown(), 1);
        assert_eq!(failure.report().closed(), 0);
        assert_eq!(failure.report().live(), 0);
        assert_eq!(
            failure.report().first(),
            Some(Issue::Close {
                role: Role::Child,
                errno: Errno::Io,
                state: Knowledge::Unknown,
            })
        );
        drop(failure);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn live_owner_blocks_terminal_finish_and_drop_settles_once() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let owner = attempt.adopt(counted(&calls, Ok(())), Role::Ancestor);
        let failure = match attempt.clone().finish(Ok::<_, Infallible>(())) {
            Err(failure) => failure,
            Ok(()) => panic!("a live owner must block terminal success"),
        };
        assert!(failure.body().is_none());
        assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::LiveOwners)));
        assert_eq!(failure.report().live(), 1);
        assert_eq!(failure.report().attempted(), 0);
        assert_eq!(calls.get(), 0);

        drop(owner);
        let after_drop = attempt.report();
        assert_eq!(after_drop.first(), Some(Issue::Gap(Gap::LiveOwners)));
        assert_eq!(after_drop.live(), 0);
        assert_eq!(after_drop.attempted(), 1);
        assert_eq!(after_drop.closed(), 1);
        assert_eq!(after_drop.unknown(), 0);
        assert_eq!(calls.get(), 1);
        assert_eq!(failure.report().live(), 1);
        assert_eq!(failure.report().attempted(), 0);
        assert!(!attempt.check());
    }

    #[test]
    fn owner_accessors_keep_the_resource_and_actual_scope() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let mut owner = attempt.adopt(counted(&calls, Ok(())), Role::Current);
        assert!(owner.attempt().same_scope(&attempt));
        assert_eq!(owner.get().value, 0);
        owner.get_mut().value = 7;
        assert_eq!(owner.get().value, 7);
        assert_eq!(attempt.report().live(), 1);
        assert!(owner.finish());
        assert_eq!(calls.get(), 1);
        assert_eq!(attempt.report().closed(), 1);
        assert!(attempt.finish(Ok::<_, Infallible>(())).is_ok());
    }

    #[test]
    fn map_body_moves_non_debug_body_and_report_and_skips_none() {
        let attempt = Attempt::default();
        attempt.gap(Gap::Unbound);
        attempt.gap(Gap::TerminalPublication);
        let body = Body("private body text".to_owned().into_boxed_str());
        let body_address = body.0.as_ptr();
        let failure = attempt.failure(Some(body));
        let report_address = failure.report().secondary().as_ptr();
        let debug = format!("{failure:?}");
        assert!(debug.contains("body_present: true"));
        assert!(!debug.contains("private body text"));
        attempt.gap(Gap::RawDailyClose);
        let calls = Cell::new(0);
        let mapped = failure.map_body(|body| {
            calls.set(calls.get() + 1);
            MappedBody(body.0)
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(mapped.body().unwrap().0.as_ptr(), body_address);
        assert_eq!(mapped.report().secondary().as_ptr(), report_address);
        assert_eq!(mapped.report().first(), Some(Issue::Gap(Gap::Unbound)));
        assert_eq!(
            mapped.report().secondary(),
            &[Issue::Gap(Gap::TerminalPublication)]
        );
        assert_eq!(mapped.report().attempted(), 0);
        assert_eq!(mapped.report().omitted(), 0);
        assert_eq!(mapped.report().live(), 0);
        let (body, report) = mapped.into_parts();
        assert_eq!(body.unwrap().0.as_ptr(), body_address);
        assert_eq!(report.secondary().as_ptr(), report_address);

        let bodyless = Attempt::default();
        bodyless.gap(Gap::NonLinux);
        bodyless.gap(Gap::DatabaseClose);
        let failure = bodyless.failure::<Body>(None);
        let report_address = failure.report().secondary().as_ptr();
        let calls = Cell::new(0);
        let mapped = failure.map_body(|body| {
            calls.set(calls.get() + 1);
            MappedBody(body.0)
        });
        assert_eq!(calls.get(), 0);
        assert!(mapped.body().is_none());
        assert_eq!(mapped.report().first(), Some(Issue::Gap(Gap::NonLinux)));
        assert_eq!(
            mapped.report().secondary(),
            &[Issue::Gap(Gap::DatabaseClose)]
        );
        assert_eq!(mapped.report().secondary().as_ptr(), report_address);
        assert!(!format!("{mapped:?}").contains("private body text"));
    }

    #[test]
    fn issue_limit_keeps_first_and_counts_omitted_diagnostics() {
        let attempt = Attempt::default();
        attempt.gap(Gap::Unbound);
        for _ in 0..259 {
            attempt.gap(Gap::TerminalPublication);
        }
        let report = attempt.report();
        assert_eq!(report.first(), Some(Issue::Gap(Gap::Unbound)));
        assert_eq!(report.secondary().len(), 255);
        assert_eq!(report.omitted(), 4);
        assert_eq!(report.attempted(), 0);
        assert_eq!(report.live(), 0);
        assert!(!attempt.check());
    }

    #[test]
    fn failure_and_finished_scope_cannot_return_later_success() {
        let attempt = Attempt::default();
        let failure = attempt.failure::<Infallible>(None);
        assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::Abandoned)));
        let failure = match attempt.finish(Ok::<_, Infallible>(())) {
            Err(failure) => failure,
            Ok(()) => panic!("a recorded failure must remain permanent"),
        };
        assert!(failure.body().is_none());
        assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::Abandoned)));

        let finished = Attempt::default();
        assert!(finished.clone().finish(Ok::<_, Infallible>(())).is_ok());
        let failure = match finished.finish(Ok::<_, Infallible>(())) {
            Err(failure) => failure,
            Ok(()) => panic!("terminal settlement is single-use"),
        };
        assert_eq!(failure.report().first(), Some(Issue::Gap(Gap::Reuse)));
        assert!(failure.body().is_none());
    }
}
