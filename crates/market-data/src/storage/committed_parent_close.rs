//! Private parent-directory custody for the canonical checked manifest reader.
//! Scope and stored path policy do not establish root-origin authority.

use super::{
    File, Path, PathBuf, RawStore, StoreError, io_err, validate_committed_manifest_evidence_node,
    validate_scope,
};
use crate::checked_close::{Attempt, CheckedFile, Close, Failure, Gap, Owner, Report, Role};
use crate::raw_custody::RawInvocation;
use std::ffi::OsString;
use std::path::Component;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedDirectoryPhase {
    Scope,
    Binding,
    Execution,
    ScopeValidation,
    Path,
    OpenRoot,
    OpenChild,
    ValidateChild,
    ParentClose,
    Body,
    Close,
}

/// Actual canonical local failure, never directory-origin authority.
pub struct CheckedCommittedDirectoryFailure<E = StoreError> {
    failure: Failure<E>,
    phase: CheckedCommittedDirectoryPhase,
    component_index: Option<usize>,
    execution: Option<crate::ExecutionGateFailure>,
    execution_primary: bool,
}

impl<E> CheckedCommittedDirectoryFailure<E> {
    fn new(
        failure: Failure<E>,
        phase: CheckedCommittedDirectoryPhase,
        component_index: Option<usize>,
    ) -> Self {
        Self {
            failure,
            phase,
            component_index,
            execution: None,
            execution_primary: false,
        }
    }

    fn with_execution(mut self, failure: crate::ExecutionGateFailure, primary: bool) -> Self {
        self.execution = Some(failure);
        self.execution_primary = primary;
        self
    }
    pub fn execution_failure(&self) -> Option<&crate::ExecutionGateFailure> {
        self.execution.as_ref()
    }
    /// The primary source report; report() remains the actual local settlement.
    pub fn source_report(&self) -> Option<&Report> {
        if self.execution_primary {
            self.execution
                .as_ref()
                .and_then(crate::ExecutionGateFailure::source_report)
        } else {
            Some(self.failure.report())
        }
    }
    pub fn body(&self) -> Option<&E> {
        self.failure.body()
    }
    pub fn report(&self) -> &Report {
        self.failure.report()
    }
    pub fn phase(&self) -> CheckedCommittedDirectoryPhase {
        self.phase
    }
    pub fn component_index(&self) -> Option<usize> {
        self.component_index
    }

    pub fn map_body<U>(self, map: impl FnOnce(E) -> U) -> CheckedCommittedDirectoryFailure<U> {
        CheckedCommittedDirectoryFailure {
            failure: self.failure.map_body(map),
            phase: self.phase,
            component_index: self.component_index,
            execution: self.execution,
            execution_primary: self.execution_primary,
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        Option<E>,
        Report,
        CheckedCommittedDirectoryPhase,
        Option<usize>,
    ) {
        let (body, report) = self.failure.into_parts();
        (body, report, self.phase, self.component_index)
    }
}

impl<E> std::fmt::Debug for CheckedCommittedDirectoryFailure<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CheckedCommittedDirectoryFailure")
            .field("body_present", &self.failure.body().is_some())
            .field("report", self.failure.report())
            .field("phase", &self.phase)
            .field("component_index", &self.component_index)
            .finish()
    }
}

fn committed_parent_scope<E>(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
) -> Result<(), CheckedCommittedDirectoryFailure<E>> {
    invocation
        .settle_local_in(expected, Ok::<(), E>(()))
        .map_err(|failure| {
            CheckedCommittedDirectoryFailure::new(
                failure,
                CheckedCommittedDirectoryPhase::Scope,
                None,
            )
        })
}

