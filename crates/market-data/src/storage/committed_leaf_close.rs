//! Private leaf, lock, and manifest completion for the checked reader.
//! Cleanup retains whole failures and uses the caller's single borrowed attempt.

use super::committed_parent_close::{CheckedCommittedDirectory, CheckedCommittedDirectoryPhase};
use super::{File, PathBuf, StoreError, io_err, validate_committed_manifest_evidence_node};
use crate::checked_close::{Attempt, Close, Failure, Gap, Owner, Report, Role};
use crate::raw_custody::RawInvocation;
use std::convert::Infallible;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedLeafKind {
    CommitLock,
    Manifest,
}
impl CheckedCommittedLeafKind {
    fn name(self) -> &'static str {
        match self {
            Self::CommitLock => "commit.lock",
            Self::Manifest => "manifest.jsonl",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedLeafPhase {
    Scope,
    Binding,
    Execution,
    Open,
    Clone,
    Validate,
    Body,
    Close,
}

pub struct CheckedCommittedLeafFailure<E = StoreError> {
    failure: Failure<E>,
    kind: CheckedCommittedLeafKind,
    phase: CheckedCommittedLeafPhase,
    // Genuine producer identity, NOT a Report/path/counter/UID authority.
    source_attempt: Attempt,
    admission: Option<LeafAdmissionFailure>,
}
impl<E> CheckedCommittedLeafFailure<E> {
    fn new(
        failure: Failure<E>,
        kind: CheckedCommittedLeafKind,
        phase: CheckedCommittedLeafPhase,
        source: &Attempt,
    ) -> Self {
        Self {
            failure,
            kind,
            phase,
            source_attempt: source.clone(),
            admission: None,
        }
    }
    pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.admission.as_ref().and_then(|a| a.execution.as_ref())
    }
    pub fn source_report(&self) -> Option<&Report> {
        match &self.admission {
            Some(a) => a.source_report(),
            None => Some(self.failure.report()),
        }
    }
    pub fn body(&self) -> Option<&E> {
        self.failure.body()
    }
    pub fn report(&self) -> &Report {
        self.failure.report()
    }
    pub fn kind(&self) -> CheckedCommittedLeafKind {
        self.kind
    }
    pub fn phase(&self) -> CheckedCommittedLeafPhase {
        self.phase
    }
    pub fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommittedLeafFailure<U> {
        CheckedCommittedLeafFailure {
            failure: self.failure.map_body(map),
            kind: self.kind,
            phase: self.phase,
            source_attempt: self.source_attempt,
            admission: self.admission,
        }
    }
    pub fn into_parts(
        self,
    ) -> (
        Option<E>,
        Report,
        CheckedCommittedLeafKind,
        CheckedCommittedLeafPhase,
    ) {
        let (body, report) = self.failure.into_parts();
        (body, report, self.kind, self.phase)
    }
}
impl<E> std::fmt::Debug for CheckedCommittedLeafFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedLeafFailure")
            .field("body_present", &self.body().is_some())
            .field("report", self.report())
            .field("kind", &self.kind)
            .field("phase", &self.phase)
            .finish()
    }
}

fn poison_foreign_scopes(actual: &Attempt, expected: &Attempt, owners: &[&Attempt]) {
    if !actual.same_scope(expected) || owners.iter().any(|owner| !actual.same_scope(owner)) {
        actual.gap(Gap::ScopeMismatch);
        expected.gap(Gap::ScopeMismatch);
        for owner in owners {
            owner.gap(Gap::ScopeMismatch);
        }
    }
}

// Generic PRIVATE production graph; R=File in the only concrete producer.
// Counted-Close models may exercise it but cannot create a native leaf guard.
struct CommittedLeafGraph<'invocation, 'attempt, 'expected, R: Close> {
    leaf: Option<Owner<R>>,
    parent_clone: Option<Owner<R>>,
    invocation: &'invocation RawInvocation<'attempt>,
    expected: &'expected Attempt,
    kind: CheckedCommittedLeafKind,
}
impl<'invocation, 'attempt, 'expected, R: Close>
    CommittedLeafGraph<'invocation, 'attempt, 'expected, R>
{
    fn new(
        invocation: &'invocation RawInvocation<'attempt>,
        expected: &'expected Attempt,
        kind: CheckedCommittedLeafKind,
    ) -> Self {
        Self {
            leaf: None,
            parent_clone: None,
            invocation,
            expected,
            kind,
        }
    }
    fn scope<E>(&self) -> Result<(), Failure<E>> {
        let actual = self.invocation.attempt();
        if let Some(owner) = self.leaf.as_ref() {
            poison_foreign_scopes(actual, self.expected, &[owner.attempt()]);
        }
        if let Some(owner) = self.parent_clone.as_ref() {
            poison_foreign_scopes(actual, self.expected, &[owner.attempt()]);
        }
        self.invocation
            .settle_local_in(self.expected, Ok::<(), E>(()))
    }
    fn drain(&mut self) {
        // Explicit new cleanup order: leaf BEFORE duplicated parent.
        // Owner::finish's bool is ledger cleanliness, NOT a native verdict.
        if let Some(owner) = self.leaf.take() {
            owner.finish();
        }
        if let Some(owner) = self.parent_clone.take() {
            owner.finish();
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn fail<T, E>(
        &mut self,
        body: Option<E>,
        phase: CheckedCommittedLeafPhase,
    ) -> Result<T, CheckedCommittedLeafFailure<E>> {
        self.drain();
        let settled = match body {
            Some(body) => self
                .invocation
                .settle_local_in(self.expected, Err::<(), E>(body)),
            None => self
                .invocation
                .settle_local_in(self.expected, Ok::<(), E>(())),
        };
        let failure = match settled {
            Err(failure) => failure,
            Ok(()) => self.invocation.refused(Gap::RawDailyClose),
        };
        Err(CheckedCommittedLeafFailure::new(
            failure,
            self.kind,
            phase,
            self.invocation.attempt(),
        ))
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn acquire<E>(
        &mut self,
        acquire: impl FnOnce() -> Result<R, E>,
        duplicate: impl FnOnce() -> Result<R, E>,
        validate: impl FnOnce(&R, &R) -> Result<(), E>,
    ) -> Result<(), CheckedCommittedLeafFailure<E>> {
        assert!(self.leaf.is_none() && self.parent_clone.is_none());
        if self.scope::<E>().is_err() {
            return self.fail(None, CheckedCommittedLeafPhase::Scope);
        }
        let leaf = match acquire() {
            Ok(leaf) => leaf,
            Err(body) => return self.fail(Some(body), CheckedCommittedLeafPhase::Open),
        };
        self.leaf = Some(self.invocation.attempt().adopt(leaf, Role::Child));
        if !self.invocation.attempt().check() {
            return self.fail(None, CheckedCommittedLeafPhase::Scope);
        }
        let parent = match duplicate() {
            Ok(parent) => parent,
            Err(body) => return self.fail(Some(body), CheckedCommittedLeafPhase::Clone),
        };
        self.parent_clone = Some(self.invocation.attempt().adopt(parent, Role::Clone));
        if !self.invocation.attempt().check() {
            return self.fail(None, CheckedCommittedLeafPhase::Scope);
        }
        self.validate(validate)
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn validate<E>(
        &mut self,
        validate: impl FnOnce(&R, &R) -> Result<(), E>,
    ) -> Result<(), CheckedCommittedLeafFailure<E>> {
        if self.scope::<E>().is_err() {
            return self.fail(None, CheckedCommittedLeafPhase::Scope);
        }
        if let Err(body) = validate(
            self.parent_clone
                .as_ref()
                .expect("actual parent clone")
                .get(),
            self.leaf.as_ref().expect("actual leaf").get(),
        ) {
            return self.fail(Some(body), CheckedCommittedLeafPhase::Validate);
        }
        if !self.invocation.attempt().check() {
            return self.fail(None, CheckedCommittedLeafPhase::Scope);
        }
        Ok(())
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn finish<T, E>(mut self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>> {
        let scope_failed = self.scope::<E>().is_err();
        let phase = if body.is_err() {
            CheckedCommittedLeafPhase::Body
        } else if scope_failed {
            CheckedCommittedLeafPhase::Scope
        } else {
            CheckedCommittedLeafPhase::Close
        };
        self.drain();
        self.invocation
            .settle_local_in(self.expected, body)
            .map_err(|failure| {
                CheckedCommittedLeafFailure::new(
                    failure,
                    self.kind,
                    phase,
                    self.invocation.attempt(),
                )
            })
    }
}
impl<R: Close> Drop for CommittedLeafGraph<'_, '_, '_, R> {
    fn drop(&mut self) {
        if self.leaf.is_some() || self.parent_clone.is_some() {
            self.invocation.attempt().gap(Gap::Abandoned);
            self.drain();
        }
    }
}

// Whole per-call admission; the initial scope result stays before context checks.
struct LeafAdmissionFailure {
    phase: CheckedCommittedLeafPhase,
    original: Result<(), Failure<Infallible>>,
    execution: Option<crate::ExecutionGateFailure>,
}
impl LeafAdmissionFailure {
    fn source_report(&self) -> Option<&Report> {
        match &self.original {
            Err(first) => Some(first.report()),
            Ok(()) => self
                .execution
                .as_ref()
                .and_then(crate::ExecutionGateFailure::source_report),
        }
    }
}
fn parent_preflight(
    parent: &CheckedCommittedDirectory<'_, '_, '_>,
) -> Option<LeafAdmissionFailure> {
    let invocation = parent.invocation();
    poison_foreign_scopes(
        invocation.attempt(),
        parent.expected_attempt(),
        &[parent.held_file().attempt()],
    );
    let original = invocation.settle_local_in(parent.expected_attempt(), Ok::<(), Infallible>(()));
    let execution = invocation
        .compare_native_context_scopes(parent.expected_attempt())
        .err();
    if original.is_err() || execution.is_some() {
        let phase = if original.is_err() {
            CheckedCommittedLeafPhase::Scope
        } else {
            CheckedCommittedLeafPhase::Execution
        };
        return Some(LeafAdmissionFailure {
            phase,
            original,
            execution,
        });
    }
    match crate::close_gate::require_native(invocation, parent.expected_attempt()) {
        Ok(()) => None,
        Err(crate::close_gate::NativeAdmissionFailure::Unbound) => Some(LeafAdmissionFailure {
            phase: CheckedCommittedLeafPhase::Binding,
            original: Err(invocation.refused(Gap::Unbound)),
            execution: None,
        }),
        Err(crate::close_gate::NativeAdmissionFailure::Execution(failure)) => {
            Some(LeafAdmissionFailure {
                phase: CheckedCommittedLeafPhase::Execution,
                original,
                execution: Some(failure),
            })
        }
    }
}

pub(super) struct CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected> {
    parent: &'parent CheckedCommittedDirectory<'invocation, 'attempt, 'expected>,
    graph: Option<CommittedLeafGraph<'invocation, 'attempt, 'expected, File>>,
    path: PathBuf,
}
impl<'invocation, 'attempt, 'expected> CheckedCommittedDirectory<'invocation, 'attempt, 'expected> {
    #[expect(
        clippy::result_large_err,
        reason = "The leaf failure owns the original body, phase, report and source identity without a new boxing allocation."
    )]
    pub(super) fn open_leaf_checked<'parent>(
        &'parent self,
        kind: CheckedCommittedLeafKind,
    ) -> Result<
        CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>,
        CheckedCommittedLeafFailure,
    > {
        use rustix::fs::{FileType, Mode, OFlags, openat};
        use rustix::io::Errno;
        let mut graph =
            CommittedLeafGraph::<File>::new(self.invocation(), self.expected_attempt(), kind);
        if let Some(admission) = parent_preflight(self) {
            let phase = admission.phase;
            let mut failure = graph
                .fail::<(), StoreError>(None, phase)
                .expect_err("actual admission failure");
            failure.admission = Some(admission);
            return Err(failure);
        }
        let path = self.path().join(kind.name());
        let context = self.input_context();
        graph.acquire(
            || {
                let descriptor = openat(
                    self.held_file().get(),
                    kind.name(),
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map_err(|error| {
                    if error == Errno::LOOP {
                        StoreError::UnsafePath {
                            path: path.display().to_string(),
                            reason: "committed Raw control files must not be symlink redirects"
                                .to_owned(),
                        }
                    } else {
                        io_err(
                            &format!(
                                "open existing committed Raw control file {}",
                                path.display()
                            ),
                            error.into(),
                        )
                    }
                })?;
                Ok::<File, StoreError>(File::from(descriptor))
            },
            || {
                self.held_file()
                    .get()
                    .try_clone()
                    .map_err(|error| io_err("duplicate committed Raw control directory", error))
            },
            |clone, leaf| {
                validate_committed_manifest_evidence_node(
                    &path,
                    clone,
                    std::ffi::OsStr::new(kind.name()),
                    leaf,
                    FileType::RegularFile,
                    Some(context.trusted_raw_owner_uid),
                )
            },
        )?;
        Ok(CheckedCommittedLeaf {
            parent: self,
            graph: Some(graph),
            path,
        })
    }
}
impl<'parent, 'invocation, 'attempt, 'expected>
    CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>
{
    fn graph(&self) -> &CommittedLeafGraph<'invocation, 'attempt, 'expected, File> {
        self.graph
            .as_ref()
            .expect("graph held until consuming finish")
    }
    fn held_file(&self) -> &File {
        self.graph()
            .leaf
            .as_ref()
            .expect("leaf held until finish")
            .get()
    }
    fn kind(&self) -> CheckedCommittedLeafKind {
        self.graph().kind
    }
    /// T is ordinary data, never proof of unrelated caller resources.
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn finish<T, E>(mut self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>> {
        self.graph.take().expect("one owned graph").finish(body)
    }
}

// Explicit Drop keeps the real parent borrow in drop checking through drain.
// The graph marks Abandoned and closes leaf then clone; never this parent.
impl Drop for CheckedCommittedLeaf<'_, '_, '_, '_> {
    fn drop(&mut self) {
        drop(self.graph.take());
    }
}

pub struct CheckedCommittedLeafParentFailure<E = StoreError> {
    source: CheckedCommittedLeafFailure<E>,
    post_parent_report: Report,
    parent_phase: Option<CheckedCommittedDirectoryPhase>,
    parent_component_index: Option<usize>,
}
impl<E> CheckedCommittedLeafParentFailure<E> {
    pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.source.execution_failure()
    }
    pub fn body(&self) -> Option<&E> {
        self.source.body()
    }
    pub fn source(&self) -> &CheckedCommittedLeafFailure<E> {
        &self.source
    }
    pub fn post_parent_report(&self) -> &Report {
        &self.post_parent_report
    }
    pub fn parent_phase(&self) -> Option<CheckedCommittedDirectoryPhase> {
        self.parent_phase
    }
    pub fn parent_component_index(&self) -> Option<usize> {
        self.parent_component_index
    }
    pub fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommittedLeafParentFailure<U> {
        CheckedCommittedLeafParentFailure {
            source: self.source.map_body(map),
            post_parent_report: self.post_parent_report,
            parent_phase: self.parent_phase,
            parent_component_index: self.parent_component_index,
        }
    }
    pub fn into_parts(
        self,
    ) -> (
        CheckedCommittedLeafFailure<E>,
        Report,
        Option<CheckedCommittedDirectoryPhase>,
        Option<usize>,
    ) {
        (
            self.source,
            self.post_parent_report,
            self.parent_phase,
            self.parent_component_index,
        )
    }
}
impl<E> std::fmt::Debug for CheckedCommittedLeafParentFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedLeafParentFailure")
            .field("source", &self.source)
            .field("post_parent_report", &self.post_parent_report)
            .field("parent_phase", &self.parent_phase)
            .field("parent_component_index", &self.parent_component_index)
            .finish()
    }
}
// Pure DATA seam BELOW genuine parent extraction; no Report authenticates identity.
fn post_parent_parts<E>(
    source: CheckedCommittedLeafFailure<E>,
    post_parent_report: Report,
    parent_phase: Option<CheckedCommittedDirectoryPhase>,
    parent_component_index: Option<usize>,
) -> CheckedCommittedLeafParentFailure<E> {
    CheckedCommittedLeafParentFailure {
        source,
        post_parent_report,
        parent_phase,
        parent_component_index,
    }
}
fn ensure_failed_before_parent_cleanup(actual: &Attempt) {
    if actual.check() {
        actual.gap(Gap::RawDailyClose);
    }
}
fn unexpected_parent_success(actual: &Attempt) -> Report {
    actual.gap(Gap::RawDailyClose);
    actual.report()
}
// PRIVATE same-enclosing-producer completion, UNWIRED to legacy readers.
pub(super) fn complete_committed_leaf_source_with_parent<E>(
    parent: CheckedCommittedDirectory<'_, '_, '_>,
    source: CheckedCommittedLeafFailure<E>,
) -> CheckedCommittedLeafParentFailure<E> {
    let actual = parent.invocation().attempt().clone();
    poison_foreign_scopes(
        &actual,
        parent.expected_attempt(),
        &[parent.held_file().attempt(), &source.source_attempt],
    );
    ensure_failed_before_parent_cleanup(&actual);
    let (report, phase, index) = match parent.finish(Ok::<(), Infallible>(())) {
        Err(failure) => {
            let (body, report, phase, index) = failure.into_parts();
            match body {
                None => (report, Some(phase), index),
                Some(never) => match never {},
            }
        }
        Ok(()) => (unexpected_parent_success(&actual), None, None),
    };
    post_parent_parts(source, report, phase, index)
}

// HELD-LEAF-CHECKPOINT-S01 BEGIN
/// Actual PRE-CLEANUP snapshot: the borrowed leaf and clone are still held.
/// Stored scope identity is private; Report, path and UID are not authority.
pub(super) struct CheckedCommittedLeafCheckpointFailure<E = StoreError> {
    failure: Failure<E>,
    kind: CheckedCommittedLeafKind,
    phase: CheckedCommittedLeafPhase,
    source_attempt: Attempt,
    admission: Option<LeafAdmissionFailure>,
}

impl<E> CheckedCommittedLeafCheckpointFailure<E> {
    fn new(
        failure: Failure<E>,
        kind: CheckedCommittedLeafKind,
        phase: CheckedCommittedLeafPhase,
        source: &Attempt,
    ) -> Self {
        Self {
            failure,
            kind,
            phase,
            source_attempt: source.clone(),
            admission: None,
        }
    }
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.admission.as_ref().and_then(|a| a.execution.as_ref())
    }
    fn source_report(&self) -> Option<&Report> {
        match &self.admission {
            Some(a) => a.source_report(),
            None => Some(self.failure.report()),
        }
    }
    pub(super) fn body(&self) -> Option<&E> {
        self.failure.body()
    }
    pub(super) fn report(&self) -> &Report {
        self.failure.report()
    }
    pub(super) fn phase(&self) -> CheckedCommittedLeafPhase {
        self.phase
    }
    pub(super) fn map_body<U>(
        self,
        map: impl FnOnce(E) -> U,
    ) -> CheckedCommittedLeafCheckpointFailure<U> {
        CheckedCommittedLeafCheckpointFailure {
            failure: self.failure.map_body(map),
            kind: self.kind,
            phase: self.phase,
            source_attempt: self.source_attempt,
            admission: self.admission,
        }
    }
}

impl<E> std::fmt::Debug for CheckedCommittedLeafCheckpointFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedLeafCheckpointFailure")
            .field("body_present", &self.body().is_some())
            .field("report", self.report())
            .field("kind", &self.kind)
            .field("phase", &self.phase)
            .finish()
    }
}

