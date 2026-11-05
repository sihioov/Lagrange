//! Immutable, unavailable binding gate for the upcoming checked reader.
//! No environment lookup or binding setter exists here.

use crate::{checked_close::Attempt, raw_custody::RawInvocation};

pub(crate) const SCOPE: Option<&str> = None;
pub(crate) const TARGET: Option<&str> = None;
pub(crate) const IMAGE: Option<&str> = None;
pub(crate) const REVISION: Option<&str> = None;
#[derive(Debug)]
pub(crate) struct Unbound;
pub(crate) fn require_bound() -> Result<(), Unbound> {
    SCOPE.ok_or(Unbound)?;
    TARGET.ok_or(Unbound)?;
    IMAGE.ok_or(Unbound)?;
    REVISION.ok_or(Unbound)?;
    Ok(())
}

#[derive(Debug)]
pub(crate) enum NativeAdmissionFailure {
    Unbound,
    Execution(crate::ExecutionGateFailure),
}

/// Keeps the four immutable defaults closed while admitting only an invocation
/// carrying the private context installed by a genuine checked Current.
pub(crate) fn require_native(
    invocation: &RawInvocation<'_>,
    expected: &Attempt,
) -> Result<(), NativeAdmissionFailure> {
    match invocation.native_context() {
        Some(context) => context
            .admit(invocation, expected)
            .map_err(NativeAdmissionFailure::Execution),
        None => require_bound()
            .map_err(|_| NativeAdmissionFailure::Unbound)
            .and(Err(NativeAdmissionFailure::Unbound)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IMAGE, NativeAdmissionFailure, REVISION, SCOPE, TARGET, require_bound, require_native,
    };
    use crate::{checked_close::Attempt, raw_custody::RawInvocation};

    #[test]
    fn all_bindings_are_absent_and_gate_refuses() {
        assert_eq!([SCOPE, TARGET, IMAGE, REVISION], [None; 4]);
        assert!(require_bound().is_err());
    }

    #[test]
    fn default_unbound_native_admission_has_zero_effects() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let mut effects = 0;

        let result = require_native(&invocation, &attempt);
        if result.is_ok() {
            effects += 1;
        }

        assert!(matches!(result, Err(NativeAdmissionFailure::Unbound)));
        assert_eq!(effects, 0);
        assert!(attempt.check());
    }
}