#[expect(
    clippy::result_large_err,
    reason = "The original scope or binding failure retains its actual local report and phase."
)]
fn committed_parent_preflight(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
) -> Result<(), CheckedCommittedDirectoryFailure> {
    let original = committed_parent_scope::<StoreError>(invocation, expected);
    let execution = invocation.compare_native_context_scopes(expected);
    crate::raw_custody::join_native_checks(
        original,
        execution,
        |original, execution| match original {
            Some(first) => first.with_execution(execution, false),
            None => CheckedCommittedDirectoryFailure::new(
                invocation
                    .settle_local_in(expected, Ok::<(), StoreError>(()))
                    .expect_err("typed execution refusal is permanent"),
                CheckedCommittedDirectoryPhase::Execution,
                None,
            )
            .with_execution(execution, true),
        },
    )?;
    match crate::close_gate::require_native(invocation, expected) {
        Ok(()) => {}
        Err(crate::close_gate::NativeAdmissionFailure::Unbound) => {
            return Err(CheckedCommittedDirectoryFailure::new(
                invocation.refused(Gap::Unbound),
                CheckedCommittedDirectoryPhase::Binding,
                None,
            ));
        }
        Err(crate::close_gate::NativeAdmissionFailure::Execution(execution)) => {
            return Err(CheckedCommittedDirectoryFailure::new(
                invocation
                    .settle_local_in(expected, Ok::<(), StoreError>(()))
                    .expect_err("typed execution refusal is permanent"),
                CheckedCommittedDirectoryPhase::Execution,
                None,
            )
            .with_execution(execution, true));
        }
    }
    Ok(())
}

// None is used only after a genuine scope/close failure has latched the
// actual invocation. All callers settle owned graph slots BEFORE this.
fn committed_parent_failure<E>(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
    body: Option<E>,
    phase: CheckedCommittedDirectoryPhase,
    component_index: Option<usize>,
) -> CheckedCommittedDirectoryFailure<E> {
    let settled = match body {
        Some(body) => invocation.settle_local_in(expected, Err::<(), E>(body)),
        None => invocation.settle_local_in(expected, Ok::<(), E>(())),
    };
    CheckedCommittedDirectoryFailure::new(
        settled.expect_err("a real body, scope or close failure is already latched"),
        phase,
        component_index,
    )
}

/// The production graph and counted-Close Source cases share these exact
/// acquire/validate/rotate/drain operations. Only actual Owner slots decide
/// which resources settle; no path name or supplied approval bit does.
struct CommittedParentGraph<'invocation, 'attempt, 'expected, R: Close> {
    current: Option<Owner<R>>,
    child: Option<Owner<R>>,
    current_index: Option<usize>,
    invocation: &'invocation RawInvocation<'attempt>,
    expected: &'expected Attempt,
}