// One-shot INTERNAL unwind bookkeeping only. It never owns/drains resources
// and its bit is not supplied by a caller or used as native authority.
struct CheckpointUnwind<'scope> {
    attempt: &'scope Attempt,
    armed: bool,
}
impl<'scope> CheckpointUnwind<'scope> {
    fn new(attempt: &'scope Attempt) -> Self {
        Self {
            attempt,
            armed: true,
        }
    }
    fn disarm(&mut self) {
        self.armed = false;
    }
}
impl Drop for CheckpointUnwind<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.attempt.gap(Gap::Abandoned);
        }
    }
}

impl<R: Close> CommittedLeafGraph<'_, '_, '_, R> {
    // Only concrete parent_preflight supplies this ordinary Scope/Binding
    // phase. Even after that failure, scope() checks BOTH real Owner slots.
    fn checkpoint_refusal<E>(
        &self,
        phase: CheckedCommittedLeafPhase,
    ) -> CheckedCommittedLeafCheckpointFailure<E> {
        let mut unwind = CheckpointUnwind::new(self.invocation.attempt());
        let failure = match self.scope::<E>() {
            Err(failure) => failure,
            Ok(()) => self.invocation.refused(Gap::RawDailyClose),
        };
        let failure = CheckedCommittedLeafCheckpointFailure::new(
            failure,
            self.kind,
            phase,
            self.invocation.attempt(),
        );
        unwind.disarm();
        failure
    }

    // The ONE non-draining production validation seam, also used by Source
    // counted graphs. R=File only in the actual concrete guard method.
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn checkpoint_borrowed<E>(
        &self,
        validate: impl FnOnce(&R, &R) -> Result<(), E>,
    ) -> Result<(), CheckedCommittedLeafCheckpointFailure<E>> {
        let mut unwind = CheckpointUnwind::new(self.invocation.attempt());
        let result = (|| {
            self.scope::<E>().map_err(|failure| {
                CheckedCommittedLeafCheckpointFailure::new(
                    failure,
                    self.kind,
                    CheckedCommittedLeafPhase::Scope,
                    self.invocation.attempt(),
                )
            })?;
            let body = validate(
                self.parent_clone
                    .as_ref()
                    .expect("actual held parent clone")
                    .get(),
                self.leaf.as_ref().expect("actual held leaf").get(),
            );
            let phase = if body.is_err() {
                CheckedCommittedLeafPhase::Validate
            } else {
                CheckedCommittedLeafPhase::Scope
            };
            self.invocation
                .settle_local_in(self.expected, body)
                .map_err(|failure| {
                    CheckedCommittedLeafCheckpointFailure::new(
                        failure,
                        self.kind,
                        phase,
                        self.invocation.attempt(),
                    )
                })
        })();
        unwind.disarm();
        result
    }
}

impl<'parent, 'invocation, 'attempt, 'expected>
    CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>
{
    #[expect(
        clippy::result_large_err,
        reason = "The borrowed checkpoint keeps its whole pre-cleanup failure while the same owners remain live."
    )]
    pub(super) fn checkpoint_borrowed(&self) -> Result<(), CheckedCommittedLeafCheckpointFailure> {
        let graph = self.graph();
        // Compare the real retained parent against the real graph too;
        // no equal Reports/counters/path strings can establish this scope.
        poison_foreign_scopes(
            graph.invocation.attempt(),
            graph.expected,
            &[
                self.parent.invocation().attempt(),
                self.parent.expected_attempt(),
                self.parent.held_file().attempt(),
                graph.leaf.as_ref().expect("held leaf").attempt(),
                graph.parent_clone.as_ref().expect("held clone").attempt(),
            ],
        );
        if let Some(admission) = parent_preflight(self.parent) {
            let mut failure = graph.checkpoint_refusal(admission.phase);
            failure.admission = Some(admission);
            return Err(failure);
        }
        let trusted_raw_owner_uid = self.parent.input_context().trusted_raw_owner_uid;
        let kind = graph.kind;
        graph.checkpoint_borrowed(|clone, leaf| {
            validate_committed_manifest_evidence_node(
                &self.path,
                clone,
                std::ffi::OsStr::new(kind.name()),
                leaf,
                rustix::fs::FileType::RegularFile,
                Some(trusted_raw_owner_uid),
            )
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitLockState {
    Fresh,
    AcquireInFlight,
    HeldObserved,
    AcquireFailed,
    UnlockAttempted,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockIoSummary {
    Ok,
    Error {
        kind: std::io::ErrorKind,
        raw: Option<i32>,
    },
}
impl LockIoSummary {
    fn observed(result: &std::io::Result<()>) -> Self {
        match result {
            Ok(()) => Self::Ok,
            Err(error) => Self::Error {
                kind: error.kind(),
                raw: error.raw_os_error(),
            },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockObservation {
    NotAttempted,
    AttemptedMissing,
    Observed(LockIoSummary),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LockJournalSnapshot {
    acquire: LockObservation,
    unlock: LockObservation,
    poisoned: bool,
}

// Diagnostic ONLY, no File/Attempt or result injection API. Retain before acquire.
#[derive(Clone)]
struct CommitLockObserver(std::sync::Arc<std::sync::Mutex<LockJournalSnapshot>>);
impl CommitLockObserver {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(
            LockJournalSnapshot {
                acquire: LockObservation::NotAttempted,
                unlock: LockObservation::NotAttempted,
                poisoned: false,
            },
        )))
    }
    fn journal(&self) -> std::sync::MutexGuard<'_, LockJournalSnapshot> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poison) => {
                let mut state = poison.into_inner();
                state.poisoned = true;
                state
            }
        }
    }
    fn snapshot(&self) -> LockJournalSnapshot {
        *self.journal()
    }
    fn begin_acquire(&self) -> bool {
        let mut state = self.journal();
        if state.poisoned || state.acquire != LockObservation::NotAttempted {
            state.poisoned = true;
            return false;
        }
        state.acquire = LockObservation::AttemptedMissing;
        true
    }
    fn acquired(&self, result: &std::io::Result<()>) -> bool {
        let observed = LockIoSummary::observed(result);
        let mut state = self.journal();
        state.acquire = LockObservation::Observed(observed);
        state.poisoned
    }
    // Cleanup is mandatory even when this diagnostic mutex is poisoned.
    fn begin_unlock(&self) {
        let mut state = self.journal();
        if state.unlock != LockObservation::NotAttempted {
            state.poisoned = true;
        }
        state.unlock = LockObservation::AttemptedMissing;
    }
    fn unlocked(&self, result: &std::io::Result<()>) -> bool {
        let observed = LockIoSummary::observed(result);
        let mut state = self.journal();
        state.unlock = LockObservation::Observed(observed);
        state.poisoned
    }
}
impl std::fmt::Debug for CommitLockObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.snapshot(), f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitLockSourcePhase {
    State,
    Acquire,
    Unlock,
    Scope,
    Checkpoint(CheckedCommittedLeafPhase),
    Ancestry {
        phase: CheckedCommittedDirectoryPhase,
        component_index: Option<usize>,
    },
}
enum CommitLockSource<E> {
    Checkpoint(CheckedCommittedLeafCheckpointFailure<E>),
    Ancestry {
        failure: ActualParentFailure<E>,
        source_attempt: Attempt,
    },
    Canonical {
        failure: Failure<E>,
        source_attempt: Attempt,
        phase: CommitLockSourcePhase,
    },
}
impl<E> CommitLockSource<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match self {
            Self::Checkpoint(f) => f.execution_failure(),
            Self::Ancestry { failure, .. } => failure.execution_failure(),
            Self::Canonical { .. } => None,
        }
    }
    fn source_report(&self) -> Option<&Report> {
        match self {
            Self::Checkpoint(f) => f.source_report(),
            Self::Ancestry { failure, .. } => failure.source_report(),
            Self::Canonical { failure, .. } => Some(failure.report()),
        }
    }
    // Actual source token, including defensive foreign-expected native Some.
    fn canonical(actual: &Attempt, body: Option<E>, phase: CommitLockSourcePhase) -> Self {
        Self::Canonical {
            failure: actual.failure(body),
            source_attempt: actual.clone(),
            phase,
        }
    }
    fn body(&self) -> Option<&E> {
        match self {
            Self::Checkpoint(f) => f.body(),
            Self::Ancestry { failure, .. } => failure.body(),
            Self::Canonical { failure, .. } => failure.body(),
        }
    }
    fn report(&self) -> &Report {
        match self {
            Self::Checkpoint(f) => f.report(),
            Self::Ancestry { failure, .. } => failure.report(),
            Self::Canonical { failure, .. } => failure.report(),
        }
    }
    fn source_attempt(&self) -> &Attempt {
        match self {
            Self::Checkpoint(f) => &f.source_attempt,
            Self::Ancestry { source_attempt, .. } => source_attempt,
            Self::Canonical { source_attempt, .. } => source_attempt,
        }
    }
    fn phase(&self) -> CommitLockSourcePhase {
        match self {
            Self::Checkpoint(f) => CommitLockSourcePhase::Checkpoint(f.phase()),
            Self::Ancestry { failure, .. } => CommitLockSourcePhase::Ancestry {
                phase: failure.phase(),
                component_index: failure.component_index(),
            },
            Self::Canonical { phase, .. } => *phase,
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CommitLockSource<U> {
        match self {
            Self::Checkpoint(f) => CommitLockSource::Checkpoint(f.map_body(map)),
            Self::Ancestry {
                failure,
                source_attempt,
            } => CommitLockSource::Ancestry {
                failure: failure.map_body(map),
                source_attempt,
            },
            Self::Canonical {
                failure,
                source_attempt,
                phase,
            } => CommitLockSource::Canonical {
                failure: failure.map_body(map),
                source_attempt,
                phase,
            },
        }
    }
}
impl<E> std::fmt::Debug for CommitLockSource<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommitLockSource")
            .field("body_present", &self.body().is_some())
            .field("report", self.report())
            .field("phase", &self.phase())
            .finish()
    }
}

// Actual same-module completion seam. Only the File implementation takes a
// genuine opaque native leaf; the model implementation owns counted graphs.
trait CommitLockLeaf: Sized {
    fn attempt(&self) -> &Attempt;
    fn kind(&self) -> CheckedCommittedLeafKind;
    fn poison_scopes(&self, source: &Attempt);
    #[expect(
        clippy::result_large_err,
        reason = "The trait returns the actual whole checkpoint failure rather than detaching its source report."
    )]
    fn checkpoint(&self) -> Result<(), CheckedCommittedLeafCheckpointFailure>;
    fn lock_shared(&self) -> std::io::Result<()>;
    fn unlock(&self) -> std::io::Result<()>;
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn finish<T, E>(self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>>;
}
impl CommitLockLeaf for CheckedCommittedLeaf<'_, '_, '_, '_> {
    fn attempt(&self) -> &Attempt {
        self.graph().invocation.attempt()
    }
    fn kind(&self) -> CheckedCommittedLeafKind {
        self.kind()
    }
    fn poison_scopes(&self, source: &Attempt) {
        let graph = self.graph();
        poison_foreign_scopes(
            graph.invocation.attempt(),
            graph.expected,
            &[
                source,
                self.parent.invocation().attempt(),
                self.parent.expected_attempt(),
                self.parent.held_file().attempt(),
                graph.leaf.as_ref().expect("held CommitLock leaf").attempt(),
                graph
                    .parent_clone
                    .as_ref()
                    .expect("held CommitLock clone")
                    .attempt(),
            ],
        );
    }
    fn checkpoint(&self) -> Result<(), CheckedCommittedLeafCheckpointFailure> {
        self.checkpoint_borrowed()
    }
    fn lock_shared(&self) -> std::io::Result<()> {
        fs2::FileExt::lock_shared(self.held_file())
    }
    fn unlock(&self) -> std::io::Result<()> {
        fs2::FileExt::unlock(self.held_file())
    }
    fn finish<T, E>(self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>> {
        self.finish(body)
    }
}

enum PostSourceLeaf {
    Failed(CheckedCommittedLeafFailure<Infallible>),
    UnexpectedSuccess(Report),
}
impl PostSourceLeaf {
    fn report(&self) -> &Report {
        match self {
            Self::Failed(f) => f.report(),
            Self::UnexpectedSuccess(report) => report,
        }
    }
    fn phase(&self) -> Option<CheckedCommittedLeafPhase> {
        match self {
            Self::Failed(f) => Some(f.phase()),
            Self::UnexpectedSuccess(_) => None,
        }
    }
}
enum CommitLockFailureChain<E> {
    Source {
        source: CommitLockSource<E>,
        post_leaf: PostSourceLeaf,
    },
    Leaf(CheckedCommittedLeafFailure<E>),
}
struct CheckedCommitLockFinishFailure<E = StoreError> {
    chain: CommitLockFailureChain<E>,
    post_unlock_report: Report,
    secondary_unlock: Option<std::io::Error>,
    observer: CommitLockObserver,
    source_attempt: Attempt,
}
impl<E> CheckedCommitLockFinishFailure<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match &self.chain {
            CommitLockFailureChain::Source { source, .. } => source.execution_failure(),
            CommitLockFailureChain::Leaf(f) => f.execution_failure(),
        }
    }
    fn source_report(&self) -> Option<&Report> {
        match &self.chain {
            CommitLockFailureChain::Source { source, .. } => source.source_report(),
            CommitLockFailureChain::Leaf(f) => f.source_report(),
        }
    }
    fn body(&self) -> Option<&E> {
        match &self.chain {
            CommitLockFailureChain::Source { source, .. } => source.body(),
            CommitLockFailureChain::Leaf(f) => f.body(),
        }
    }
    fn report(&self) -> &Report {
        match &self.chain {
            CommitLockFailureChain::Source { source, .. } => source.report(),
            CommitLockFailureChain::Leaf(f) => f.report(),
        }
    }
    fn post_leaf_report(&self) -> &Report {
        match &self.chain {
            CommitLockFailureChain::Source { post_leaf, .. } => post_leaf.report(),
            CommitLockFailureChain::Leaf(f) => f.report(),
        }
    }
    fn post_leaf_phase(&self) -> Option<CheckedCommittedLeafPhase> {
        match &self.chain {
            CommitLockFailureChain::Source { post_leaf, .. } => post_leaf.phase(),
            CommitLockFailureChain::Leaf(f) => Some(f.phase()),
        }
    }
    fn post_unlock_report(&self) -> &Report {
        &self.post_unlock_report
    }
    fn secondary_unlock_kind(&self) -> Option<std::io::ErrorKind> {
        self.secondary_unlock.as_ref().map(std::io::Error::kind)
    }
    fn secondary_unlock_raw_os_error(&self) -> Option<i32> {
        self.secondary_unlock
            .as_ref()
            .and_then(std::io::Error::raw_os_error)
    }
    // Retains original source tokens until a genuine future parent is compared.
    fn poison_against_parent(&self, parent: &CheckedCommittedDirectory<'_, '_, '_>) {
        let mut owners = vec![parent.held_file().attempt(), &self.source_attempt];
        match &self.chain {
            CommitLockFailureChain::Source { source, post_leaf } => {
                owners.push(source.source_attempt());
                if let PostSourceLeaf::Failed(f) = post_leaf {
                    owners.push(&f.source_attempt);
                }
            }
            CommitLockFailureChain::Leaf(f) => owners.push(&f.source_attempt),
        }
        poison_foreign_scopes(
            parent.invocation().attempt(),
            parent.expected_attempt(),
            &owners,
        );
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommitLockFinishFailure<U> {
        let chain = match self.chain {
            CommitLockFailureChain::Source { source, post_leaf } => {
                CommitLockFailureChain::Source {
                    source: source.map_body(map),
                    post_leaf,
                }
            }
            CommitLockFailureChain::Leaf(f) => CommitLockFailureChain::Leaf(f.map_body(map)),
        };
        CheckedCommitLockFinishFailure {
            chain,
            post_unlock_report: self.post_unlock_report,
            secondary_unlock: self.secondary_unlock,
            observer: self.observer,
            source_attempt: self.source_attempt,
        }
    }
}
impl<E> std::fmt::Debug for CheckedCommitLockFinishFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommitLockFinishFailure")
            .field("body_present", &self.body().is_some())
            .field("report", self.report())
            .field("post_unlock_report", self.post_unlock_report())
            .field("post_leaf_report", self.post_leaf_report())
            .field("post_leaf_phase", &self.post_leaf_phase())
            .field("secondary_unlock_kind", &self.secondary_unlock_kind())
            .field(
                "secondary_unlock_raw_os_error",
                &self.secondary_unlock_raw_os_error(),
            )
            .field("observations", &self.observer)
            .finish()
    }
}