impl<'invocation, 'attempt, 'expected, R: Close>
    CommittedParentGraph<'invocation, 'attempt, 'expected, R>
{
    fn new(invocation: &'invocation RawInvocation<'attempt>, expected: &'expected Attempt) -> Self {
        Self {
            current: None,
            child: None,
            current_index: None,
            invocation,
            expected,
        }
    }

    fn drain(&mut self) {
        if let Some(child) = self.child.take() {
            child.finish();
        }
        if let Some(parent) = self.current.take() {
            parent.finish();
        }
    }

    fn fail<T, E>(
        &mut self,
        body: Option<E>,
        phase: CheckedCommittedDirectoryPhase,
        component_index: Option<usize>,
    ) -> Result<T, CheckedCommittedDirectoryFailure<E>> {
        self.drain();
        Err(committed_parent_failure(
            self.invocation,
            self.expected,
            body,
            phase,
            component_index,
        ))
    }

    fn acquire_root<E>(
        &mut self,
        acquire: impl FnOnce() -> Result<R, E>,
    ) -> Result<(), CheckedCommittedDirectoryFailure<E>> {
        assert!(self.current.is_none() && self.child.is_none());
        if committed_parent_scope::<E>(self.invocation, self.expected).is_err() {
            return self.fail(None, CheckedCommittedDirectoryPhase::Scope, None);
        }
        let resource = match acquire() {
            Ok(resource) => resource,
            Err(body) => {
                return self.fail(Some(body), CheckedCommittedDirectoryPhase::OpenRoot, None);
            }
        };
        // Immediate canonical adoption of the returned actual File, with
        // no intervening fallible I/O. Abort/allocation/adoption-internal
        // failure behavior is not established by this Source proposal.
        self.current = Some(self.invocation.attempt().adopt(resource, Role::Ancestor));
        if !self.invocation.attempt().check() {
            return self.fail(None, CheckedCommittedDirectoryPhase::Scope, None);
        }
        Ok(())
    }

    fn step<E>(
        &mut self,
        index: usize,
        acquire: impl FnOnce(&R) -> Result<R, E>,
        validate: impl FnOnce(&R, &R) -> Result<(), E>,
    ) -> Result<(), CheckedCommittedDirectoryFailure<E>> {
        assert!(self.child.is_none());
        if committed_parent_scope::<E>(self.invocation, self.expected).is_err() {
            return self.fail(None, CheckedCommittedDirectoryPhase::Scope, Some(index));
        }
        let resource = match acquire(self.current.as_ref().expect("held parent").get()) {
            Ok(resource) => resource,
            Err(body) => {
                return self.fail(
                    Some(body),
                    CheckedCommittedDirectoryPhase::OpenChild,
                    Some(index),
                );
            }
        };
        self.child = Some(self.invocation.attempt().adopt(resource, Role::Ancestor));
        if !self.invocation.attempt().check() {
            return self.fail(None, CheckedCommittedDirectoryPhase::Scope, Some(index));
        }
        let body = validate(
            self.current.as_ref().expect("held parent").get(),
            self.child.as_ref().expect("held child").get(),
        );
        if let Err(body) = body {
            return self.fail(
                Some(body),
                CheckedCommittedDirectoryPhase::ValidateChild,
                Some(index),
            );
        }
        let clean_before_parent_close = self.invocation.attempt().check();
        let parent_clean = self.current.take().expect("validated predecessor").finish();
        if !clean_before_parent_close || !parent_clean {
            return self.fail(
                None,
                if !clean_before_parent_close {
                    CheckedCommittedDirectoryPhase::Scope
                } else {
                    CheckedCommittedDirectoryPhase::ParentClose
                },
                Some(index),
            );
        }
        // Move the canonical child Owner only AFTER predecessor completion.
        self.current = self.child.take();
        self.current_index = Some(index);
        Ok(())
    }

    fn retain<E>(
        mut self,
    ) -> Result<
        CommittedDirectoryHolder<'invocation, 'attempt, 'expected, R>,
        CheckedCommittedDirectoryFailure<E>,
    > {
        if committed_parent_scope::<E>(self.invocation, self.expected).is_err() {
            let index = self.current_index;
            return self.fail(None, CheckedCommittedDirectoryPhase::Scope, index);
        }
        assert!(self.child.is_none());
        Ok(CommittedDirectoryHolder {
            owner: Some(self.current.take().expect("final actual parent Owner")),
            component_index: self.current_index,
            invocation: self.invocation,
            expected: self.expected,
        })
    }
}

impl<R: Close> Drop for CommittedParentGraph<'_, '_, '_, R> {
    fn drop(&mut self) {
        if self.child.is_some() || self.current.is_some() {
            self.invocation.attempt().gap(Gap::Abandoned);
            self.drain();
        }
    }
}

struct CommittedDirectoryHolder<'invocation, 'attempt, 'expected, R: Close> {
    owner: Option<Owner<R>>,
    component_index: Option<usize>,
    invocation: &'invocation RawInvocation<'attempt>,
    expected: &'expected Attempt,
}

impl<R: Close> CommittedDirectoryHolder<'_, '_, '_, R> {
    fn held(&self) -> &Owner<R> {
        self.owner
            .as_ref()
            .expect("retained until consuming finish")
    }

    fn finish<T, E>(
        mut self,
        body: Result<T, E>,
    ) -> Result<T, CheckedCommittedDirectoryFailure<E>> {
        let body_failed = body.is_err();
        let scope_failed = committed_parent_scope::<E>(self.invocation, self.expected).is_err();
        self.owner.take().expect("one retained owner").finish();
        self.invocation
            .settle_local_in(self.expected, body)
            .map_err(|failure| {
                CheckedCommittedDirectoryFailure::new(
                    failure,
                    if body_failed {
                        CheckedCommittedDirectoryPhase::Body
                    } else if scope_failed {
                        CheckedCommittedDirectoryPhase::Scope
                    } else {
                        CheckedCommittedDirectoryPhase::Close
                    },
                    self.component_index,
                )
            })
    }
}

impl<R: Close> Drop for CommittedDirectoryHolder<'_, '_, '_, R> {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.attempt().gap(Gap::Abandoned);
            owner.finish();
        }
    }
}

// Ordinary actual opening inputs; never origin, UID or path authority.
pub(super) struct CommittedDirectoryInputContext {
    pub(super) raw_root: PathBuf,
    pub(super) provider: String,
    pub(super) market: String,
    pub(super) trusted_raw_owner_uid: u32,
}