struct CommitLockOwner<L: CommitLockLeaf, E = StoreError> {
    leaf: Option<L>,
    state: CommitLockState,
    source: Option<CommitLockSource<E>>,
    source_attempt: Attempt,
    observer: CommitLockObserver,
    pending_unlock_error: Option<std::io::Error>,
}
impl<L: CommitLockLeaf, E> CommitLockOwner<L, E> {
    // A refused constructor returns the SAME owning object for cleanup; it
    // never treats a Manifest leaf as an acquired CommitLock.
    #[expect(
        clippy::result_large_err,
        reason = "A refused constructor returns the same owning lock object so mandatory cleanup cannot be detached."
    )]
    fn new(leaf: L) -> Result<Self, Self> {
        let actual = leaf.attempt().clone();
        let mut owner = Self {
            leaf: Some(leaf),
            state: CommitLockState::Fresh,
            source: None,
            source_attempt: actual,
            observer: CommitLockObserver::new(),
            pending_unlock_error: None,
        };
        owner.poison_scopes();
        let refusal = if owner.leaf().kind() != CheckedCommittedLeafKind::CommitLock {
            Some(CommitLockSourcePhase::State)
        } else if !owner.source_attempt.check() {
            Some(CommitLockSourcePhase::Scope)
        } else {
            None
        };
        if let Some(phase) = refusal {
            owner.refuse(phase);
            Err(owner)
        } else {
            Ok(owner)
        }
    }
    fn leaf(&self) -> &L {
        self.leaf
            .as_ref()
            .expect("actual leaf held until consuming completion")
    }
    fn poison_scopes(&self) {
        self.leaf().poison_scopes(&self.source_attempt);
        if let Some(source) = &self.source {
            self.leaf().poison_scopes(source.source_attempt());
        }
    }
    fn refuse(&mut self, phase: CommitLockSourcePhase) {
        self.source_attempt.gap(Gap::RawDailyClose);
        if self.source.is_none() {
            self.source = Some(CommitLockSource::canonical(
                &self.source_attempt,
                None,
                phase,
            ));
        }
    }
    fn acquire_with(
        &mut self,
        map_checkpoint: impl FnOnce(StoreError) -> E,
        map_io: impl FnOnce(std::io::Error) -> E,
    ) -> Result<(), ()> {
        self.poison_scopes();
        if self.state != CommitLockState::Fresh
            || self.source.is_some()
            || self.leaf().kind() != CheckedCommittedLeafKind::CommitLock
        {
            self.refuse(CommitLockSourcePhase::State);
            return Err(());
        }
        if let Err(failure) = self.leaf().checkpoint() {
            self.source = Some(CommitLockSource::Checkpoint(
                failure.map_body(map_checkpoint),
            ));
            self.poison_scopes();
            return Err(());
        }
        let actual = self.source_attempt.clone();
        let mut unwind = CheckpointUnwind::new(&actual);
        if !self.observer.begin_acquire() {
            actual.gap(Gap::Abandoned);
            self.refuse(CommitLockSourcePhase::State);
            unwind.disarm();
            return Err(());
        }
        self.state = CommitLockState::AcquireInFlight;
        let result = self.leaf().lock_shared();
        // Independent actual outcome is recorded BEFORE main-ledger handling.
        let poisoned = self.observer.acquired(&result);
        self.state = if result.is_ok() {
            CommitLockState::HeldObserved
        } else {
            CommitLockState::AcquireFailed
        };
        if poisoned {
            actual.gap(Gap::Abandoned);
        }
        let result = match result {
            Err(error) => {
                self.source = Some(CommitLockSource::canonical(
                    &actual,
                    Some(map_io(error)),
                    CommitLockSourcePhase::Acquire,
                ));
                Err(())
            }
            Ok(()) if actual.check() => Ok(()),
            Ok(()) => {
                self.refuse(CommitLockSourcePhase::Scope);
                Err(())
            }
        };
        unwind.disarm();
        result
    }
    // Pure owned-body capture from the genuine producer, no resource factory.
    fn unlock_once(&mut self) {
        if !matches!(
            self.state,
            CommitLockState::HeldObserved | CommitLockState::AcquireInFlight
        ) {
            return;
        }
        let actual = self.source_attempt.clone();
        let mut unwind = CheckpointUnwind::new(&actual);
        if self.state == CommitLockState::AcquireInFlight {
            actual.gap(Gap::Abandoned);
        }
        // Capture real pre-unlock failure before observing any unlock outcome.
        self.poison_scopes();
        if self.source.is_none() && !actual.check() {
            self.source = Some(CommitLockSource::canonical(
                &actual,
                None,
                CommitLockSourcePhase::Scope,
            ));
        }
        // Marks BEFORE effect/journal; even a caught unlock panic is not retried.
        self.state = CommitLockState::UnlockAttempted;
        self.observer.begin_unlock();
        let result = self.leaf().unlock();
        let poisoned = self.observer.unlocked(&result);
        self.pending_unlock_error = result.err();
        if poisoned {
            actual.gap(Gap::Abandoned);
        }
        if self.pending_unlock_error.is_some() {
            actual.gap(Gap::RawDailyClose);
        }
        unwind.disarm();
    }
    #[expect(
        clippy::result_large_err,
        reason = "Preserve the first whole source, typed unlock secondary and separate post-unlock and post-leaf snapshots."
    )]
    fn finish_with<T>(
        mut self,
        body: Result<T, E>,
        map_unlock: impl FnOnce(std::io::Error) -> E,
    ) -> Result<T, CheckedCommitLockFinishFailure<E>> {
        self.poison_scopes();
        if self.source.is_none() && body.is_ok() {
            if !matches!(
                self.state,
                CommitLockState::HeldObserved | CommitLockState::UnlockAttempted
            ) {
                self.refuse(CommitLockSourcePhase::State);
            } else if !self.source_attempt.check() && self.pending_unlock_error.is_none() {
                self.refuse(CommitLockSourcePhase::Scope);
            }
        }
        self.unlock_once(); // NEVER suppressed by a dirty ledger or prior source.
        let secondary_unlock = match self.pending_unlock_error.take() {
            Some(error) if self.source.is_none() && body.is_ok() => {
                self.source = Some(CommitLockSource::canonical(
                    &self.source_attempt,
                    Some(map_unlock(error)),
                    CommitLockSourcePhase::Unlock,
                ));
                None
            }
            error => error,
        };
        if self.observer.snapshot().poisoned {
            self.source_attempt.gap(Gap::Abandoned);
        }
        if self.source.is_none() && body.is_ok() && !self.source_attempt.check() {
            self.refuse(CommitLockSourcePhase::Scope);
        }
        self.poison_scopes(); // Before ANY source extraction or real leaf cleanup.
        if self.source.is_some() && self.source_attempt.check() {
            self.source_attempt.gap(Gap::RawDailyClose);
        }
        let post_unlock_report = self.source_attempt.report();
        let source = self.source.take();
        let leaf = self.leaf.take().expect("one genuine owned leaf");
        self.state = CommitLockState::Finished;
        let source_attempt = self.source_attempt.clone();
        let observer = self.observer.clone();
        if let Some(source) = source {
            drop(body); // Earlier actual Some/None remains primary, never replaced.
            let post_leaf = match leaf.finish(Ok::<(), Infallible>(())) {
                Err(failure) => PostSourceLeaf::Failed(failure),
                Ok(()) => {
                    source_attempt.gap(Gap::RawDailyClose);
                    PostSourceLeaf::UnexpectedSuccess(source_attempt.report())
                }
            };
            Err(CheckedCommitLockFinishFailure {
                chain: CommitLockFailureChain::Source { source, post_leaf },
                post_unlock_report,
                secondary_unlock,
                observer,
                source_attempt,
            })
        } else {
            leaf.finish(body)
                .map_err(|failure| CheckedCommitLockFinishFailure {
                    chain: CommitLockFailureChain::Leaf(failure),
                    post_unlock_report,
                    secondary_unlock,
                    observer,
                    source_attempt,
                })
        }
    }
}
impl<L: CommitLockLeaf, E> Drop for CommitLockOwner<L, E> {
    fn drop(&mut self) {
        if self.leaf.is_some() {
            self.poison_scopes();
            self.source_attempt.gap(Gap::Abandoned);
            self.unlock_once();
            // The real leaf Drop marks Abandoned and settles leaf->clone once.
            drop(self.leaf.take());
            self.state = CommitLockState::Finished;
        }
    }
}
impl CommitLockOwner<CheckedCommittedLeaf<'_, '_, '_, '_>, StoreError> {
    fn acquire_checked(&mut self) -> Result<(), ()> {
        self.acquire_with(
            |error| error,
            |error| io_err("lock committed Raw manifest evidence", error),
        )
    }
    #[expect(
        clippy::result_large_err,
        reason = "The native completion retains the original typed source and all cleanup reports without a new allocation."
    )]
    fn finish_checked<T>(
        self,
        body: Result<T, StoreError>,
    ) -> Result<T, CheckedCommitLockFinishFailure> {
        self.finish_with(body, |error| {
            io_err("unlock committed Raw manifest evidence", error)
        })
    }
}

type ActualParentFailure<E> = super::committed_parent_close::CheckedCommittedDirectoryFailure<E>;

struct CommitLockCompletion<T, E> {
    source_attempt: Attempt,
    observer: CommitLockObserver,
    result: Result<T, CheckedCommitLockFinishFailure<E>>,
}
impl<T, E> CommitLockCompletion<T, E> {
    fn observer(&self) -> CommitLockObserver {
        self.observer.clone()
    }
}
impl CommitLockOwner<CheckedCommittedLeaf<'_, '_, '_, '_>, StoreError> {
    fn finish_for_parent_checked<T>(
        self,
        body: Result<T, StoreError>,
    ) -> CommitLockCompletion<T, StoreError> {
        let source_attempt = self.source_attempt.clone();
        let observer = self.observer.clone();
        let result = self.finish_checked(body); // Exact existing native unlock context/map.
        CommitLockCompletion {
            source_attempt,
            observer,
            result,
        }
    }
}

enum PostOwnerParent<P> {
    Failed(P),
    UnexpectedSuccess(Report),
}
#[expect(
    clippy::large_enum_variant,
    reason = "Keep the whole owner failure and later parent outcome in the same owned chronology without introducing a boxing boundary."
)]
enum CommitLockParentKind<E, P, Q> {
    Owner {
        owner: CheckedCommitLockFinishFailure<E>,
        parent: PostOwnerParent<P>,
    },
    ParentOnly(Q),
}
struct CommitLockParentFailure<E, P, Q> {
    kind: CommitLockParentKind<E, P, Q>,
    source_attempt: Attempt,
    observer: CommitLockObserver,
}
impl<E, P, Q> CommitLockParentFailure<E, P, Q> {
    fn owner_failure(&self) -> Option<&CheckedCommitLockFinishFailure<E>> {
        match &self.kind {
            CommitLockParentKind::Owner { owner, .. } => Some(owner),
            CommitLockParentKind::ParentOnly(_) => None,
        }
    }
    fn parent_failure(&self) -> Option<&Q> {
        match &self.kind {
            CommitLockParentKind::ParentOnly(parent) => Some(parent),
            _ => None,
        }
    }
    fn map_body_with<U, V, M>(
        self,
        map: M,
        map_parent: impl FnOnce(Q, M) -> V,
    ) -> CommitLockParentFailure<U, P, V>
    where
        M: FnOnce(E) -> U,
    {
        let kind = match self.kind {
            CommitLockParentKind::Owner { owner, parent } => CommitLockParentKind::Owner {
                owner: owner.map_body(map),
                parent,
            },
            CommitLockParentKind::ParentOnly(parent) => {
                CommitLockParentKind::ParentOnly(map_parent(parent, map))
            }
        };
        CommitLockParentFailure {
            kind,
            source_attempt: self.source_attempt,
            observer: self.observer,
        }
    }
}
impl<E, P, Q> std::fmt::Debug for CommitLockParentFailure<E, P, Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // P/Q need not implement Debug; never format body or arbitrary model data.
        f.debug_struct("CommitLockParentFailure")
            .field("owner_failure", &self.owner_failure())
            .field("parent_only_present", &self.parent_failure().is_some())
            .field("observations", &self.observer)
            .finish()
    }
}
type CheckedCommitLockParentFailure<E = StoreError> =
    CommitLockParentFailure<E, ActualParentFailure<Infallible>, ActualParentFailure<E>>;
impl<E> CommitLockParentFailure<E, ActualParentFailure<Infallible>, ActualParentFailure<E>> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match &self.kind {
            CommitLockParentKind::Owner { owner, .. } => owner.execution_failure(),
            CommitLockParentKind::ParentOnly(parent) => parent.execution_failure(),
        }
    }
    fn source_report(&self) -> Option<&Report> {
        match &self.kind {
            CommitLockParentKind::Owner { owner, .. } => owner.source_report(),
            CommitLockParentKind::ParentOnly(parent) => parent.source_report(),
        }
    }
    fn body(&self) -> Option<&E> {
        match &self.kind {
            CommitLockParentKind::Owner { owner, .. } => owner.body(),
            CommitLockParentKind::ParentOnly(parent) => parent.body(),
        }
    }
    fn post_parent_report(&self) -> &Report {
        match &self.kind {
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::Failed(parent),
                ..
            } => parent.report(),
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::UnexpectedSuccess(report),
                ..
            } => report,
            CommitLockParentKind::ParentOnly(parent) => parent.report(),
        }
    }
    fn parent_phase(&self) -> Option<CheckedCommittedDirectoryPhase> {
        match &self.kind {
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::Failed(parent),
                ..
            } => Some(parent.phase()),
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::UnexpectedSuccess(_),
                ..
            } => None,
            CommitLockParentKind::ParentOnly(parent) => Some(parent.phase()),
        }
    }
    fn parent_component_index(&self) -> Option<usize> {
        match &self.kind {
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::Failed(parent),
                ..
            } => parent.component_index(),
            CommitLockParentKind::Owner {
                parent: PostOwnerParent::UnexpectedSuccess(_),
                ..
            } => None,
            CommitLockParentKind::ParentOnly(parent) => parent.component_index(),
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommitLockParentFailure<U> {
        self.map_body_with(map, |parent, map| parent.map_body(map))
    }
}

// Actual token relation only; the SAME physical parent local is a caller contract.
fn poison_completion_parent_scopes<T, E>(
    completion: &CommitLockCompletion<T, E>,
    actual: &Attempt,
    expected: &Attempt,
    held_owner: &Attempt,
) {
    poison_foreign_scopes(actual, expected, &[&completion.source_attempt, held_owner]);
}
enum ParentFinishInput<T> {
    AfterOwnerFailure,
    Value(T),
}
enum ParentFinishOutcome<T, P, Q> {
    AfterOwnerFailure(Result<(), P>),
    Value(Result<T, Q>),
}

// Same production result dispatch for genuine native Parent and counted Source
// outcomes below that boundary. No resource/failure factory or Report authority.
#[expect(
    clippy::result_large_err,
    reason = "The result preserves whole owner and parent failures and their source identity."
)]
fn complete_commit_lock_parent_with<T, E, P, Q>(
    completion: CommitLockCompletion<T, E>,
    actual_parent: &Attempt,
    finish: impl FnOnce(ParentFinishInput<T>) -> ParentFinishOutcome<T, P, Q>,
) -> Result<T, CommitLockParentFailure<E, P, Q>> {
    let CommitLockCompletion {
        source_attempt,
        observer,
        result,
    } = completion;
    match result {
        Err(owner) => {
            if actual_parent.check() {
                actual_parent.gap(Gap::RawDailyClose);
            }
            let parent = match finish(ParentFinishInput::AfterOwnerFailure) {
                ParentFinishOutcome::AfterOwnerFailure(Err(parent)) => {
                    PostOwnerParent::Failed(parent)
                }
                ParentFinishOutcome::AfterOwnerFailure(Ok(())) => {
                    actual_parent.gap(Gap::RawDailyClose);
                    PostOwnerParent::UnexpectedSuccess(actual_parent.report())
                }
                ParentFinishOutcome::Value(_) => {
                    actual_parent.gap(Gap::RawDailyClose);
                    unreachable!("private parent completion shape")
                }
            };
            Err(CommitLockParentFailure {
                kind: CommitLockParentKind::Owner { owner, parent },
                source_attempt,
                observer,
            })
        }
        Ok(value) => match finish(ParentFinishInput::Value(value)) {
            ParentFinishOutcome::Value(Ok(value)) => Ok(value),
            ParentFinishOutcome::Value(Err(parent)) => Err(CommitLockParentFailure {
                kind: CommitLockParentKind::ParentOnly(parent),
                source_attempt,
                observer,
            }),
            ParentFinishOutcome::AfterOwnerFailure(_) => {
                actual_parent.gap(Gap::RawDailyClose);
                unreachable!("private parent completion shape")
            }
        },
    }
}

// Mandatory caller order (native fixture DEFERRED):
// let completion = owner.finish_for_parent_checked(body); // leaf+clone consumed
// complete_commit_lock_parent_checked(completion, parent); // SAME parent local
// Never move parent into an argument/closure while the actual leaf borrow lives.
#[expect(
    clippy::result_large_err,
    reason = "Consume the same retained parent while returning the whole original owner and parent diagnostics."
)]
fn complete_commit_lock_parent_checked<T, E>(
    completion: CommitLockCompletion<T, E>,
    parent: CheckedCommittedDirectory<'_, '_, '_>,
) -> Result<T, CheckedCommitLockParentFailure<E>> {
    let actual = parent.invocation().attempt().clone();
    poison_completion_parent_scopes(
        &completion,
        &actual,
        parent.expected_attempt(),
        parent.held_file().attempt(),
    );
    if let Err(failure) = &completion.result {
        failure.poison_against_parent(&parent);
    }
    complete_commit_lock_parent_with(completion, &actual, move |body| match body {
        ParentFinishInput::AfterOwnerFailure => {
            ParentFinishOutcome::AfterOwnerFailure(parent.finish(Ok::<(), Infallible>(())))
        }
        ParentFinishInput::Value(value) => {
            ParentFinishOutcome::Value(parent.finish(Ok::<T, E>(value)))
        }
    })
}

#[derive(Clone, Copy)]
enum CommitLockBorrowedOperation {
    Ancestry,
    HeldCheckpoint,
}
impl CommitLockBorrowedOperation {
    fn admits(self, state: CommitLockState) -> bool {
        match self {
            Self::Ancestry => matches!(
                state,
                CommitLockState::Fresh | CommitLockState::HeldObserved
            ),
            Self::HeldCheckpoint => state == CommitLockState::HeldObserved,
        }
    }
}
impl<L: CommitLockLeaf, E> CommitLockOwner<L, E> {
    fn admit_borrowed_operation(
        &mut self,
        operation: CommitLockBorrowedOperation,
    ) -> Result<(), ()> {
        if self.leaf.is_none() {
            self.refuse(CommitLockSourcePhase::State);
            return Err(());
        }
        self.poison_scopes(); // Includes the earlier source and all actual native parent/leaf/clone tokens.
        if !operation.admits(self.state)
            || self.source.is_some()
            || self.leaf().kind() != CheckedCommittedLeafKind::CommitLock
        {
            self.refuse(CommitLockSourcePhase::State); // Existing first source remains whole.
            return Err(());
        }
        if !self.source_attempt.check() {
            self.refuse(CommitLockSourcePhase::Scope);
            return Err(());
        }
        Ok(())
    }
    // ONE production admission/capture/result seam. P is the actual opaque
    // operation failure natively; counted cases use ordinary canonical DATA.
    fn borrowed_operation_with<P>(
        &mut self,
        operation: CommitLockBorrowedOperation,
        produce: impl FnOnce(&L) -> (Attempt, Result<(), P>),
        into_source: impl FnOnce(P, Attempt) -> CommitLockSource<E>,
    ) -> Result<(), ()> {
        let actual = self.source_attempt.clone();
        let mut unwind = CheckpointUnwind::new(&actual);
        let result = (|| {
            self.admit_borrowed_operation(operation)?;
            // Entire operation/result completes before owner.source mutation.
            // Neither closure captures a mutable self alongside leaf/parent.
            let (source_attempt, result) = produce(self.leaf());
            self.leaf().poison_scopes(&source_attempt);
            match result {
                Err(failure) => {
                    if self.source.is_none() {
                        self.source = Some(into_source(failure, source_attempt));
                    }
                    self.poison_scopes(); // Preserve real source tokens until final diagnostic extraction.
                    if actual.check() {
                        actual.gap(Gap::RawDailyClose);
                    }
                    Err(())
                }
                Ok(()) => {
                    self.poison_scopes();
                    if actual.check() {
                        Ok(())
                    } else {
                        self.refuse(CommitLockSourcePhase::Scope);
                        Err(())
                    }
                }
            }
        })();
        unwind.disarm();
        result
    }
    fn checkpoint_held_with(&mut self, map: impl FnOnce(StoreError) -> E) -> Result<(), ()> {
        self.borrowed_operation_with(
            CommitLockBorrowedOperation::HeldCheckpoint,
            |leaf| {
                let source_attempt = leaf.attempt().clone();
                let result = leaf.checkpoint();
                (source_attempt, result)
            },
            |failure, _source_attempt| CommitLockSource::Checkpoint(failure.map_body(map)),
        )
    }
}
impl CommitLockOwner<CheckedCommittedLeaf<'_, '_, '_, '_>, StoreError> {
    fn revalidate_ancestry_checked(&mut self) -> Result<(), ()> {
        self.borrowed_operation_with(
            CommitLockBorrowedOperation::Ancestry,
            |leaf| {
                let parent = leaf.parent;
                let source_attempt = parent.invocation().attempt().clone();
                let result = parent.revalidate_ancestry_checked();
                (source_attempt, result)
            },
            |failure, source_attempt| CommitLockSource::Ancestry {
                failure,
                source_attempt,
            },
        )
    }
    fn checkpoint_held_checked(&mut self) -> Result<(), ()> {
        self.checkpoint_held_with(|error| error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestOpenBoundary {
    OwnerAdmission,
    ManifestOpen,
}
enum ManifestOpenSource<E> {
    OwnerOnly,
    Open(CheckedCommittedLeafFailure<E>),
}
// Owned data AFTER lock owner consumed. No Parent/leaf/File borrow escapes.
struct ManifestOpenPending<E> {
    source: ManifestOpenSource<E>,
    source_attempt: Attempt,
    completion: CommitLockCompletion<(), E>,
}
impl<E> ManifestOpenPending<E> {
    fn boundary(&self) -> ManifestOpenBoundary {
        match self.source {
            ManifestOpenSource::OwnerOnly => ManifestOpenBoundary::OwnerAdmission,
            ManifestOpenSource::Open(_) => ManifestOpenBoundary::ManifestOpen,
        }
    }
    fn open_source(&self) -> Option<&CheckedCommittedLeafFailure<E>> {
        match &self.source {
            ManifestOpenSource::Open(source) => Some(source),
            ManifestOpenSource::OwnerOnly => None,
        }
    }
    fn poison_scopes(&self, actual: &Attempt, expected: &Attempt, held: &Attempt) {
        let mut tokens = vec![&self.source_attempt, &self.completion.source_attempt, held];
        if let ManifestOpenSource::Open(source) = &self.source {
            tokens.push(&source.source_attempt);
        }
        poison_foreign_scopes(actual, expected, &tokens);
    }
    fn poison_against_parent(&self, parent: &CheckedCommittedDirectory<'_, '_, '_>) {
        self.poison_scopes(
            parent.invocation().attempt(),
            parent.expected_attempt(),
            parent.held_file().attempt(),
        );
        if let Err(failure) = &self.completion.result {
            failure.poison_against_parent(parent);
        }
    }
}
impl<E> std::fmt::Debug for ManifestOpenPending<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestOpenPending")
            .field("boundary", &self.boundary())
            .field("open_source", &self.open_source())
            .field("observations", &self.completion.observer)
            .finish()
    }
}
// One real production admission/open-result/owner-cleanup dispatch. M is the
// actual native Manifest leaf or a counted production graph in Source cases.
#[expect(
    clippy::result_large_err,
    reason = "The owned pending error keeps the original open failure and lock cleanup until the borrowed phase ends."
)]
fn open_manifest_with<L: CommitLockLeaf, E, M>(
    mut owner: CommitLockOwner<L, E>,
    source_attempt: Attempt,
    open: impl FnOnce() -> Result<M, CheckedCommittedLeafFailure<E>>,
    finish: impl FnOnce(CommitLockOwner<L, E>) -> CommitLockCompletion<(), E>,
) -> Result<(CommitLockOwner<L, E>, M), ManifestOpenPending<E>> {
    let actual = owner.source_attempt.clone();
    let mut unwind = CheckpointUnwind::new(&actual);
    owner.poison_scopes();
    owner.leaf().poison_scopes(&source_attempt);
    if owner.observer.snapshot().poisoned {
        actual.gap(Gap::Abandoned);
    }
    let refused = if owner.state != CommitLockState::HeldObserved
        || owner.source.is_some()
        || owner.leaf().kind() != CheckedCommittedLeafKind::CommitLock
    {
        Some(CommitLockSourcePhase::State)
    } else if !actual.check() {
        Some(CommitLockSourcePhase::Scope)
    } else {
        None
    };
    if let Some(phase) = refused {
        owner.refuse(phase); // Earlier whole Some/None is never replaced.
        let completion = finish(owner);
        unwind.disarm();
        return Err(ManifestOpenPending {
            source: ManifestOpenSource::OwnerOnly,
            source_attempt,
            completion,
        });
    }
    match open() {
        Ok(manifest) => {
            // Return BOTH real acquired owners honestly. Late dirt must not
            // discard a newly acquired Manifest or fabricate final success.
            unwind.disarm();
            Ok((owner, manifest))
        }
        Err(source) => {
            owner.leaf().poison_scopes(&source.source_attempt);
            poison_foreign_scopes(&actual, &source_attempt, &[&source.source_attempt]);
            if actual.check() {
                actual.gap(Gap::RawDailyClose);
            }
            let completion = finish(owner); // Actual unlock->lock leaf->clone.
            unwind.disarm();
            Err(ManifestOpenPending {
                source: ManifestOpenSource::Open(source),
                source_attempt,
                completion,
            })
        }
    }
}
impl<'parent, 'invocation, 'attempt, 'expected>
    CommitLockOwner<CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>, StoreError>
{
    #[expect(
        clippy::result_large_err,
        reason = "Return the actual manifest-open failure and whole lock completion without replacing their ownership boundary."
    )]
    fn open_manifest_held_checked(
        self,
    ) -> Result<
        (
            Self,
            CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>,
        ),
        ManifestOpenPending<StoreError>,
    > {
        // Copy the EXTERNAL parent reference, not a borrow of owning self.
        let parent: &'parent CheckedCommittedDirectory<'invocation, 'attempt, 'expected> =
            self.leaf().parent;
        let source_attempt = parent.invocation().attempt().clone();
        open_manifest_with(
            self,
            source_attempt,
            || parent.open_leaf_checked(CheckedCommittedLeafKind::Manifest),
            |owner| owner.finish_for_parent_checked(Ok(())),
        )
    }
}
enum PostManifestOpen<L> {
    Failed(L),
    UnexpectedSuccess(Report),
}
enum ManifestOpenFailureKind<E, L, O> {
    OpenFirst {
        source: CheckedCommittedLeafFailure<E>,
        later: PostManifestOpen<L>,
    },
    OwnerOnly(PostManifestOpen<O>),
}
struct ManifestOpenFailure<E, L, O> {
    kind: ManifestOpenFailureKind<E, L, O>,
    source_attempt: Attempt,
    observer: CommitLockObserver,
}
impl<E, L, O> ManifestOpenFailure<E, L, O> {
    fn boundary(&self) -> ManifestOpenBoundary {
        match self.kind {
            ManifestOpenFailureKind::OpenFirst { .. } => ManifestOpenBoundary::ManifestOpen,
            ManifestOpenFailureKind::OwnerOnly(_) => ManifestOpenBoundary::OwnerAdmission,
        }
    }
    fn open_source(&self) -> Option<&CheckedCommittedLeafFailure<E>> {
        match &self.kind {
            ManifestOpenFailureKind::OpenFirst { source, .. } => Some(source),
            _ => None,
        }
    }
    // L is intentionally unchanged on OpenFirst: downstream bodies are
    // SECONDARY, never mapped as another primary. O is the owner-only primary.
    fn map_body_with<U, V, M>(
        self,
        map: M,
        map_owner: impl FnOnce(O, M) -> V,
    ) -> ManifestOpenFailure<U, L, V>
    where
        M: FnOnce(E) -> U,
    {
        let kind = match self.kind {
            ManifestOpenFailureKind::OpenFirst { source, later } => {
                ManifestOpenFailureKind::OpenFirst {
                    source: source.map_body(map),
                    later,
                }
            }
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(owner)) => {
                ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(map_owner(owner, map)))
            }
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::UnexpectedSuccess(report)) => {
                ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::UnexpectedSuccess(report))
            }
        };
        ManifestOpenFailure {
            kind,
            source_attempt: self.source_attempt,
            observer: self.observer,
        }
    }
}
impl<E, L, O> std::fmt::Debug for ManifestOpenFailure<E, L, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestOpenFailure")
            .field("boundary", &self.boundary())
            .field("open_source", &self.open_source())
            .field("observations", &self.observer)
            .finish()
    }
}
// Actual parent-completion Result remains WHOLE, independently of leaf-open
// primary. A callback's impossible Ok yields actual gap/report, never a factory.
fn complete_manifest_open_with<E, L>(
    pending: ManifestOpenPending<E>,
    actual: &Attempt,
    complete: impl FnOnce(CommitLockCompletion<(), E>) -> Result<(), L>,
) -> ManifestOpenFailure<E, L, L> {
    let ManifestOpenPending {
        source,
        source_attempt,
        completion,
    } = pending;
    if actual.check() {
        actual.gap(Gap::RawDailyClose);
    }
    let observer = completion.observer();
    let later = match complete(completion) {
        Err(failure) => PostManifestOpen::Failed(failure),
        Ok(()) => {
            actual.gap(Gap::RawDailyClose);
            PostManifestOpen::UnexpectedSuccess(actual.report())
        }
    };
    let kind = match source {
        ManifestOpenSource::Open(source) => ManifestOpenFailureKind::OpenFirst { source, later },
        ManifestOpenSource::OwnerOnly => ManifestOpenFailureKind::OwnerOnly(later),
    };
    ManifestOpenFailure {
        kind,
        source_attempt,
        observer,
    }
}
type CheckedManifestOpenFailure<E = StoreError> = ManifestOpenFailure<
    E,
    CheckedCommitLockParentFailure<StoreError>,
    CheckedCommitLockParentFailure<E>,
>;
impl<E> CheckedManifestOpenFailure<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match &self.kind {
            ManifestOpenFailureKind::OpenFirst { source, .. } => source.execution_failure(),
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(owner)) => {
                owner.execution_failure()
            }
            _ => None,
        }
    }
    fn source_report(&self) -> Option<&Report> {
        match &self.kind {
            ManifestOpenFailureKind::OpenFirst { source, .. } => source.source_report(),
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(owner)) => {
                owner.source_report()
            }
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::UnexpectedSuccess(report)) => {
                Some(report)
            }
        }
    }
    fn body(&self) -> Option<&E> {
        match &self.kind {
            ManifestOpenFailureKind::OpenFirst { source, .. } => source.body(),
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(owner)) => owner.body(),
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::UnexpectedSuccess(_)) => None,
        }
    }
    fn post_parent_report(&self) -> &Report {
        match &self.kind {
            ManifestOpenFailureKind::OpenFirst {
                later: PostManifestOpen::Failed(later),
                ..
            } => later.post_parent_report(),
            ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(later)) => {
                later.post_parent_report()
            }
            ManifestOpenFailureKind::OpenFirst {
                later: PostManifestOpen::UnexpectedSuccess(report),
                ..
            }
            | ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::UnexpectedSuccess(report)) => {
                report
            }
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedManifestOpenFailure<U> {
        self.map_body_with(map, |owner, map| owner.map_body(map))
    }
}
// Call ONLY after openErr has consumed lock owner and the open Result/borrow
// temporary scope has ended. Move the SAME external parent local here.
#[expect(
    clippy::result_large_err,
    reason = "The parent-completion closure returns its whole cleanup failure alongside the earlier manifest-open source."
)]
fn complete_manifest_open_checked(
    pending: ManifestOpenPending<StoreError>,
    parent: CheckedCommittedDirectory<'_, '_, '_>,
) -> CheckedManifestOpenFailure {
    let actual = parent.invocation().attempt().clone();
    pending.poison_against_parent(&parent); // Original leaf + whole owner tokens before extraction.
    complete_manifest_open_with(pending, &actual, move |completion| {
        complete_commit_lock_parent_checked(completion, parent)
    })
}

use super::{
    CommittedManifestReadLimits, committed_manifest_read_buffer_len,
    read_committed_manifest_bounded,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CheckedCommittedManifestReadPhase {
    Scope,
    Kind,
    Admission,
    Limits,
    BeforeRead,
    Read,
    AfterRead,
    Settlement,
}

enum ManifestReadFailureSource<E> {
    Checkpoint(CheckedCommittedLeafCheckpointFailure<E>),
    Local(Failure<E>),
}

// Opaque operation failure while the caller still owns leaf, clone and parent.
// The source token is captured before effects; Reports remain diagnostics.
pub(super) struct CheckedCommittedManifestReadFailure<E = StoreError> {
    source: ManifestReadFailureSource<E>,
    phase: CheckedCommittedManifestReadPhase,
    kind: CheckedCommittedLeafKind,
    source_attempt: Attempt,
}

impl<E> CheckedCommittedManifestReadFailure<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.checkpoint_failure()
            .and_then(|f| f.execution_failure())
    }
    fn source_report(&self) -> Option<&Report> {
        match &self.source {
            ManifestReadFailureSource::Checkpoint(f) => f.source_report(),
            ManifestReadFailureSource::Local(f) => Some(f.report()),
        }
    }
    fn local(
        failure: Failure<E>,
        phase: CheckedCommittedManifestReadPhase,
        kind: CheckedCommittedLeafKind,
        source: &Attempt,
    ) -> Self {
        Self {
            source: ManifestReadFailureSource::Local(failure),
            phase,
            kind,
            source_attempt: source.clone(),
        }
    }

    fn checkpoint(
        failure: CheckedCommittedLeafCheckpointFailure<E>,
        phase: CheckedCommittedManifestReadPhase,
        kind: CheckedCommittedLeafKind,
        source: &Attempt,
    ) -> Self {
        // Retain the whole genuine checkpoint and its own source token.
        poison_foreign_scopes(source, source, &[&failure.source_attempt]);
        Self {
            source: ManifestReadFailureSource::Checkpoint(failure),
            phase,
            kind,
            source_attempt: source.clone(),
        }
    }

    pub(super) fn body(&self) -> Option<&E> {
        match &self.source {
            ManifestReadFailureSource::Checkpoint(failure) => failure.body(),
            ManifestReadFailureSource::Local(failure) => failure.body(),
        }
    }

    pub(super) fn report(&self) -> &Report {
        match &self.source {
            ManifestReadFailureSource::Checkpoint(failure) => failure.report(),
            ManifestReadFailureSource::Local(failure) => failure.report(),
        }
    }

    pub(super) fn phase(&self) -> CheckedCommittedManifestReadPhase {
        self.phase
    }

    pub(super) fn kind(&self) -> CheckedCommittedLeafKind {
        self.kind
    }

    pub(super) fn checkpoint_failure(&self) -> Option<&CheckedCommittedLeafCheckpointFailure<E>> {
        match &self.source {
            ManifestReadFailureSource::Checkpoint(failure) => Some(failure),
            ManifestReadFailureSource::Local(_) => None,
        }
    }

    // A future same-module owner/parent bridge must compare this actual token
    // and any retained checkpoint token BEFORE diagnostic extraction.
    fn source_attempt(&self) -> &Attempt {
        &self.source_attempt
    }

    pub(super) fn map_body<U>(
        self,
        map: impl FnOnce(E) -> U,
    ) -> CheckedCommittedManifestReadFailure<U> {
        let source = match self.source {
            ManifestReadFailureSource::Checkpoint(failure) => {
                ManifestReadFailureSource::Checkpoint(failure.map_body(map))
            }
            ManifestReadFailureSource::Local(failure) => {
                ManifestReadFailureSource::Local(failure.map_body(map))
            }
        };
        CheckedCommittedManifestReadFailure {
            source,
            phase: self.phase,
            kind: self.kind,
            source_attempt: self.source_attempt,
        }
    }
}

impl<E> std::fmt::Debug for CheckedCommittedManifestReadFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedManifestReadFailure")
            .field("body_present", &self.body().is_some())
            .field("phase", &self.phase)
            .field("kind", &self.kind)
            .field(
                "checkpoint_phase",
                &self.checkpoint_failure().map(|failure| failure.phase()),
            )
            .field("report", self.report())
            .finish()
    }
}

fn manifest_read_body_failure<E, R: Close>(
    graph: &CommittedLeafGraph<'_, '_, '_, R>,
    source: &Attempt,
    phase: CheckedCommittedManifestReadPhase,
    body: E,
) -> CheckedCommittedManifestReadFailure<E> {
    let failure = graph
        .invocation
        .settle_local_in(graph.expected, Err::<(), E>(body))
        .expect_err("an original bounded-read body cannot become success");
    CheckedCommittedManifestReadFailure::local(failure, phase, graph.kind, source)
}

// ONE borrowed production sequence. Models use the actual counted Owner graph.
// Scope equality is ledger identity only, never physical parent/root evidence.
// No resource is moved, drained, unlocked or closed by this operation.
#[expect(
    clippy::result_large_err,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
fn run_borrowed_manifest_read<T, E, R: Close>(
    graph: &CommittedLeafGraph<'_, '_, '_, R>,
    captured_source: &Attempt,
    parent_scopes: &[&Attempt],
    preflight: impl FnOnce() -> Result<(), CheckedCommittedLeafCheckpointFailure<E>>,
    mut checkpoint: impl FnMut() -> Result<(), CheckedCommittedLeafCheckpointFailure<E>>,
    prepare: impl FnOnce() -> Result<usize, E>,
    read: impl FnOnce(&R, usize) -> Result<T, E>,
) -> Result<T, CheckedCommittedManifestReadFailure<E>> {
    let source = graph.invocation.attempt().clone();
    let mut unwind = CheckpointUnwind::new(&source);
    let result = (|| {
        poison_foreign_scopes(&source, graph.expected, &[captured_source]);
        poison_foreign_scopes(&source, graph.expected, parent_scopes);
        // Check BOTH Owner slots even after earlier parent/source dirt.
        graph.scope::<E>().map_err(|failure| {
            CheckedCommittedManifestReadFailure::local(
                failure,
                CheckedCommittedManifestReadPhase::Scope,
                graph.kind,
                &source,
            )
        })?;
        if graph.kind != CheckedCommittedLeafKind::Manifest {
            return Err(CheckedCommittedManifestReadFailure::local(
                graph.invocation.refused(Gap::RawDailyClose),
                CheckedCommittedManifestReadPhase::Kind,
                graph.kind,
                &source,
            ));
        }
        preflight().map_err(|failure| {
            CheckedCommittedManifestReadFailure::checkpoint(
                failure,
                CheckedCommittedManifestReadPhase::Admission,
                graph.kind,
                &source,
            )
        })?;
        let buffer_len = prepare().map_err(|body| {
            manifest_read_body_failure(
                graph,
                &source,
                CheckedCommittedManifestReadPhase::Limits,
                body,
            )
        })?;
        checkpoint().map_err(|failure| {
            CheckedCommittedManifestReadFailure::checkpoint(
                failure,
                CheckedCommittedManifestReadPhase::BeforeRead,
                graph.kind,
                &source,
            )
        })?;
        let value = read(
            graph
                .leaf
                .as_ref()
                .expect("borrowed Manifest remains held")
                .get(),
            buffer_len,
        )
        .map_err(|body| {
            manifest_read_body_failure(
                graph,
                &source,
                CheckedCommittedManifestReadPhase::Read,
                body,
            )
        })?;
        // A read error has returned above and is never replaced by a postcheck.
        checkpoint().map_err(|failure| {
            CheckedCommittedManifestReadFailure::checkpoint(
                failure,
                CheckedCommittedManifestReadPhase::AfterRead,
                graph.kind,
                &source,
            )
        })?;
        graph
            .invocation
            .settle_local_in(graph.expected, Ok(value))
            .map_err(|failure| {
                CheckedCommittedManifestReadFailure::local(
                    failure,
                    CheckedCommittedManifestReadPhase::Settlement,
                    graph.kind,
                    &source,
                )
            })
    })();
    unwind.disarm();
    result
}