/// Owns the actual final File and borrows the real local invocation/scope.
/// Its ordinary path and trusted UID policy do not prove root origin.
pub(super) struct CheckedCommittedDirectory<'invocation, 'attempt, 'expected> {
    holder: CommittedDirectoryHolder<'invocation, 'attempt, 'expected, File>,
    path: PathBuf,
    input_context: CommittedDirectoryInputContext,
}

impl<'invocation, 'attempt, 'expected> CheckedCommittedDirectory<'invocation, 'attempt, 'expected> {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn held_file(&self) -> &CheckedFile {
        self.holder.held()
    }
    pub(super) fn invocation(&self) -> &'invocation RawInvocation<'attempt> {
        self.holder.invocation
    }
    pub(super) fn expected_attempt(&self) -> &'expected Attempt {
        self.holder.expected
    }
    pub(super) fn input_context(&self) -> &CommittedDirectoryInputContext {
        &self.input_context
    }
    /// T is ordinary caller business data, not proof of arbitrary caller resources.
    pub(super) fn finish<T, E>(
        self,
        body: Result<T, E>,
    ) -> Result<T, CheckedCommittedDirectoryFailure<E>> {
        self.holder.finish(body)
    }
}

fn committed_directory_open_error(
    path: &Path,
    context: &str,
    error: rustix::io::Errno,
) -> StoreError {
    use rustix::io::Errno;
    if error == Errno::LOOP || error == Errno::NOTDIR {
        StoreError::UnsafePath {
            path: path.display().to_string(),
            reason: "committed Raw directory components must not be symlinks or non-directories"
                .to_owned(),
        }
    } else {
        io_err(context, error.into())
    }
}

fn committed_directory_components(
    root: &Path,
    provider: &str,
    market: &str,
) -> Result<(Vec<OsString>, usize), StoreError> {
    if !root.is_absolute() {
        return Err(StoreError::UnsafePath {
            path: root.display().to_string(),
            reason: "committed manifest evidence requires an absolute RawStore root".to_owned(),
        });
    }
    let mut components = Vec::<OsString>::new();
    for component in root.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::ParentDir | Component::Prefix(_) => {
                return Err(StoreError::UnsafePath {
                    path: root.display().to_string(),
                    reason: "committed manifest evidence root must not contain parent components"
                        .to_owned(),
                });
            }
        }
    }
    let raw_root_component = components.len();
    components.push(OsString::from("raw"));
    components.push(OsString::from("manifests"));
    components.push(OsString::from(format!("provider={provider}")));
    components.push(OsString::from(format!("market={market}")));
    Ok((components, raw_root_component))
}

impl RawStore {
    /// Private checked parent walk. The immutable gate refuses before
    /// path and native effects.
    #[expect(
        clippy::result_large_err,
        reason = "Parent acquisition returns its whole original scope, open and close diagnostics without a replacement error allocation."
    )]
    pub(super) fn open_committed_manifest_directory_checked<'invocation, 'attempt, 'expected>(
        &self,
        invocation: &'invocation RawInvocation<'attempt>,
        expected: &'expected Attempt,
        provider: &str,
        market: &str,
        trusted_raw_owner_uid: u32,
    ) -> Result<
        CheckedCommittedDirectory<'invocation, 'attempt, 'expected>,
        CheckedCommittedDirectoryFailure,
    > {
        use rustix::fs::{FileType, Mode, OFlags, open, openat};
        committed_parent_preflight(invocation, expected)?;
        if let Err(body) = validate_scope(provider, market) {
            return Err(committed_parent_failure(
                invocation,
                expected,
                Some(body),
                CheckedCommittedDirectoryPhase::ScopeValidation,
                None,
            ));
        }
        let (components, raw_root_component) =
            match committed_directory_components(&self.root, provider, market) {
                Ok(value) => value,
                Err(body) => {
                    return Err(committed_parent_failure(
                        invocation,
                        expected,
                        Some(body),
                        CheckedCommittedDirectoryPhase::Path,
                        None,
                    ));
                }
            };
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut graph = CommittedParentGraph::<File>::new(invocation, expected);
        graph.acquire_root(|| {
            Ok::<File, StoreError>(File::from(
                open("/", directory_flags, Mode::empty()).map_err(|error| {
                    committed_directory_open_error(Path::new("/"), "open filesystem root", error)
                })?,
            ))
        })?;
        let mut path = PathBuf::from("/");
        for (index, component) in components.iter().enumerate() {
            path.push(component);
            graph.step(
                index,
                |directory| {
                    let child = openat(
                        directory,
                        component.as_os_str(),
                        directory_flags,
                        Mode::empty(),
                    )
                    .map_err(|error| {
                        committed_directory_open_error(
                            &path,
                            &format!("open committed Raw directory {}", path.display()),
                            error,
                        )
                    })?;
                    let child = File::from(child);
                    Ok::<File, StoreError>(child)
                },
                |directory, child| {
                    let trusted_owner =
                        (index >= raw_root_component).then_some(trusted_raw_owner_uid);
                    validate_committed_manifest_evidence_node(
                        &path,
                        directory,
                        component.as_os_str(),
                        child,
                        FileType::Directory,
                        trusted_owner,
                    )
                },
            )?;
        }
        let holder = graph.retain::<StoreError>()?;
        Ok(CheckedCommittedDirectory {
            holder,
            path,
            input_context: CommittedDirectoryInputContext {
                raw_root: self.root.clone(),
                provider: provider.to_owned(),
                market: market.to_owned(),
                trusted_raw_owner_uid,
            },
        })
    }
}