impl<'parent, 'invocation, 'attempt, 'expected>
    CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>
{
    // Bytes only: the same original bounds/read policy, no parse/record
    // enforcement, rewind, close or reader/publication completion.
    #[expect(
        clippy::result_large_err,
        reason = "Keep the whole borrowed read or checkpoint failure until the actual manifest and owner cleanup completes."
    )]
    pub(super) fn read_manifest_bounded_checked(
        &self,
        limits: &CommittedManifestReadLimits,
    ) -> Result<Vec<u8>, CheckedCommittedManifestReadFailure> {
        let graph = self.graph();
        let source = graph.invocation.attempt().clone();
        run_borrowed_manifest_read(
            graph,
            &source,
            &[
                self.parent.invocation().attempt(),
                self.parent.expected_attempt(),
                self.parent.held_file().attempt(),
            ],
            || match parent_preflight(self.parent) {
                Some(admission) => {
                    let mut failure = graph.checkpoint_refusal(admission.phase);
                    failure.admission = Some(admission);
                    Err(failure)
                }
                None => Ok(()),
            },
            || self.checkpoint_borrowed(),
            || committed_manifest_read_buffer_len(limits),
            |file, buffer_len| {
                read_committed_manifest_bounded(
                    file,
                    &self.path,
                    limits.max_manifest_bytes,
                    buffer_len,
                )
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestOperationPhase {
    Scope,
    Kind,
    Admission(CheckedCommittedLeafPhase),
    Read(CheckedCommittedManifestReadPhase),
    Parse,
    Settlement,
}

enum ManifestOperationSource<E> {
    Read(CheckedCommittedManifestReadFailure<E>),
    Local {
        failure: Failure<E>,
        phase: ManifestOperationPhase,
        kind: CheckedCommittedLeafKind,
    },
}
// Whole original source, with intrinsic graph identity captured BEFORE effects.
// No public constructor, resource factory or diagnostic identity authority.
struct ManifestOperationFailure<E> {
    source: ManifestOperationSource<E>,
    source_attempt: Attempt,
    admission: Option<LeafAdmissionFailure>,
}
impl<E> ManifestOperationFailure<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.admission
            .as_ref()
            .and_then(|a| a.execution.as_ref())
            .or_else(|| self.read_failure().and_then(|f| f.execution_failure()))
    }
    fn source_report(&self) -> Option<&Report> {
        if let Some(admission) = &self.admission {
            return admission.source_report();
        }
        match &self.source {
            ManifestOperationSource::Read(f) => f.source_report(),
            ManifestOperationSource::Local { failure, .. } => Some(failure.report()),
        }
    }
    fn poison_against(&self, actual: &Attempt, expected: &Attempt) {
        poison_foreign_scopes(actual, expected, &[&self.source_attempt]);
        if let ManifestOperationSource::Read(read) = &self.source {
            poison_foreign_scopes(actual, expected, &[read.source_attempt()]);
            if let Some(checkpoint) = read.checkpoint_failure() {
                poison_foreign_scopes(actual, expected, &[&checkpoint.source_attempt]);
            }
        }
    }
    fn body(&self) -> Option<&E> {
        match &self.source {
            ManifestOperationSource::Read(read) => read.body(),
            ManifestOperationSource::Local { failure, .. } => failure.body(),
        }
    }
    fn report(&self) -> &Report {
        match &self.source {
            ManifestOperationSource::Read(read) => read.report(),
            ManifestOperationSource::Local { failure, .. } => failure.report(),
        }
    }
    fn phase(&self) -> ManifestOperationPhase {
        match &self.source {
            ManifestOperationSource::Read(read) => ManifestOperationPhase::Read(read.phase()),
            ManifestOperationSource::Local { phase, .. } => *phase,
        }
    }
    fn kind(&self) -> CheckedCommittedLeafKind {
        match &self.source {
            ManifestOperationSource::Read(read) => read.kind(),
            ManifestOperationSource::Local { kind, .. } => *kind,
        }
    }
    fn read_failure(&self) -> Option<&CheckedCommittedManifestReadFailure<E>> {
        match &self.source {
            ManifestOperationSource::Read(read) => Some(read),
            _ => None,
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> ManifestOperationFailure<U> {
        let source = match self.source {
            ManifestOperationSource::Read(read) => {
                ManifestOperationSource::Read(read.map_body(map))
            }
            ManifestOperationSource::Local {
                failure,
                phase,
                kind,
            } => ManifestOperationSource::Local {
                failure: failure.map_body(map),
                phase,
                kind,
            },
        };
        ManifestOperationFailure {
            source,
            source_attempt: self.source_attempt,
            admission: self.admission,
        }
    }
}
impl<E> std::fmt::Debug for ManifestOperationFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestOperationFailure")
            .field("body_present", &self.body().is_some())
            .field("kind", &self.kind())
            .field("phase", &self.phase())
            .field("report", self.report())
            .finish()
    }
}

// Private callback context contains real invocation borrows, not authority.
// Future read/parser driver may wrap its ORIGINAL results; no parser runs here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ManifestFirstSource {
    Unobserved,
    Manifest,
    Owner,
}
struct ManifestOperationContext<'scope, 'attempt> {
    invocation: &'scope RawInvocation<'attempt>,
    expected: &'scope Attempt,
    source_attempt: Attempt,
    kind: CheckedCommittedLeafKind,
    first: std::cell::Cell<ManifestFirstSource>, // private one-shot chronology, not native authority
}
impl ManifestOperationContext<'_, '_> {
    fn note_first(&self, first: ManifestFirstSource) {
        if self.first.get() == ManifestFirstSource::Unobserved {
            self.first.set(first);
        }
    }
    // Future held-body driver may interleave its actual owner ancestry/node
    // step between successful read and parse. No ancestry API is assumed.
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn owner_step<L: CommitLockLeaf, E>(
        &self,
        owner: &mut CommitLockOwner<L, E>,
        operation: impl FnOnce(&mut CommitLockOwner<L, E>),
    ) -> Result<(), ManifestOperationFailure<E>> {
        owner.poison_scopes();
        owner.leaf().poison_scopes(&self.source_attempt);
        if self.first.get() == ManifestFirstSource::Manifest {
            return Err(self.refuse(ManifestOperationPhase::Scope, Gap::RawDailyClose));
        }
        if owner.source.is_none() {
            if owner.state != CommitLockState::HeldObserved
                || owner.leaf().kind() != CheckedCommittedLeafKind::CommitLock
            {
                owner.refuse(CommitLockSourcePhase::State);
            } else if !self.source_attempt.check() {
                owner.refuse(CommitLockSourcePhase::Scope);
            } else {
                operation(owner);
            }
        }
        owner.poison_scopes();
        owner.leaf().poison_scopes(&self.source_attempt);
        if owner.source.is_none() {
            if owner.state != CommitLockState::HeldObserved
                || owner.leaf().kind() != CheckedCommittedLeafKind::CommitLock
            {
                owner.refuse(CommitLockSourcePhase::State);
            } else if !self.source_attempt.check() {
                owner.refuse(CommitLockSourcePhase::Scope);
            }
        }
        if owner.source.is_some() {
            self.note_first(ManifestFirstSource::Owner);
            Err(self.refuse(ManifestOperationPhase::Scope, Gap::RawDailyClose))
        } else {
            Ok(())
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn local<T, E>(
        &self,
        phase: ManifestOperationPhase,
        body: Result<T, E>,
    ) -> Result<T, ManifestOperationFailure<E>> {
        poison_foreign_scopes(
            self.invocation.attempt(),
            self.expected,
            &[&self.source_attempt],
        );
        self.invocation
            .settle_local_in(self.expected, body)
            .map_err(|failure| {
                self.note_first(ManifestFirstSource::Manifest);
                ManifestOperationFailure {
                    source: ManifestOperationSource::Local {
                        failure,
                        phase,
                        kind: self.kind,
                    },
                    source_attempt: self.source_attempt.clone(),
                    admission: None,
                }
            })
    }
    fn refuse<E>(&self, phase: ManifestOperationPhase, gap: Gap) -> ManifestOperationFailure<E> {
        self.invocation.attempt().gap(gap);
        match self.local::<(), E>(phase, Ok(())) {
            Err(failure) => failure,
            Ok(()) => unreachable!("permanent actual gap refuses local success"),
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn read<T, E>(
        &self,
        result: Result<T, CheckedCommittedManifestReadFailure<E>>,
    ) -> Result<T, ManifestOperationFailure<E>> {
        result.map_err(|read| {
            self.note_first(ManifestFirstSource::Manifest);
            let failure = ManifestOperationFailure {
                source: ManifestOperationSource::Read(read),
                source_attempt: self.source_attempt.clone(),
                admission: None,
            };
            failure.poison_against(self.invocation.attempt(), self.expected);
            failure
        })
    }
}

trait ManifestCompletionLeaf: Sized {
    fn invocation(&self) -> &RawInvocation<'_>;
    fn expected(&self) -> &Attempt;
    fn kind(&self) -> CheckedCommittedLeafKind;
    fn poison_scopes(&self, source: &Attempt);
    fn admission(&self) -> Option<CheckedCommittedLeafPhase>;
    fn admission_failure(&self) -> Option<LeafAdmissionFailure> {
        self.admission().map(|phase| LeafAdmissionFailure {
            phase,
            original: Ok(()),
            execution: None,
        })
    }
    #[expect(
        clippy::result_large_err,
        reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
    )]
    fn finish<T, E>(self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>>;
}
impl ManifestCompletionLeaf for CheckedCommittedLeaf<'_, '_, '_, '_> {
    fn invocation(&self) -> &RawInvocation<'_> {
        self.graph().invocation
    }
    fn expected(&self) -> &Attempt {
        self.graph().expected
    }
    fn kind(&self) -> CheckedCommittedLeafKind {
        self.graph().kind
    }
    fn poison_scopes(&self, source: &Attempt) {
        // Existing actual graph/parent/invocation/expected/leaf/clone comparisons.
        CommitLockLeaf::poison_scopes(self, source);
    }
    fn admission(&self) -> Option<CheckedCommittedLeafPhase> {
        // Production uses admission_failure(), which retains the whole typed result.
        (!self.graph().invocation.attempt().check()).then_some(CheckedCommittedLeafPhase::Scope)
    }
    fn admission_failure(&self) -> Option<LeafAdmissionFailure> {
        parent_preflight(self.parent)
    }
    fn finish<T, E>(self, body: Result<T, E>) -> Result<T, CheckedCommittedLeafFailure<E>> {
        CheckedCommittedLeaf::finish(self, body)
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ManifestPrimary<E> {
    Operation {
        source: ManifestOperationFailure<E>,
        post_manifest: PostSourceLeaf,
    },
    Close(CheckedCommittedLeafFailure<Infallible>),
}
impl<E> ManifestPrimary<E> {
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match self {
            Self::Operation { source, .. } => source.execution_failure(),
            Self::Close(f) => f.execution_failure(),
        }
    }
    fn source_report(&self) -> Option<&Report> {
        match self {
            Self::Operation { source, .. } => source.source_report(),
            Self::Close(f) => f.source_report(),
        }
    }
    fn poison_against(&self, actual: &Attempt) {
        match self {
            Self::Operation {
                source,
                post_manifest,
            } => {
                source.poison_against(actual, actual);
                if let PostSourceLeaf::Failed(failure) = post_manifest {
                    poison_foreign_scopes(actual, actual, &[&failure.source_attempt]);
                }
            }
            Self::Close(failure) => {
                poison_foreign_scopes(actual, actual, &[&failure.source_attempt])
            }
        }
    }
    fn body(&self) -> Option<&E> {
        match self {
            Self::Operation { source, .. } => source.body(),
            Self::Close(_) => None,
        }
    }
    fn post_manifest_report(&self) -> &Report {
        match self {
            Self::Operation { post_manifest, .. } => post_manifest.report(),
            Self::Close(f) => f.report(),
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> ManifestPrimary<U> {
        match self {
            Self::Operation {
                source,
                post_manifest,
            } => ManifestPrimary::Operation {
                source: source.map_body(map),
                post_manifest,
            },
            Self::Close(failure) => ManifestPrimary::Close(failure),
        }
    }
}
#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ManifestPlan<T, E> {
    OwnerFirst,
    Operation(ManifestOperationFailure<E>),
    Value(T),
}
#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ManifestCompletionKind<T, E> {
    OwnerFirst(PostSourceLeaf),
    ManifestFirst(ManifestPrimary<E>),
    Value(T),
}
struct ManifestCompletion<T, E> {
    kind: ManifestCompletionKind<T, E>,
    source_attempt: Attempt,
}
fn finish_manifest_after_source<M: ManifestCompletionLeaf>(
    manifest: M,
    actual: &Attempt,
) -> PostSourceLeaf {
    if actual.check() {
        actual.gap(Gap::RawDailyClose);
    }
    match manifest.finish(Ok::<(), Infallible>(())) {
        Err(failure) => PostSourceLeaf::Failed(failure),
        Ok(()) => {
            actual.gap(Gap::RawDailyClose);
            PostSourceLeaf::UnexpectedSuccess(actual.report())
        }
    }
}

// ONE production operation/chronology/consume dispatcher, also used by counted
// actual CommittedLeafGraph models. Held owner steps use the same private context.
#[expect(
    clippy::result_large_err,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
fn consume_manifest_operation<M, L, T, E>(
    manifest: M,
    owner: &mut CommitLockOwner<L, E>,
    operation: impl FnOnce(
        &M,
        &mut CommitLockOwner<L, E>,
        &ManifestOperationContext<'_, '_>,
    ) -> Result<T, ManifestOperationFailure<E>>,
) -> ManifestCompletion<T, E>
where
    M: ManifestCompletionLeaf,
    L: CommitLockLeaf,
{
    let actual = manifest.invocation().attempt().clone(); // BEFORE operation or diagnostic extraction.
    let mut unwind = CheckpointUnwind::new(&actual);
    let plan = {
        manifest.poison_scopes(&actual);
        manifest.poison_scopes(&owner.source_attempt);
        owner.poison_scopes();
        owner.leaf().poison_scopes(&actual);
        // State/source are inspected BEFORE Manifest cleanup, never late dirt.
        if owner.source.is_none() && owner.state != CommitLockState::HeldObserved {
            owner.refuse(CommitLockSourcePhase::State);
        }
        let context = ManifestOperationContext {
            invocation: manifest.invocation(),
            expected: manifest.expected(),
            source_attempt: actual.clone(),
            kind: manifest.kind(),
            first: std::cell::Cell::new(ManifestFirstSource::Unobserved),
        };
        if owner.source.is_some() {
            ManifestPlan::OwnerFirst
        } else if manifest.kind() != CheckedCommittedLeafKind::Manifest {
            ManifestPlan::Operation(
                context.refuse(ManifestOperationPhase::Kind, Gap::RawDailyClose),
            )
        } else if !actual.check() {
            ManifestPlan::Operation(
                context.refuse(ManifestOperationPhase::Scope, Gap::RawDailyClose),
            )
        } else if let Some(admission) = manifest.admission_failure() {
            let mut source = context.refuse(
                ManifestOperationPhase::Admission(admission.phase),
                Gap::RawDailyClose,
            );
            source.admission = Some(admission);
            ManifestPlan::Operation(source)
        } else {
            let result = operation(&manifest, owner, &context);
            if let Err(failure) = &result {
                failure.poison_against(&actual, manifest.expected());
            }
            manifest.poison_scopes(&actual);
            owner.poison_scopes();
            owner.leaf().poison_scopes(&actual);
            if owner.source.is_some() && context.first.get() != ManifestFirstSource::Manifest {
                ManifestPlan::OwnerFirst
            } else {
                match result
                    .and_then(|value| context.local(ManifestOperationPhase::Settlement, Ok(value)))
                {
                    Err(failure) => ManifestPlan::Operation(failure),
                    Ok(value) => ManifestPlan::Value(value),
                }
            }
        }
    }; // invocation/context borrows end before consuming the actual Manifest.
    let kind = match plan {
        ManifestPlan::OwnerFirst => {
            ManifestCompletionKind::OwnerFirst(finish_manifest_after_source(manifest, &actual))
        }
        ManifestPlan::Operation(source) => {
            ManifestCompletionKind::ManifestFirst(ManifestPrimary::Operation {
                source,
                post_manifest: finish_manifest_after_source(manifest, &actual),
            })
        }
        ManifestPlan::Value(value) => match manifest.finish(Ok::<T, Infallible>(value)) {
            Ok(value) => ManifestCompletionKind::Value(value),
            Err(failure) => ManifestCompletionKind::ManifestFirst(ManifestPrimary::Close(failure)),
        },
    };
    unwind.disarm();
    drop(unwind);
    ManifestCompletion {
        kind,
        source_attempt: actual,
    }
}
impl<'parent, 'invocation, 'attempt, 'expected>
    CheckedCommittedLeaf<'parent, 'invocation, 'attempt, 'expected>
{
    // Infrastructure only. Future driver can scope.read(real borrowed read)
    // and scope.local(Parse, ORIGINAL parser result); neither is invoked here.
    fn finish_manifest_operation_checked<'lp, 'li, 'la, 'le, T>(
        self,
        owner: &mut CommitLockOwner<CheckedCommittedLeaf<'lp, 'li, 'la, 'le>, StoreError>,
        operation: impl FnOnce(
            &Self,
            &mut CommitLockOwner<CheckedCommittedLeaf<'lp, 'li, 'la, 'le>, StoreError>,
            &ManifestOperationContext<'_, '_>,
        ) -> Result<T, ManifestOperationFailure<StoreError>>,
    ) -> ManifestCompletion<T, StoreError> {
        consume_manifest_operation(self, owner, operation)
    }
}

// Ordinary payload, not cleanup permission. Only the private dispatcher chooses it.
enum ManifestPayload<T> {
    Value(T),
    NoValue,
}
#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ManifestPending<E> {
    Value,
    OwnerFirst(PostSourceLeaf),
    ManifestFirst(ManifestPrimary<E>),
}
impl<E> ManifestPending<E> {
    fn poison_against(&self, actual: &Attempt) {
        match self {
            Self::ManifestFirst(primary) => primary.poison_against(actual),
            Self::OwnerFirst(PostSourceLeaf::Failed(failure)) => {
                poison_foreign_scopes(actual, actual, &[&failure.source_attempt])
            }
            _ => {}
        }
    }
}
struct ManifestOwnerCompletion<T, E> {
    source_attempt: Attempt,
    pending: ManifestPending<E>,
    owner: CommitLockCompletion<ManifestPayload<T>, E>,
}
impl<T, E> ManifestCompletion<T, E> {
    fn owner_input<L: CommitLockLeaf>(
        self,
        owner: &CommitLockOwner<L, E>,
    ) -> (Attempt, ManifestPending<E>, ManifestPayload<T>) {
        owner.poison_scopes();
        owner.leaf().poison_scopes(&self.source_attempt);
        poison_foreign_scopes(
            &self.source_attempt,
            &owner.source_attempt,
            &[owner.leaf().attempt()],
        );
        let (pending, payload) = match self.kind {
            ManifestCompletionKind::OwnerFirst(post_manifest) => (
                ManifestPending::OwnerFirst(post_manifest),
                ManifestPayload::NoValue,
            ),
            ManifestCompletionKind::ManifestFirst(primary) => (
                ManifestPending::ManifestFirst(primary),
                ManifestPayload::NoValue,
            ),
            ManifestCompletionKind::Value(value) => {
                (ManifestPending::Value, ManifestPayload::Value(value))
            }
        };
        pending.poison_against(&owner.source_attempt);
        (self.source_attempt, pending, payload)
    }
}
impl<T> ManifestCompletion<T, StoreError> {
    fn finish_owner_checked(
        self,
        owner: CommitLockOwner<CheckedCommittedLeaf<'_, '_, '_, '_>, StoreError>,
    ) -> ManifestOwnerCompletion<T, StoreError> {
        let (source_attempt, pending, payload) = self.owner_input(&owner);
        let owner = owner.finish_for_parent_checked(Ok(payload)); // Exact existing native map/unlock order.
        ManifestOwnerCompletion {
            source_attempt,
            pending,
            owner,
        }
    }
}

#[expect(
    clippy::large_enum_variant,
    reason = "Retain the original whole owner and parent failure as later diagnostics without a new allocation or flattening."
)]
enum ManifestLater<O, P, Q> {
    Failed(CommitLockParentFailure<O, P, Q>),
    UnexpectedSuccess(Report),
}
impl<O, P, Q> ManifestLater<O, P, Q> {
    fn failure(&self) -> Option<&CommitLockParentFailure<O, P, Q>> {
        match self {
            Self::Failed(failure) => Some(failure),
            _ => None,
        }
    }
    fn map_body_with<U, V, F>(
        self,
        map: F,
        map_parent: impl FnOnce(Q, F) -> V,
    ) -> ManifestLater<U, P, V>
    where
        F: FnOnce(O) -> U,
    {
        match self {
            Self::Failed(failure) => ManifestLater::Failed(failure.map_body_with(map, map_parent)),
            Self::UnexpectedSuccess(report) => ManifestLater::UnexpectedSuccess(report),
        }
    }
}
#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ManifestFinishKind<E, O, P, Q> {
    OwnerFirst {
        post_manifest: PostSourceLeaf,
        later: ManifestLater<O, P, Q>,
    },
    ManifestFirst {
        primary: ManifestPrimary<E>,
        later: ManifestLater<O, P, Q>,
    },
    Later(CommitLockParentFailure<O, P, Q>),
    Local(Failure<Infallible>),
}
// E and O remain separate: mapping an original Manifest body never flattens or
// rewrites a later genuine owner/parent typed body/secondary unlock diagnostic.
struct ManifestFinishFailure<E, O, P, Q> {
    kind: ManifestFinishKind<E, O, P, Q>,
    source_attempt: Attempt,
    observer: CommitLockObserver,
}
impl<E, O, P, Q> ManifestFinishFailure<E, O, P, Q> {
    fn manifest_primary(&self) -> Option<&ManifestPrimary<E>> {
        match &self.kind {
            ManifestFinishKind::ManifestFirst { primary, .. } => Some(primary),
            _ => None,
        }
    }
    fn later_failure(&self) -> Option<&CommitLockParentFailure<O, P, Q>> {
        match &self.kind {
            ManifestFinishKind::OwnerFirst { later, .. }
            | ManifestFinishKind::ManifestFirst { later, .. } => later.failure(),
            ManifestFinishKind::Later(failure) => Some(failure),
            _ => None,
        }
    }
    fn post_manifest_report(&self) -> Option<&Report> {
        match &self.kind {
            ManifestFinishKind::OwnerFirst { post_manifest, .. } => Some(post_manifest.report()),
            ManifestFinishKind::ManifestFirst { primary, .. } => {
                Some(primary.post_manifest_report())
            }
            _ => None,
        }
    }
    fn map_manifest_body<U>(self, map: impl FnOnce(E) -> U) -> ManifestFinishFailure<U, O, P, Q> {
        let kind = match self.kind {
            ManifestFinishKind::OwnerFirst {
                post_manifest,
                later,
            } => ManifestFinishKind::OwnerFirst {
                post_manifest,
                later,
            },
            ManifestFinishKind::ManifestFirst { primary, later } => {
                ManifestFinishKind::ManifestFirst {
                    primary: primary.map_body(map),
                    later,
                }
            }
            ManifestFinishKind::Later(failure) => ManifestFinishKind::Later(failure),
            ManifestFinishKind::Local(failure) => ManifestFinishKind::Local(failure),
        };
        ManifestFinishFailure {
            kind,
            source_attempt: self.source_attempt,
            observer: self.observer,
        }
    }
    fn map_owner_body_with<U, V, F>(
        self,
        map: F,
        map_parent: impl FnOnce(Q, F) -> V,
    ) -> ManifestFinishFailure<E, U, P, V>
    where
        F: FnOnce(O) -> U,
    {
        let kind = match self.kind {
            ManifestFinishKind::OwnerFirst {
                post_manifest,
                later,
            } => ManifestFinishKind::OwnerFirst {
                post_manifest,
                later: later.map_body_with(map, map_parent),
            },
            ManifestFinishKind::ManifestFirst { primary, later } => {
                ManifestFinishKind::ManifestFirst {
                    primary,
                    later: later.map_body_with(map, map_parent),
                }
            }
            ManifestFinishKind::Later(failure) => {
                ManifestFinishKind::Later(failure.map_body_with(map, map_parent))
            }
            ManifestFinishKind::Local(failure) => ManifestFinishKind::Local(failure),
        };
        ManifestFinishFailure {
            kind,
            source_attempt: self.source_attempt,
            observer: self.observer,
        }
    }
}
impl<E, O, P, Q> std::fmt::Debug for ManifestFinishFailure<E, O, P, Q> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestFinishFailure")
            .field(
                "manifest_body_present",
                &self
                    .manifest_primary()
                    .and_then(ManifestPrimary::body)
                    .is_some(),
            )
            .field(
                "owner_first",
                &matches!(&self.kind, ManifestFinishKind::OwnerFirst { .. }),
            )
            .field("post_manifest_report", &self.post_manifest_report())
            .field("later_present", &self.later_failure().is_some())
            .field("observations", &self.observer)
            .finish()
    }
}
#[expect(
    clippy::result_large_err,
    reason = "Keep the manifest-primary lane and genuine later owner and parent failures as separate owned values."
)]
fn complete_manifest_parent_with<T, E, P, Q>(
    completion: ManifestOwnerCompletion<T, E>,
    actual_parent: &Attempt,
    finish: impl FnOnce(
        CommitLockCompletion<ManifestPayload<T>, E>,
    ) -> Result<ManifestPayload<T>, CommitLockParentFailure<E, P, Q>>,
) -> Result<T, ManifestFinishFailure<E, E, P, Q>> {
    let ManifestOwnerCompletion {
        source_attempt,
        pending,
        owner,
    } = completion;
    poison_foreign_scopes(
        actual_parent,
        actual_parent,
        &[&source_attempt, &owner.source_attempt],
    );
    pending.poison_against(actual_parent);
    let observer = owner.observer();
    let mut unwind = CheckpointUnwind::new(actual_parent);
    // Existing owner/parent helpers retain their WHOLE actual errors/reports.
    let result = finish(owner);
    let failure = match (pending, result) {
        (ManifestPending::Value, Ok(ManifestPayload::Value(value))) if actual_parent.check() => {
            unwind.disarm();
            return Ok(value);
        }
        (ManifestPending::Value, Err(failure)) => ManifestFinishKind::Later(failure),
        (ManifestPending::Value, Ok(_)) => {
            actual_parent.gap(Gap::RawDailyClose);
            ManifestFinishKind::Local(actual_parent.failure(None))
        }
        (ManifestPending::OwnerFirst(post_manifest), result) => {
            let later = match result {
                Err(failure) => ManifestLater::Failed(failure),
                Ok(_) => {
                    actual_parent.gap(Gap::RawDailyClose);
                    ManifestLater::UnexpectedSuccess(actual_parent.report())
                }
            };
            ManifestFinishKind::OwnerFirst {
                post_manifest,
                later,
            }
        }
        (ManifestPending::ManifestFirst(primary), result) => {
            let later = match result {
                Err(failure) => ManifestLater::Failed(failure),
                Ok(_) => {
                    actual_parent.gap(Gap::RawDailyClose);
                    ManifestLater::UnexpectedSuccess(actual_parent.report())
                }
            };
            ManifestFinishKind::ManifestFirst { primary, later }
        }
    };
    unwind.disarm();
    Err(ManifestFinishFailure {
        kind: failure,
        source_attempt,
        observer,
    })
}
type CheckedManifestFinishFailure = ManifestFinishFailure<
    StoreError,
    StoreError,
    ActualParentFailure<Infallible>,
    ActualParentFailure<StoreError>,
>;
#[expect(
    clippy::result_large_err,
    reason = "The same-parent completion returns every original typed failure and cleanup snapshot intact."
)]
fn complete_manifest_parent_checked<T>(
    completion: ManifestOwnerCompletion<T, StoreError>,
    parent: CheckedCommittedDirectory<'_, '_, '_>,
) -> Result<T, CheckedManifestFinishFailure> {
    let actual = parent.invocation().attempt().clone();
    poison_foreign_scopes(
        &actual,
        parent.expected_attempt(),
        &[
            parent.held_file().attempt(),
            &completion.source_attempt,
            &completion.owner.source_attempt,
        ],
    );
    completion.pending.poison_against(&actual);
    // No guard borrows survive in completion. SAME parent local moves only NOW.
    complete_manifest_parent_with(completion, &actual, move |owner| {
        complete_commit_lock_parent_checked(owner, parent)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedEvidenceBoundary {
    Preflight,
    ParentOpen,
    CommitLockOpen,
    Owner,
    ManifestOpen,
    Manifest,
    Local,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedEvidenceReadStage {
    Scope,
    Kind,
    Admission,
    Limits,
    BeforeRead,
    Read,
    AfterRead,
    Settlement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedEvidencePhase {
    Scope,
    Binding,
    ScopeValidation,
    Limits,
    State,
    Acquire,
    Body,
    Unlock,
    Parent(CheckedCommittedDirectoryPhase),
    Leaf(CheckedCommittedLeafPhase),
    Ancestry {
        phase: CheckedCommittedDirectoryPhase,
        component_index: Option<usize>,
    },
    Read {
        stage: CheckedCommittedEvidenceReadStage,
        checkpoint: Option<CheckedCommittedLeafPhase>,
    },
    Kind,
    Parse,
    Local,
    Settlement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedLockObservation {
    NotAttempted,
    AttemptedMissing,
    Ok,
    Error {
        kind: std::io::ErrorKind,
        raw_os_error: Option<i32>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckedCommittedLockObservations {
    acquire: CheckedCommittedLockObservation,
    unlock: CheckedCommittedLockObservation,
    poisoned: bool,
}
impl CheckedCommittedLockObservations {
    pub fn acquire(&self) -> CheckedCommittedLockObservation {
        self.acquire
    }
    pub fn unlock(&self) -> CheckedCommittedLockObservation {
        self.unlock
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
}
fn strict_lock_observations(observer: &CommitLockObserver) -> CheckedCommittedLockObservations {
    fn project(o: LockObservation) -> CheckedCommittedLockObservation {
        match o {
            LockObservation::NotAttempted => CheckedCommittedLockObservation::NotAttempted,
            LockObservation::AttemptedMissing => CheckedCommittedLockObservation::AttemptedMissing,
            LockObservation::Observed(LockIoSummary::Ok) => CheckedCommittedLockObservation::Ok,
            LockObservation::Observed(LockIoSummary::Error { kind, raw }) => {
                CheckedCommittedLockObservation::Error {
                    kind,
                    raw_os_error: raw,
                }
            }
        }
    }
    let journal = observer.snapshot();
    CheckedCommittedLockObservations {
        acquire: project(journal.acquire),
        unlock: project(journal.unlock),
        poisoned: journal.poisoned,
    }
}
fn strict_owner_phase<E>(
    failure: &CheckedCommitLockFinishFailure<E>,
) -> CheckedCommittedEvidencePhase {
    match &failure.chain {
        CommitLockFailureChain::Leaf(leaf) => CheckedCommittedEvidencePhase::Leaf(leaf.phase()),
        CommitLockFailureChain::Source { source, .. } => match source.phase() {
            CommitLockSourcePhase::State => CheckedCommittedEvidencePhase::State,
            CommitLockSourcePhase::Acquire => CheckedCommittedEvidencePhase::Acquire,
            CommitLockSourcePhase::Unlock => CheckedCommittedEvidencePhase::Unlock,
            CommitLockSourcePhase::Scope => CheckedCommittedEvidencePhase::Scope,
            CommitLockSourcePhase::Checkpoint(p) => CheckedCommittedEvidencePhase::Leaf(p),
            CommitLockSourcePhase::Ancestry {
                phase,
                component_index,
            } => CheckedCommittedEvidencePhase::Ancestry {
                phase,
                component_index,
            },
        },
    }
}
fn strict_manifest_phase<E>(primary: &ManifestPrimary<E>) -> CheckedCommittedEvidencePhase {
    match primary {
        ManifestPrimary::Close(f) => CheckedCommittedEvidencePhase::Leaf(f.phase()),
        ManifestPrimary::Operation { source, .. } => match source.phase() {
            ManifestOperationPhase::Scope => CheckedCommittedEvidencePhase::Scope,
            ManifestOperationPhase::Kind => CheckedCommittedEvidencePhase::Kind,
            ManifestOperationPhase::Admission(p) => CheckedCommittedEvidencePhase::Leaf(p),
            ManifestOperationPhase::Parse => CheckedCommittedEvidencePhase::Parse,
            ManifestOperationPhase::Settlement => CheckedCommittedEvidencePhase::Settlement,
            ManifestOperationPhase::Read(p) => {
                let stage = match p {
                    CheckedCommittedManifestReadPhase::Scope => {
                        CheckedCommittedEvidenceReadStage::Scope
                    }
                    CheckedCommittedManifestReadPhase::Kind => {
                        CheckedCommittedEvidenceReadStage::Kind
                    }
                    CheckedCommittedManifestReadPhase::Admission => {
                        CheckedCommittedEvidenceReadStage::Admission
                    }
                    CheckedCommittedManifestReadPhase::Limits => {
                        CheckedCommittedEvidenceReadStage::Limits
                    }
                    CheckedCommittedManifestReadPhase::BeforeRead => {
                        CheckedCommittedEvidenceReadStage::BeforeRead
                    }
                    CheckedCommittedManifestReadPhase::Read => {
                        CheckedCommittedEvidenceReadStage::Read
                    }
                    CheckedCommittedManifestReadPhase::AfterRead => {
                        CheckedCommittedEvidenceReadStage::AfterRead
                    }
                    CheckedCommittedManifestReadPhase::Settlement => {
                        CheckedCommittedEvidenceReadStage::Settlement
                    }
                };
                CheckedCommittedEvidencePhase::Read {
                    stage,
                    checkpoint: source
                        .read_failure()
                        .and_then(|r| r.checkpoint_failure())
                        .map(|c| c.phase()),
                }
            }
        },
    }
}

// The SAME generic production lane selection/map is exercised below the native
// parent boundary with genuine counted failures. P/Q are DATA there, not native aliases.
enum StrictManifestFailure<E, S, P, PE, PS> {
    Manifest(ManifestFinishFailure<E, S, P, PS>),
    Owner(ManifestFinishFailure<S, E, P, PE>),
    Local {
        failure: Failure<Infallible>,
        source_attempt: Attempt,
        observer: CommitLockObserver,
    },
}
fn normalize_strict_manifest_failure<E, P, Q>(
    failure: ManifestFinishFailure<E, E, P, Q>,
) -> StrictManifestFailure<E, E, P, Q, Q> {
    // Select BEFORE mapping. ManifestFirst.later is always secondary.
    match &failure.kind {
        ManifestFinishKind::ManifestFirst { .. } => StrictManifestFailure::Manifest(failure),
        ManifestFinishKind::OwnerFirst { .. } | ManifestFinishKind::Later(_) => {
            StrictManifestFailure::Owner(failure)
        }
        ManifestFinishKind::Local(_) => {
            let ManifestFinishFailure {
                kind,
                source_attempt,
                observer,
            } = failure;
            match kind {
                ManifestFinishKind::Local(failure) => StrictManifestFailure::Local {
                    failure,
                    source_attempt,
                    observer,
                },
                _ => unreachable!("private selected Local lane"),
            }
        }
    }
}
impl<E, S, P, PE, PS> StrictManifestFailure<E, S, P, PE, PS> {
    fn map_body_with<U, PU, F>(
        self,
        map: F,
        map_parent: impl FnOnce(PE, F) -> PU,
    ) -> StrictManifestFailure<U, S, P, PU, PS>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Self::Manifest(f) => StrictManifestFailure::Manifest(f.map_manifest_body(map)),
            Self::Owner(f) => StrictManifestFailure::Owner(f.map_owner_body_with(map, map_parent)),
            Self::Local {
                failure,
                source_attempt,
                observer,
            } => StrictManifestFailure::Local {
                failure,
                source_attempt,
                observer,
            },
        }
    }
}
type NativeStrictManifestFailure<E> = StrictManifestFailure<
    E,
    StoreError,
    ActualParentFailure<Infallible>,
    ActualParentFailure<E>,
    ActualParentFailure<StoreError>,
>;

enum StrictEarlyFailure<E> {
    Admission {
        failure: Failure<E>,
        source_attempt: Attempt,
        phase: CheckedCommittedEvidencePhase,
    },
    Pure {
        body: E,
        source_attempt: Attempt,
        phase: CheckedCommittedEvidencePhase,
    },
    Execution {
        original: Option<Box<StrictEarlyFailure<E>>>,
        failure: crate::ExecutionGateFailure,
        local: Result<(), Failure<Infallible>>,
        source_attempt: Attempt,
    },
}
impl<E> StrictEarlyFailure<E> {
    fn body(&self) -> Option<&E> {
        match self {
            Self::Admission { failure, .. } => failure.body(),
            Self::Pure { body, .. } => Some(body),
            Self::Execution { original, .. } => original.as_ref().and_then(|first| first.body()),
        }
    }
    fn report(&self) -> Option<&Report> {
        match self {
            Self::Admission { failure, .. } => Some(failure.report()),
            Self::Pure { .. } => None,
            Self::Execution {
                original, failure, ..
            } => match original {
                Some(first) => first.report(),
                None => failure.source_report(),
            },
        }
    }
    fn phase(&self) -> Option<CheckedCommittedEvidencePhase> {
        match self {
            Self::Admission { phase, .. } | Self::Pure { phase, .. } => Some(*phase),
            Self::Execution { original, .. } => original.as_ref().and_then(|first| first.phase()),
        }
    }
    fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match self {
            Self::Execution { failure, .. } => Some(failure),
            _ => None,
        }
    }
    fn map_body<U>(self, map: impl FnOnce(E) -> U) -> StrictEarlyFailure<U> {
        match self {
            Self::Admission {
                failure,
                source_attempt,
                phase,
            } => StrictEarlyFailure::Admission {
                failure: failure.map_body(map),
                source_attempt,
                phase,
            },
            Self::Pure {
                body,
                source_attempt,
                phase,
            } => StrictEarlyFailure::Pure {
                body: map(body),
                source_attempt,
                phase,
            },
            Self::Execution {
                original,
                failure,
                local,
                source_attempt,
            } => StrictEarlyFailure::Execution {
                original: original.map(|first| Box::new(first.map_body(map))),
                failure,
                local,
                source_attempt,
            },
        }
    }
}
// These private callbacks feed the actual per-call typed gate result.
fn strict_reader_preflight<E>(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    pure: impl FnOnce() -> Result<(), (E, CheckedCommittedEvidencePhase)>,
    gate: impl FnOnce() -> Result<(), super::super::close_gate::NativeAdmissionFailure>,
) -> Result<(), StrictEarlyFailure<E>> {
    let actual = invocation.attempt().clone();
    poison_foreign_scopes(&actual, expected, &[]);
    let refusal = |phase| StrictEarlyFailure::Admission {
        failure: invocation.refused(Gap::RawDailyClose),
        source_attempt: actual.clone(),
        phase,
    };
    let original = if actual.check() {
        Ok(())
    } else {
        Err(refusal(CheckedCommittedEvidencePhase::Scope))
    };
    let execution = invocation.compare_native_context_scopes(expected);
    crate::raw_custody::join_native_checks(original, execution, |original, failure| {
        StrictEarlyFailure::Execution {
            original: original.map(Box::new),
            failure,
            local: invocation.settle_local_in(expected, Ok::<(), Infallible>(())),
            source_attempt: actual.clone(),
        }
    })?;
    if let Err((body, phase)) = pure() {
        actual.gap(Gap::RawDailyClose);
        return Err(StrictEarlyFailure::Pure {
            body,
            source_attempt: actual,
            phase,
        });
    }
    if !actual.check() {
        return Err(refusal(CheckedCommittedEvidencePhase::Scope));
    }
    match gate() {
        Ok(()) => {}
        Err(super::super::close_gate::NativeAdmissionFailure::Unbound) => {
            return Err(refusal(CheckedCommittedEvidencePhase::Binding));
        }
        Err(super::super::close_gate::NativeAdmissionFailure::Execution(failure)) => {
            return Err(StrictEarlyFailure::Execution {
                original: None,
                failure,
                local: invocation.settle_local_in(expected, Ok::<(), Infallible>(())),
                source_attempt: actual,
            });
        }
    }
    invocation
        .settle_local_in(expected, Ok(()))
        .map_err(|failure| StrictEarlyFailure::Admission {
            failure,
            source_attempt: actual,
            phase: CheckedCommittedEvidencePhase::Scope,
        })
}

#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum StrictEvidenceFailureKind<E> {
    Early(StrictEarlyFailure<E>),
    ParentOpen {
        failure: ActualParentFailure<E>,
        source_attempt: Attempt,
    },
    LockOpen(CheckedCommittedLeafParentFailure<E>),
    Owner(CheckedCommitLockParentFailure<E>),
    ManifestOpen(CheckedManifestOpenFailure<E>),
    Manifest(NativeStrictManifestFailure<E>),
}
/// Whole actual errors after their real local cleanup; diagnostics never grant
/// origin, rights, lock ownership, publication or further effect permission.
pub struct CheckedCommittedEvidenceFailure<E = StoreError> {
    inner: StrictEvidenceFailureKind<E>,
}

struct StrictOwnerDiagnostics<'a> {
    source: Option<&'a Report>,
    phase: CheckedCommittedEvidencePhase,
    post_unlock: Option<&'a Report>,
    post_lock: Option<&'a Report>,
    post_lock_phase: Option<CheckedCommittedLeafPhase>,
    post_parent: &'a Report,
    parent_phase: Option<CheckedCommittedDirectoryPhase>,
    parent_index: Option<usize>,
    unlock_kind: Option<std::io::ErrorKind>,
    unlock_raw: Option<i32>,
    observations: CheckedCommittedLockObservations,
}
fn strict_owner_diagnostics<E>(
    failure: &CheckedCommitLockParentFailure<E>,
) -> StrictOwnerDiagnostics<'_> {
    let owner = failure.owner_failure();
    StrictOwnerDiagnostics {
        source: failure.source_report(),
        phase: match owner {
            Some(owner) => strict_owner_phase(owner),
            None => CheckedCommittedEvidencePhase::Parent(
                failure.parent_failure().expect("actual ParentOnly").phase(),
            ),
        },
        post_unlock: owner.map(|f| f.post_unlock_report()),
        post_lock: owner.map(|f| f.post_leaf_report()),
        post_lock_phase: owner.and_then(|f| f.post_leaf_phase()),
        post_parent: failure.post_parent_report(),
        parent_phase: failure.parent_phase(),
        parent_index: failure.parent_component_index(),
        unlock_kind: owner.and_then(|f| f.secondary_unlock_kind()),
        unlock_raw: owner.and_then(|f| f.secondary_unlock_raw_os_error()),
        observations: strict_lock_observations(&failure.observer),
    }
}
impl<E> CheckedCommittedEvidenceFailure<E> {
    pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        match &self.inner {
            StrictEvidenceFailureKind::Early(f) => f.execution_failure(),
            StrictEvidenceFailureKind::ParentOpen { failure, .. } => failure.execution_failure(),
            StrictEvidenceFailureKind::LockOpen(f) => f.execution_failure(),
            StrictEvidenceFailureKind::Owner(f) => f.execution_failure(),
            StrictEvidenceFailureKind::ManifestOpen(f) => f.execution_failure(),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => f
                .manifest_primary()
                .and_then(ManifestPrimary::execution_failure),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                f.later_failure().and_then(|f| f.execution_failure())
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local { .. }) => None,
        }
    }
    pub fn execution_local_result(&self) -> Option<&Result<(), Failure<Infallible>>> {
        match &self.inner {
            StrictEvidenceFailureKind::Early(StrictEarlyFailure::Execution { local, .. }) => {
                Some(local)
            }
            _ => None,
        }
    }
    pub fn body(&self) -> Option<&E> {
        match &self.inner {
            StrictEvidenceFailureKind::Early(failure) => failure.body(),
            StrictEvidenceFailureKind::ParentOpen { failure, .. } => failure.body(),
            StrictEvidenceFailureKind::LockOpen(f) => f.body(),
            StrictEvidenceFailureKind::Owner(f) => f.body(),
            StrictEvidenceFailureKind::ManifestOpen(f) => f.body(),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                f.manifest_primary().and_then(|p| p.body())
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                f.later_failure().and_then(|p| p.body())
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local { .. }) => None,
        }
    }
    pub fn boundary(&self) -> CheckedCommittedEvidenceBoundary {
        match &self.inner {
            StrictEvidenceFailureKind::Early(_) => CheckedCommittedEvidenceBoundary::Preflight,
            StrictEvidenceFailureKind::ParentOpen { .. } => {
                CheckedCommittedEvidenceBoundary::ParentOpen
            }
            StrictEvidenceFailureKind::LockOpen(_) => {
                CheckedCommittedEvidenceBoundary::CommitLockOpen
            }
            StrictEvidenceFailureKind::Owner(_) => CheckedCommittedEvidenceBoundary::Owner,
            StrictEvidenceFailureKind::ManifestOpen(_) => {
                CheckedCommittedEvidenceBoundary::ManifestOpen
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local { .. }) => {
                CheckedCommittedEvidenceBoundary::Local
            }
            StrictEvidenceFailureKind::Manifest(_) => CheckedCommittedEvidenceBoundary::Manifest,
        }
    }
    // A pure error before acquisition has NO resource report. Nothing is recaptured.
    pub fn source_report(&self) -> Option<&Report> {
        match &self.inner {
            StrictEvidenceFailureKind::Early(failure) => failure.report(),
            StrictEvidenceFailureKind::ParentOpen { failure, .. } => failure.source_report(),
            StrictEvidenceFailureKind::LockOpen(f) => f.source().source_report(),
            StrictEvidenceFailureKind::Owner(f) => f.source_report(),
            StrictEvidenceFailureKind::ManifestOpen(f) => f.source_report(),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => f
                .manifest_primary()
                .and_then(ManifestPrimary::source_report),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                f.later_failure().and_then(|p| p.source_report())
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local {
                failure, ..
            }) => Some(failure.report()),
        }
    }
    fn owner_diagnostics(&self) -> Option<StrictOwnerDiagnostics<'_>> {
        match &self.inner {
            StrictEvidenceFailureKind::Owner(f) => Some(strict_owner_diagnostics(f)),
            StrictEvidenceFailureKind::ManifestOpen(f) => match &f.kind {
                ManifestOpenFailureKind::OpenFirst {
                    later: PostManifestOpen::Failed(f),
                    ..
                } => Some(strict_owner_diagnostics(f)),
                ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(f)) => {
                    Some(strict_owner_diagnostics(f))
                }
                _ => None,
            },
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                f.later_failure().map(strict_owner_diagnostics)
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                f.later_failure().map(strict_owner_diagnostics)
            }
            _ => None,
        }
    }
    pub fn phase(&self) -> Option<CheckedCommittedEvidencePhase> {
        match &self.inner {
            StrictEvidenceFailureKind::Early(failure) => failure.phase(),
            StrictEvidenceFailureKind::ParentOpen { failure, .. } => {
                Some(CheckedCommittedEvidencePhase::Parent(failure.phase()))
            }
            StrictEvidenceFailureKind::LockOpen(f) => {
                Some(CheckedCommittedEvidencePhase::Leaf(f.source().phase()))
            }
            StrictEvidenceFailureKind::ManifestOpen(f) => f
                .open_source()
                .map(|s| CheckedCommittedEvidencePhase::Leaf(s.phase()))
                .or_else(|| self.owner_diagnostics().map(|d| d.phase)),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                f.manifest_primary().map(strict_manifest_phase)
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local { .. }) => {
                Some(CheckedCommittedEvidencePhase::Local)
            }
            _ => self.owner_diagnostics().map(|d| d.phase),
        }
    }
    pub fn leaf_kind(&self) -> Option<CheckedCommittedLeafKind> {
        match &self.inner {
            StrictEvidenceFailureKind::LockOpen(f) => Some(f.source().kind()),
            StrictEvidenceFailureKind::Owner(f) => f
                .owner_failure()
                .map(|_| CheckedCommittedLeafKind::CommitLock),
            StrictEvidenceFailureKind::ManifestOpen(f) => match &f.kind {
                ManifestOpenFailureKind::OpenFirst { source, .. } => Some(source.kind()),
                ManifestOpenFailureKind::OwnerOnly(PostManifestOpen::Failed(f)) => f
                    .owner_failure()
                    .map(|_| CheckedCommittedLeafKind::CommitLock),
                _ => None,
            },
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(_)) => {
                Some(CheckedCommittedLeafKind::Manifest)
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => f
                .later_failure()
                .and_then(|f| f.owner_failure())
                .map(|_| CheckedCommittedLeafKind::CommitLock),
            _ => None,
        }
    }
    pub fn component_index(&self) -> Option<usize> {
        match self.phase() {
            Some(CheckedCommittedEvidencePhase::Ancestry {
                component_index, ..
            }) => component_index,
            Some(CheckedCommittedEvidencePhase::Parent(_)) => match &self.inner {
                StrictEvidenceFailureKind::ParentOpen { failure, .. } => failure.component_index(),
                _ => self.owner_diagnostics().and_then(|d| d.parent_index),
            },
            _ => None,
        }
    }
    pub fn read_report(&self) -> Option<&Report> {
        match &self.inner {
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                match f.manifest_primary()? {
                    ManifestPrimary::Operation { source, .. } => {
                        source.read_failure().map(|r| r.report())
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    pub fn checkpoint_report(&self) -> Option<&Report> {
        match &self.inner {
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                match f.manifest_primary()? {
                    ManifestPrimary::Operation { source, .. } => source
                        .read_failure()
                        .and_then(|r| r.checkpoint_failure())
                        .map(|c| c.report()),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    pub fn post_manifest_report(&self) -> Option<&Report> {
        match &self.inner {
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                f.post_manifest_report()
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                f.post_manifest_report()
            }
            _ => None, // failed open has no Manifest owner/close snapshot
        }
    }
    pub fn post_manifest_phase(&self) -> Option<CheckedCommittedLeafPhase> {
        fn phase<E, O, P, Q>(
            f: &ManifestFinishFailure<E, O, P, Q>,
        ) -> Option<CheckedCommittedLeafPhase> {
            match &f.kind {
                ManifestFinishKind::OwnerFirst { post_manifest, .. } => post_manifest.phase(),
                ManifestFinishKind::ManifestFirst { primary, .. } => match primary {
                    ManifestPrimary::Operation { post_manifest, .. } => post_manifest.phase(),
                    ManifestPrimary::Close(f) => Some(f.phase()),
                },
                _ => None,
            }
        }
        match &self.inner {
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => phase(f),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => phase(f),
            _ => None,
        }
    }
    pub fn owner_source_phase(&self) -> Option<CheckedCommittedEvidencePhase> {
        self.owner_diagnostics().map(|d| d.phase)
    }
    pub fn post_lock_phase(&self) -> Option<CheckedCommittedLeafPhase> {
        self.owner_diagnostics().and_then(|d| d.post_lock_phase)
    }
    pub fn owner_source_report(&self) -> Option<&Report> {
        self.owner_diagnostics().and_then(|d| d.source)
    }
    pub fn post_unlock_report(&self) -> Option<&Report> {
        self.owner_diagnostics().and_then(|d| d.post_unlock)
    }
    pub fn post_lock_report(&self) -> Option<&Report> {
        self.owner_diagnostics().and_then(|d| d.post_lock)
    }
    pub fn post_parent_report(&self) -> Option<&Report> {
        match &self.inner {
            StrictEvidenceFailureKind::LockOpen(f) => Some(f.post_parent_report()),
            StrictEvidenceFailureKind::ManifestOpen(f) => Some(f.post_parent_report()),
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                match &f.kind {
                    ManifestFinishKind::ManifestFirst {
                        later: ManifestLater::UnexpectedSuccess(report),
                        ..
                    } => Some(report),
                    _ => self.owner_diagnostics().map(|d| d.post_parent),
                }
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => match &f.kind {
                ManifestFinishKind::OwnerFirst {
                    later: ManifestLater::UnexpectedSuccess(report),
                    ..
                } => Some(report),
                _ => self.owner_diagnostics().map(|d| d.post_parent),
            },
            _ => self.owner_diagnostics().map(|d| d.post_parent),
        }
    }
    pub fn parent_phase(&self) -> Option<CheckedCommittedDirectoryPhase> {
        match &self.inner {
            StrictEvidenceFailureKind::LockOpen(f) => f.parent_phase(),
            _ => self.owner_diagnostics().and_then(|d| d.parent_phase),
        }
    }
    pub fn parent_component_index(&self) -> Option<usize> {
        match &self.inner {
            StrictEvidenceFailureKind::LockOpen(f) => f.parent_component_index(),
            _ => self.owner_diagnostics().and_then(|d| d.parent_index),
        }
    }
    pub fn secondary_unlock_kind(&self) -> Option<std::io::ErrorKind> {
        self.owner_diagnostics().and_then(|d| d.unlock_kind)
    }
    pub fn secondary_unlock_raw_os_error(&self) -> Option<i32> {
        self.owner_diagnostics().and_then(|d| d.unlock_raw)
    }
    pub fn observations(&self) -> Option<CheckedCommittedLockObservations> {
        match &self.inner {
            StrictEvidenceFailureKind::ManifestOpen(f) => {
                Some(strict_lock_observations(&f.observer))
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Manifest(f)) => {
                Some(strict_lock_observations(&f.observer))
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Owner(f)) => {
                Some(strict_lock_observations(&f.observer))
            }
            StrictEvidenceFailureKind::Manifest(StrictManifestFailure::Local {
                observer, ..
            }) => Some(strict_lock_observations(observer)),
            _ => self.owner_diagnostics().map(|d| d.observations),
        }
    }
    pub fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommittedEvidenceFailure<U> {
        let inner = match self.inner {
            StrictEvidenceFailureKind::Early(f) => {
                StrictEvidenceFailureKind::Early(f.map_body(map))
            }
            StrictEvidenceFailureKind::ParentOpen {
                failure,
                source_attempt,
            } => StrictEvidenceFailureKind::ParentOpen {
                failure: failure.map_body(map),
                source_attempt,
            },
            StrictEvidenceFailureKind::LockOpen(f) => {
                StrictEvidenceFailureKind::LockOpen(f.map_body(map))
            }
            StrictEvidenceFailureKind::Owner(f) => {
                StrictEvidenceFailureKind::Owner(f.map_body(map))
            }
            StrictEvidenceFailureKind::ManifestOpen(f) => {
                StrictEvidenceFailureKind::ManifestOpen(f.map_body(map))
            }
            StrictEvidenceFailureKind::Manifest(f) => StrictEvidenceFailureKind::Manifest(
                f.map_body_with(map, |parent, map| parent.map_body(map)),
            ),
        };
        CheckedCommittedEvidenceFailure { inner }
    }
}
impl<E> std::fmt::Debug for CheckedCommittedEvidenceFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedEvidenceFailure")
            .field("body_present", &self.body().is_some())
            .field("boundary", &self.boundary())
            .field("phase", &self.phase())
            .field("leaf_kind", &self.leaf_kind())
            .field("component_index", &self.component_index())
            .field("source_report", &self.source_report())
            .field("read_report", &self.read_report())
            .field("checkpoint_report", &self.checkpoint_report())
            .field("post_manifest_report", &self.post_manifest_report())
            .field("owner_source_report", &self.owner_source_report())
            .field("post_unlock_report", &self.post_unlock_report())
            .field("post_lock_report", &self.post_lock_report())
            .field("post_parent_report", &self.post_parent_report())
            .field("post_manifest_phase", &self.post_manifest_phase())
            .field("post_lock_phase", &self.post_lock_phase())
            .field("owner_source_phase", &self.owner_source_phase())
            .field("parent_phase", &self.parent_phase())
            .field("parent_component_index", &self.parent_component_index())
            .field("secondary_unlock_kind", &self.secondary_unlock_kind())
            .field(
                "secondary_unlock_raw_os_error",
                &self.secondary_unlock_raw_os_error(),
            )
            .field("observations", &self.observations())
            .finish()
    }
}

// One pre-open sequencing seam; actual closures invoke accepted native methods.
// No model-only operation loop or caller-provided cleanup permission.
#[expect(
    clippy::result_large_err,
    reason = "An early refusal returns the owning completion rather than detaching cleanup or replacing its reports."
)]
fn prepare_strict_reader_owner<L: CommitLockLeaf, E>(
    owner: Result<CommitLockOwner<L, E>, CommitLockOwner<L, E>>,
    hook: impl FnOnce(&L),
    ancestry: impl FnOnce(&mut CommitLockOwner<L, E>) -> Result<(), ()>,
    acquire: impl FnOnce(&mut CommitLockOwner<L, E>) -> Result<(), ()>,
    held_ancestry: impl FnOnce(&mut CommitLockOwner<L, E>) -> Result<(), ()>,
    held_checkpoint: impl FnOnce(&mut CommitLockOwner<L, E>) -> Result<(), ()>,
    finish: impl FnOnce(CommitLockOwner<L, E>) -> CommitLockCompletion<(), E>,
) -> Result<CommitLockOwner<L, E>, CommitLockCompletion<(), E>> {
    let mut owner = match owner {
        Ok(owner) => owner,
        Err(owner) => return Err(finish(owner)),
    };
    let actual = owner.source_attempt.clone();
    let mut unwind = CheckpointUnwind::new(&actual);
    hook(owner.leaf()); // actual owner exists BEFORE original void hook
    let result = ancestry(&mut owner)
        .and_then(|()| acquire(&mut owner))
        .and_then(|()| held_ancestry(&mut owner))
        .and_then(|()| held_checkpoint(&mut owner));
    owner.poison_scopes();
    let result = if result.is_err() || owner.source.is_some() || !actual.check() {
        if owner.source.is_none() {
            owner.refuse(CommitLockSourcePhase::Scope);
        }
        Err(finish(owner))
    } else {
        Ok(owner)
    };
    unwind.disarm();
    result
}
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the original hook, read, ancestry, checkpoint and parser callbacks explicit at one borrowed operation boundary."
)]
#[expect(
    clippy::result_large_err,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