// COMMITTED-REVALIDATE-S01-BEGIN
// Actual holder identities, never a Report/path/UID approval token.
fn poison_transient_holder_scopes<R: Close>(current: &CommittedDirectoryHolder<'_, '_, '_, R>) {
    let actual = current.invocation.attempt();
    let expected = current.expected;
    let owned = current.held().attempt();
    if !actual.same_scope(expected) || !actual.same_scope(owned) {
        actual.gap(Gap::ScopeMismatch);
        expected.gap(Gap::ScopeMismatch);
        owned.gap(Gap::ScopeMismatch);
    }
}

// One production completion path, also used by private counted-Close Source cases.
fn compare_then_finish_transient<R: Close, E>(
    current: CommittedDirectoryHolder<'_, '_, '_, R>,
    compare: impl FnOnce(&R) -> Result<(), E>,
) -> Result<(), CheckedCommittedDirectoryFailure<E>> {
    poison_transient_holder_scopes(&current);
    let comparison = if current.invocation.attempt().check() {
        compare(current.held().get())
    } else {
        // Consume the real current holder even when the main ledger is dirty.
        // Its existing local settlement suppresses success without inventing E.
        Ok::<(), E>(())
    };
    current.finish(comparison)
}

// Ordinary stored paths only. The callback is the original File comparison.
fn compare_committed_directory_paths(
    path: &Path,
    current_path: &Path,
    compare_files: impl FnOnce() -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    if current_path != path {
        return Err(StoreError::UnsafePath {
            path: path.display().to_string(),
            reason: "committed Raw control directory path changed during the read".to_owned(),
        });
    }
    compare_files()
}

impl CheckedCommittedDirectory<'_, '_, '_> {
    // Rewalk with the same retained parent and invocation during the checked read.
    #[expect(
        clippy::result_large_err,
        reason = "An ancestry failure keeps the original transient-parent cleanup report and component phase."
    )]
    pub(super) fn revalidate_ancestry_checked(
        &self,
    ) -> Result<(), CheckedCommittedDirectoryFailure> {
        poison_transient_holder_scopes(&self.holder);
        committed_parent_preflight(self.invocation(), self.expected_attempt())?;
        let context = self.input_context();
        let store = RawStore::new(context.raw_root.clone());
        let current = store.open_committed_manifest_directory_checked(
            self.invocation(),
            self.expected_attempt(),
            &context.provider,
            &context.market,
            context.trusted_raw_owner_uid,
        )?;
        // Extract real ownership before creating borrows for comparison callbacks.
        let CheckedCommittedDirectory {
            holder: current_holder,
            path: current_path,
            input_context: _current_context,
        } = current;
        compare_then_finish_transient(current_holder, |current_file| {
            compare_committed_directory_paths(self.path(), current_path.as_path(), || {
                super::revalidate_committed_manifest_evidence_directory(
                    self.path(),
                    self.held_file().get(),
                    current_file,
                )
            })
        })
    }
}