fn strict_reader_manifest_body<M, L, E, B, T>(
    manifest: &M,
    owner: &mut CommitLockOwner<L, E>,
    context: &ManifestOperationContext<'_, '_>,
    hook: impl FnOnce(&M),
    read: impl FnOnce(&M) -> Result<B, CheckedCommittedManifestReadFailure<E>>,
    ancestry: impl FnOnce(&mut CommitLockOwner<L, E>),
    checkpoint: impl FnOnce(&mut CommitLockOwner<L, E>),
    parse: impl FnOnce(B) -> Result<T, E>,
) -> Result<T, ManifestOperationFailure<E>>
where
    L: CommitLockLeaf,
{
    hook(manifest);
    let bytes = context.read(read(manifest))?;
    context.owner_step(owner, ancestry)?;
    context.owner_step(owner, checkpoint)?;
    context.local(ManifestOperationPhase::Parse, parse(bytes))
}
// Every variant OWNS only pending/completion/error data, never a native borrow.
enum StrictReaderPending<L, O, M, C> {
    LockOpen(L),
    Owner(O),
    ManifestOpen(M),
    Manifest(C),
}
fn complete_strict_reader_pending<P, L, O, M, C, T, F>(
    pending: StrictReaderPending<L, O, M, C>,
    parent: P,
    lock: impl FnOnce(L, P) -> F,
    owner: impl FnOnce(O, P) -> Result<T, F>,
    open: impl FnOnce(M, P) -> F,
    manifest: impl FnOnce(C, P) -> Result<T, F>,
) -> Result<T, F> {
    match pending {
        StrictReaderPending::LockOpen(source) => Err(lock(source, parent)),
        StrictReaderPending::Owner(completion) => owner(completion, parent),
        StrictReaderPending::ManifestOpen(pending) => Err(open(pending, parent)),
        StrictReaderPending::Manifest(completion) => manifest(completion, parent),
    }
}
type NativeStrictReaderPending<T = Vec<super::CommittedManifestEvidenceRecord>> =
    StrictReaderPending<
        CheckedCommittedLeafFailure,
        CommitLockCompletion<(), StoreError>,
        ManifestOpenPending<StoreError>,
        ManifestOwnerCompletion<T, StoreError>,
    >;
// Borrowed native phase cannot move the retained parent. Return only owned data.

impl super::RawStore {
    /// Strict evidence only. Immutable unbound gate still refuses all native
    /// effects. UID/limits are explicit policy; neither is origin authority.
    #[expect(
        clippy::result_large_err,
        reason = "Return the whole checked-reader failure including original Some or None and all real cleanup snapshots."
    )]
    pub fn read_committed_manifest_evidence_checked(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
        limits: &super::CommittedManifestReadLimits,
    ) -> Result<
        Vec<super::CommittedManifestEvidenceRecord>,
        CheckedCommittedEvidenceFailure<StoreError>,
    > {
        self.read_committed_manifest_evidence_checked_with_ops(
            invocation,
            expected,
            provider,
            market,
            trusted_raw_owner_uid,
            limits,
            &super::SystemCommittedManifestReadOps,
        )
    }
    #[expect(
        clippy::result_large_err,
        clippy::too_many_arguments,
        reason = "Keep caller scope, policy and hooks explicit while returning the whole reader error without a new allocation boundary."
    )]
    fn read_committed_manifest_evidence_checked_with_ops<
        O: super::CommittedManifestReadOps + ?Sized,
    >(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
        limits: &super::CommittedManifestReadLimits,
        operations: &O,
    ) -> Result<
        Vec<super::CommittedManifestEvidenceRecord>,
        CheckedCommittedEvidenceFailure<StoreError>,
    > {
        self.read_committed_manifest_evidence_checked_with_parser(
            invocation,
            expected,
            provider,
            market,
            trusted_raw_owner_uid,
            limits,
            operations,
            |path, bytes| {
                super::parse_committed_manifest_evidence(
                    provider,
                    market,
                    path,
                    &bytes,
                    limits.max_records,
                )
            },
        )
    }
}

#[expect(
    clippy::result_large_err,
    reason = "Nested manifest callbacks return the original read and operation failures so the owning completion preserves the first source."
)]
fn strict_reader_under_parent_with_parser<O: super::CommittedManifestReadOps + ?Sized, T>(
    parent: &CheckedCommittedDirectory<'_, '_, '_>,
    limits: &super::CommittedManifestReadLimits,
    operations: &O,
    parse: impl FnOnce(&std::path::Path, Vec<u8>) -> Result<T, StoreError>,
) -> NativeStrictReaderPending<T> {
    let lock = match parent.open_leaf_checked(CheckedCommittedLeafKind::CommitLock) {
        Ok(lock) => lock,
        Err(source) => return StrictReaderPending::LockOpen(source),
    };
    let owner = prepare_strict_reader_owner(
        CommitLockOwner::new(lock),
        |leaf| operations.after_control_open("commit.lock", &leaf.path),
        |owner| owner.revalidate_ancestry_checked(),
        |owner| owner.acquire_checked(), // includes the ONE initial node checkpoint
        |owner| owner.revalidate_ancestry_checked(),
        |owner| owner.checkpoint_held_checked(),
        |owner| owner.finish_for_parent_checked(Ok(())),
    );
    let owner = match owner {
        Ok(owner) => owner,
        Err(completion) => return StrictReaderPending::Owner(completion),
    };
    let opened = owner.open_manifest_held_checked();
    let (mut owner, manifest) = match opened {
        Ok(pair) => pair,
        Err(pending) => return StrictReaderPending::ManifestOpen(pending),
    };
    let completion =
        manifest.finish_manifest_operation_checked(&mut owner, |manifest, owner, context| {
            strict_reader_manifest_body(
                manifest,
                owner,
                context,
                |manifest| operations.after_control_open("manifest.jsonl", &manifest.path),
                |manifest| manifest.read_manifest_bounded_checked(limits),
                |owner| {
                    let _ = owner.revalidate_ancestry_checked();
                },
                |owner| {
                    let _ = owner.checkpoint_held_checked();
                },
                |bytes| parse(&manifest.path, bytes),
            )
        });
    StrictReaderPending::Manifest(completion.finish_owner_checked(owner))
}
impl super::RawStore {
    #[expect(
        clippy::result_large_err,
        clippy::too_many_arguments,
        reason = "Keep invocation, expected scope, UID, limits and parse strategy explicit while preserving the whole native completion error."
    )]
    fn read_committed_manifest_evidence_checked_with_parser<
        O: super::CommittedManifestReadOps + ?Sized,
        T,
    >(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
        limits: &super::CommittedManifestReadLimits,
        operations: &O,
        parse: impl FnOnce(&std::path::Path, Vec<u8>) -> Result<T, StoreError>,
    ) -> Result<T, CheckedCommittedEvidenceFailure<StoreError>> {
        strict_reader_preflight(
            invocation,
            expected,
            || {
                super::validate_scope(provider, market)
                    .map_err(|e| (e, CheckedCommittedEvidencePhase::ScopeValidation))?;
                super::committed_manifest_read_buffer_len(limits)
                    .map_err(|e| (e, CheckedCommittedEvidencePhase::Limits))?;
                Ok(())
            },
            || crate::close_gate::require_native(invocation, expected),
        )
        .map_err(|failure| CheckedCommittedEvidenceFailure {
            inner: StrictEvidenceFailureKind::Early(failure),
        })?;
        let source_attempt = invocation.attempt().clone(); // BEFORE actual native parent producer
        let parent = self
            .open_committed_manifest_directory_checked(
                invocation,
                expected,
                provider,
                market,
                trusted_raw_owner_uid,
            )
            .map_err(|failure| CheckedCommittedEvidenceFailure {
                inner: StrictEvidenceFailureKind::ParentOpen {
                    failure,
                    source_attempt,
                },
            })?;
        let pending = strict_reader_under_parent_with_parser(&parent, limits, operations, parse);
        // The borrowed phase has RETURNED: owner/Manifest/Result/context borrows
        // are over. Only NOW move this SAME physical parent local, never a copy.
        complete_strict_reader_pending(
            pending,
            parent,
            |source, parent| CheckedCommittedEvidenceFailure {
                inner: StrictEvidenceFailureKind::LockOpen(
                    complete_committed_leaf_source_with_parent(parent, source),
                ),
            },
            |completion, parent| {
                let actual = parent.invocation().attempt().clone();
                let observer = completion.observer(); // genuine ticket BEFORE consumption
                match complete_commit_lock_parent_checked(completion, parent) {
                    Err(failure) => Err(CheckedCommittedEvidenceFailure {
                        inner: StrictEvidenceFailureKind::Owner(failure),
                    }),
                    Ok(()) => {
                        actual.gap(Gap::RawDailyClose);
                        Err(CheckedCommittedEvidenceFailure {
                            inner: StrictEvidenceFailureKind::Manifest(
                                StrictManifestFailure::Local {
                                    failure: actual.failure(None),
                                    source_attempt: actual,
                                    observer,
                                },
                            ),
                        })
                    }
                }
            },
            |pending, parent| CheckedCommittedEvidenceFailure {
                inner: StrictEvidenceFailureKind::ManifestOpen(complete_manifest_open_checked(
                    pending, parent,
                )),
            },
            |completion, parent| {
                complete_manifest_parent_checked(completion, parent).map_err(|failure| {
                    CheckedCommittedEvidenceFailure {
                        inner: StrictEvidenceFailureKind::Manifest(
                            normalize_strict_manifest_failure(failure),
                        ),
                    }
                })
            },
        )
    }
    /// Manifest-only observation with explicit policy and the same strict cleanup.
    #[expect(
        clippy::result_large_err,
        reason = "Return the whole reader failure with only genuinely returned rows without reconstructing or allocating a replacement source."
    )]
    pub fn read_committed_manifest_evidence_observed_checked(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &Attempt,
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
        limits: &super::CommittedManifestReadLimits,
    ) -> Result<
        super::CheckedCommittedEvidenceObservation,
        super::CheckedCommittedEvidenceObservationFailure<StoreError>,
    > {
        super::strict_manifest_observation::observe_native(|observation| {
            self.read_committed_manifest_evidence_checked_with_parser(
                invocation,
                expected,
                provider,
                market,
                trusted_raw_owner_uid,
                limits,
                &super::SystemCommittedManifestReadOps,
                |path, bytes| {
                    observation.parse_once(|| {
                        super::parse_committed_manifest_evidence_retained(
                            provider,
                            market,
                            path,
                            &bytes,
                            limits.max_records,
                        )
                    })
                },
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counted {
        label: &'static str,
        events: Rc<RefCell<Vec<&'static str>>>,
    }
    impl Close for Counted {
        fn close(self) -> Result<(), crate::checked_close::Errno> {
            self.events.borrow_mut().push(self.label);
            Ok(())
        }
    }
    fn counted(label: &'static str, events: &Rc<RefCell<Vec<&'static str>>>) -> Counted {
        Counted {
            label,
            events: Rc::clone(events),
        }
    }
    struct HookCounter(AtomicUsize);
    impl super::super::CommittedManifestReadOps for HookCounter {
        fn after_control_open(&self, _leaf: &str, _path: &std::path::Path) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct MappedError(StoreError);
    struct Payload(Box<str>);

    #[test]
    fn public_readers_refuse_unbound_before_hooks_or_resources() {
        // These are explicit refusal-only data, not native UID or path fixtures.
        let store = super::super::RawStore::new("never-opened-raw-root");
        let limits = super::super::CommittedManifestReadLimits {
            max_manifest_bytes: 128,
            max_records: 3,
        };
        let checked_attempt = Attempt::default();
        let checked_invocation = RawInvocation::new(&checked_attempt);
        let failure = match store.read_committed_manifest_evidence_checked(
            &checked_invocation,
            &checked_attempt,
            "krx",
            "kr",
            0,
            &limits,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("the immutable gate must refuse the public checked reader"),
        };
        assert_eq!(
            failure.boundary(),
            CheckedCommittedEvidenceBoundary::Preflight
        );
        assert_eq!(
            failure.phase(),
            Some(CheckedCommittedEvidencePhase::Binding)
        );
        assert!(failure.body().is_none());
        let report = failure.source_report().expect("actual gate refusal report");
        assert_eq!(report.attempted(), 0);
        assert_eq!(report.live(), 0);
        assert!(failure.observations().is_none());

        let observed_attempt = Attempt::default();
        let observed_invocation = RawInvocation::new(&observed_attempt);
        let observed = match store.read_committed_manifest_evidence_observed_checked(
            &observed_invocation,
            &observed_attempt,
            "krx",
            "kr",
            0,
            &limits,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("the public observation must retain its real gate failure"),
        };
        assert_eq!(
            observed.parse_state(),
            super::super::CheckedCommittedEvidenceParseState::NotParsed
        );
        assert!(observed.records().is_none());
        assert!(observed.source().body().is_none());
        assert_eq!(
            observed.source().phase(),
            Some(CheckedCommittedEvidencePhase::Binding)
        );
        assert_eq!(observed.source().source_report().unwrap().attempted(), 0);

        let hook_attempt = Attempt::default();
        let hook_invocation = RawInvocation::new(&hook_attempt);
        let hooks = HookCounter(AtomicUsize::new(0));
        assert!(
            store
                .read_committed_manifest_evidence_checked_with_ops(
                    &hook_invocation,
                    &hook_attempt,
                    "krx",
                    "kr",
                    0,
                    &limits,
                    &hooks,
                )
                .is_err()
        );
        assert_eq!(hooks.0.load(Ordering::SeqCst), 0);
        assert_eq!(hook_attempt.report().attempted(), 0);
        assert_eq!(hook_attempt.report().live(), 0);
    }

    #[test]
    fn foreign_scope_is_poisoned_even_dirty_before_limit_admission() {
        let store = super::super::RawStore::new("never-opened-raw-root");
        let actual = Attempt::default();
        let expected = Attempt::default();
        let actual_events = Rc::new(RefCell::new(Vec::new()));
        let expected_events = Rc::new(RefCell::new(Vec::new()));
        let outer = actual.adopt(counted("actual", &actual_events), Role::Ancestor);
        let other = expected.adopt(counted("expected", &expected_events), Role::Source);
        actual.gap(Gap::Unbound);
        let invocation = RawInvocation::new(&actual);
        let invalid_limits = super::super::CommittedManifestReadLimits {
            max_manifest_bytes: 0,
            max_records: 0,
        };
        let failure = match store.read_committed_manifest_evidence_checked(
            &invocation,
            &expected,
            "krx",
            "kr",
            0,
            &invalid_limits,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("foreign scope cannot return a value"),
        };
        assert_eq!(failure.phase(), Some(CheckedCommittedEvidencePhase::Scope));
        assert!(failure.body().is_none());
        let report = failure.source_report().unwrap();
        assert_eq!(
            report.first(),
            Some(crate::checked_close::Issue::Gap(Gap::Unbound))
        );
        assert!(
            report
                .secondary()
                .contains(&crate::checked_close::Issue::Gap(Gap::ScopeMismatch))
        );
        assert_eq!(report.attempted(), 0);
        assert_eq!(report.live(), 1);
        assert_eq!(
            expected.report().first(),
            Some(crate::checked_close::Issue::Gap(Gap::ScopeMismatch))
        );
        assert_eq!(expected.report().attempted(), 0);
        assert_eq!(expected.report().live(), 1);
        let calls = std::cell::Cell::new(0);
        let failure = failure.map_body(|body| {
            calls.set(calls.get() + 1);
            MappedError(body)
        });
        assert_eq!(calls.get(), 0);
        assert!(failure.body().is_none());
        assert!(!outer.finish());
        assert!(!other.finish());
        assert_eq!(&*actual_events.borrow(), &["actual"]);
        assert_eq!(&*expected_events.borrow(), &["expected"]);
        assert_eq!(actual.report().attempted(), 1);
        assert_eq!(expected.report().attempted(), 1);
        assert_eq!(failure.source_report().unwrap().attempted(), 0);
        assert_eq!(failure.source_report().unwrap().live(), 1);
    }

    #[test]
    fn actual_limit_admission_keeps_pure_report_absent_and_maps_some_once() {
        let store = super::super::RawStore::new("never-opened-raw-root");
        for (max_manifest_bytes, max_records) in [(0, 1), (1, 0), (usize::MAX, 1)] {
            let attempt = Attempt::default();
            let invocation = RawInvocation::new(&attempt);
            let limits = super::super::CommittedManifestReadLimits {
                max_manifest_bytes,
                max_records,
            };
            let failure = match store.read_committed_manifest_evidence_checked(
                &invocation,
                &attempt,
                "krx",
                "kr",
                0,
                &limits,
            ) {
                Err(failure) => failure,
                Ok(_) => panic!("invalid explicit limits must be refused before effects"),
            };
            assert_eq!(failure.phase(), Some(CheckedCommittedEvidencePhase::Limits));
            assert!(failure.source_report().is_none());
            assert_eq!(attempt.report().attempted(), 0);
            assert_eq!(attempt.report().live(), 0);
            let calls = std::cell::Cell::new(0);
            let mapped = failure.map_body(|body| {
                calls.set(calls.get() + 1);
                MappedError(body)
            });
            assert_eq!(calls.get(), 1);
            assert!(mapped.source_report().is_none());
            match &mapped.body().unwrap().0 {
                StoreError::InvalidCommittedManifestReadLimits { reason } => {
                    assert_eq!(
                        *reason,
                        if max_manifest_bytes == 0 {
                            "max_manifest_bytes must be positive"
                        } else {
                            "max_records must be positive"
                        }
                    );
                }
                StoreError::CommittedManifestReadLimitOverflow => {
                    assert_eq!(max_manifest_bytes, usize::MAX)
                }
                other => panic!("expected original limit error, got {other:?}"),
            }
            assert!(!format!("{mapped:?}").contains("max_manifest_bytes must be positive"));
        }
        let pure_attempt = Attempt::default();
        let pure_invocation = RawInvocation::new(&pure_attempt);
        let invalid_limits = super::super::CommittedManifestReadLimits {
            max_manifest_bytes: 0,
            max_records: 1,
        };
        let pure_failure = match store.read_committed_manifest_evidence_observed_checked(
            &pure_invocation,
            &pure_attempt,
            "krx",
            "kr",
            0,
            &invalid_limits,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("the observed facade must retain the actual pure refusal"),
        };
        assert_eq!(
            pure_failure.parse_state(),
            super::super::CheckedCommittedEvidenceParseState::NotParsed
        );
        assert!(pure_failure.records().is_none());
        assert!(pure_failure.source().body().is_some());
        assert!(pure_failure.source().source_report().is_none());
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let limits = super::super::CommittedManifestReadLimits {
            max_manifest_bytes: 1,
            max_records: 1,
        };
        let failure = match store.read_committed_manifest_evidence_observed_checked(
            &invocation,
            &attempt,
            "krx",
            "kr",
            0,
            &limits,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("positive limits reach only the immutable binding refusal"),
        };
        assert_eq!(
            failure.source().phase(),
            Some(CheckedCommittedEvidencePhase::Binding)
        );
        assert!(failure.records().is_none());
    }

    #[test]
    fn owned_pending_dispatch_moves_value_after_parent_cleanup_with_outer_live() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let events = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(counted("outer", &events), Role::Source);
        let parent = attempt.adopt(counted("parent", &events), Role::Ancestor);
        let leaf = attempt.adopt(counted("leaf", &events), Role::Child);
        let clone = attempt.adopt(counted("clone", &events), Role::Clone);
        assert!(leaf.finish());
        assert!(clone.finish());
        let payload = Payload("owned pending value".to_owned().into_boxed_str());
        let address = payload.0.as_ptr();
        let pending = StrictReaderPending::<(), (), (), Payload>::Manifest(payload);
        let result: Result<Payload, Failure<Infallible>> = complete_strict_reader_pending(
            pending,
            parent,
            |(), _| panic!("wrong pending lane"),
            |(), _| panic!("wrong pending lane"),
            |(), _| panic!("wrong pending lane"),
            |value, parent| {
                assert_eq!(&*events.borrow(), &["leaf", "clone"]);
                assert!(parent.attempt().same_scope(&attempt));
                assert!(parent.finish());
                invocation.settle_local_in(&attempt, Ok(value))
            },
        );
        let value = match result {
            Ok(value) => value,
            Err(_) => panic!("clean counted completion should retain the value"),
        };
        assert_eq!(value.0.as_ptr(), address);
        assert_eq!(&*events.borrow(), &["leaf", "clone", "parent"]);
        assert_eq!(attempt.report().attempted(), 3);
        assert_eq!(attempt.report().live(), 1);
        assert!(outer.finish());
        assert_eq!(&*events.borrow(), &["leaf", "clone", "parent", "outer"]);
        assert_eq!(attempt.report().live(), 0);
    }
}

#[cfg(test)]
mod typed_execution_repair_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn strict_preflight_owns_real_reentry_and_later_unit_without_binding_projection() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let model = RefCell::new(());
        let _borrow = model.borrow_mut();
        let calls = Cell::new(0);
        let pointer = Cell::new(std::ptr::null::<Report>());
        let result = strict_reader_preflight::<StoreError>(
            &invocation,
            &attempt,
            || Ok(()),
            || {
                calls.set(calls.get() + 1);
                match crate::paper_source_evidence::borrow_execution_current(&model, &attempt) {
                    Err(failure) => {
                        pointer.set(std::ptr::from_ref(failure.source_report().unwrap()));
                        Err(crate::close_gate::NativeAdmissionFailure::Execution(
                            failure,
                        ))
                    }
                    Ok(_) => panic!("ordinary cell already borrowed"),
                }
            },
        );
        let failure = CheckedCommittedEvidenceFailure {
            inner: StrictEvidenceFailureKind::Early(result.err().unwrap()),
        };
        assert_eq!(calls.get(), 1);
        assert_eq!(
            failure.execution_failure().unwrap().kind(),
            crate::ExecutionGateFailureKind::Reentrant
        );
        assert_eq!(
            std::ptr::from_ref(failure.source_report().unwrap()),
            pointer.get()
        );
        assert!(failure.execution_local_result().unwrap().is_err());
        assert_eq!(failure.phase(), None);
        assert!(failure.body().is_none());
        assert_eq!(attempt.report().attempted(), 0);
        let mapped = failure.map_body(|_| calls.set(calls.get() + 1));
        assert_eq!(calls.get(), 1);
        assert_eq!(
            std::ptr::from_ref(mapped.source_report().unwrap()),
            pointer.get()
        );
        assert!(mapped.execution_failure().is_some());
    }
}
