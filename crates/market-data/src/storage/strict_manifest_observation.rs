//! Retains only parser-returned manifest rows across the complete checked read.
//! No read-internal prefix, rejected row, or interrupted-allocation recovery is claimed.

use super::{
    CheckedCommittedEvidenceFailure, CommittedManifestEvidenceRecord, ManifestPrefixFailure,
};

/// Parser-return state only; neither custody nor origin authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedCommittedEvidenceParseState {
    NotParsed,
    RejectedPrefix,
    Parsed,
}

/// Genuine returned parser records after the complete strict native driver.
pub struct CheckedCommittedEvidenceObservation {
    records: Vec<CommittedManifestEvidenceRecord>,
}
impl CheckedCommittedEvidenceObservation {
    pub fn records(&self) -> &[CommittedManifestEvidenceRecord] {
        &self.records
    }
    pub fn into_records(self) -> Vec<CommittedManifestEvidenceRecord> {
        self.records
    }
}
impl std::fmt::Debug for CheckedCommittedEvidenceObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedEvidenceObservation")
            .field("parse_state", &CheckedCommittedEvidenceParseState::Parsed)
            .field("record_count", &self.records.len())
            .finish()
    }
}

/// Whole original strict failure with only actually returned parser records.
pub struct CheckedCommittedEvidenceObservationFailure<E = super::StoreError> {
    source: CheckedCommittedEvidenceFailure<E>,
    records: Option<Vec<CommittedManifestEvidenceRecord>>,
    state: CheckedCommittedEvidenceParseState,
}
impl<E> CheckedCommittedEvidenceObservationFailure<E> {
    pub fn source(&self) -> &CheckedCommittedEvidenceFailure<E> {
        &self.source
    }
    pub fn records(&self) -> Option<&[CommittedManifestEvidenceRecord]> {
        self.records.as_deref()
    }
    pub fn parse_state(&self) -> CheckedCommittedEvidenceParseState {
        self.state
    }
}
impl<E> std::fmt::Debug for CheckedCommittedEvidenceObservationFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCommittedEvidenceObservationFailure")
            .field("parse_state", &self.state)
            .field(
                "record_count",
                &self.records.as_ref().map(|rows| rows.len()),
            )
            .field("first_body_present", &self.source.body().is_some())
            .finish()
    }
}

// Private ordinary-data bag. It is owned OUTSIDE the entire native unit T.
pub(super) struct ReturnedParse<R> {
    records: Option<Vec<R>>,
    state: CheckedCommittedEvidenceParseState,
}
impl<R> ReturnedParse<R> {
    fn new() -> Self {
        Self {
            records: None,
            state: CheckedCommittedEvidenceParseState::NotParsed,
        }
    }

    // This is the only new parse-result boundary. The native FnOnce strategy
    // invokes it at the old parse point, before the unchanged context.local.
    pub(super) fn parse_once<E>(
        &mut self,
        parse: impl FnOnce() -> Result<Vec<R>, ManifestPrefixFailure<R, E>>,
    ) -> Result<(), E> {
        match parse() {
            Ok(records) => {
                self.records = Some(records);
                self.state = CheckedCommittedEvidenceParseState::Parsed;
                Ok(())
            }
            Err(ManifestPrefixFailure { records, source }) => {
                self.records = Some(records);
                self.state = CheckedCommittedEvidenceParseState::RejectedPrefix;
                Err(source)
            }
        }
    }
}
struct ReturnedFailure<R, F> {
    source: F,
    observation: ReturnedParse<R>,
}

// Shared by the actual native wrapper and ordinary/Counted Source cases.
// F moves whole, with its original snapshot and chronology untouched.
fn run_observation<R, F>(
    operation: impl FnOnce(&mut ReturnedParse<R>) -> Result<(), F>,
) -> Result<Vec<R>, ReturnedFailure<R, F>> {
    let mut observation = ReturnedParse::new();
    let result = operation(&mut observation);
    match result {
        Err(source) => Err(ReturnedFailure {
            source,
            observation,
        }),
        Ok(()) => match (observation.state, observation.records) {
            (CheckedCommittedEvidenceParseState::Parsed, Some(records)) => Ok(records),
            // Native success is reachable only after the FnOnce parse
            // strategy returned Ok and every original completion succeeded.
            // Earlier/parse/cleanup refusals all return the real F instead.
            _ => unreachable!("private observed unit success requires returned parsed records"),
        },
    }
}

#[expect(
    clippy::result_large_err,
    reason = "Preserve the whole strict source with its returned parser rows across every later cleanup stage."
)]
pub(super) fn observe_native(
    operation: impl FnOnce(
        &mut ReturnedParse<CommittedManifestEvidenceRecord>,
    ) -> Result<(), CheckedCommittedEvidenceFailure>,
) -> Result<CheckedCommittedEvidenceObservation, CheckedCommittedEvidenceObservationFailure> {
    match run_observation(operation) {
        Ok(records) => Ok(CheckedCommittedEvidenceObservation { records }),
        Err(ReturnedFailure {
            source,
            observation,
        }) => Err(CheckedCommittedEvidenceObservationFailure {
            source,
            records: observation.records,
            state: observation.state,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checked_close::{Attempt, Close, Errno, Failure, Gap, Role};
    use crate::raw_custody::RawInvocation;
    use std::{cell::Cell, convert::Infallible, rc::Rc};

    struct Row {
        value: Box<str>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Row {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    struct Body(Box<str>);
    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Err(Errno::Io)
        }
    }
    struct Whole {
        first: Failure<Body>,
        later: Failure<Infallible>,
    }
    fn row(drops: &Rc<Cell<usize>>) -> Row {
        Row {
            value: "ordinary returned row".to_owned().into_boxed_str(),
            drops: Rc::clone(drops),
        }
    }

    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "This production-helper test keeps the first canonical failure and later cleanup failure together to verify chronology."
    )]
    fn rejected_prefix_keeps_non_debug_rows_and_first_some_across_cleanup() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let closes = Rc::new(Cell::new(0));
        let owner = attempt.adopt(Counted(Rc::clone(&closes)), Role::Source);
        let drops = Rc::new(Cell::new(0));
        let returned = row(&drops);
        let row_address = returned.value.as_ptr();
        let body = Body("first parser body".to_owned().into_boxed_str());
        let body_address = body.0.as_ptr();
        let result = run_observation::<Row, Whole>(|bag| {
            let parsed = bag.parse_once(|| {
                super::super::retain_manifest_parse_prefix(Vec::new(), |rows| {
                    rows.push(returned);
                    Err(body)
                })
            });
            let first = match invocation.settle_local_in(&attempt, parsed) {
                Err(failure) => failure,
                Ok(()) => panic!("the original parser body must fail"),
            };
            assert_eq!(first.report().attempted(), 0);
            assert_eq!(first.report().live(), 1);
            assert!(!owner.finish());
            let later = match invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())) {
                Err(failure) => failure,
                Ok(()) => panic!("later cleanup cannot clear the source failure"),
            };
            Err(Whole { first, later })
        });
        let failure = match result {
            Err(failure) => failure,
            Ok(_) => panic!("whole source must survive"),
        };
        assert_eq!(
            failure.observation.state,
            CheckedCommittedEvidenceParseState::RejectedPrefix
        );
        assert_eq!(failure.observation.records.as_ref().unwrap().len(), 1);
        assert_eq!(
            failure.observation.records.as_ref().unwrap()[0]
                .value
                .as_ptr(),
            row_address
        );
        assert_eq!(
            failure.source.first.body().unwrap().0.as_ptr(),
            body_address
        );
        assert_eq!(failure.source.first.report().attempted(), 0);
        assert_eq!(failure.source.first.report().live(), 1);
        assert_eq!(failure.source.later.report().attempted(), 1);
        assert_eq!(failure.source.later.report().unknown(), 1);
        assert_eq!(failure.source.later.report().live(), 0);
        assert!(failure.source.later.body().is_none());
        assert_eq!(closes.get(), 1);
        assert_eq!(drops.get(), 0);
        assert!(!format!("{:?}", failure.source.first).contains("first parser body"));
        drop(failure);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    #[expect(
        clippy::result_large_err,
        reason = "This production-helper test retains the first bodyless failure and actual later cleanup failure alongside returned rows."
    )]
    fn parsed_rows_stay_outside_value_when_local_settlement_fails_bodyless() {
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        let closes = Rc::new(Cell::new(0));
        let owner = attempt.adopt(Counted(Rc::clone(&closes)), Role::Source);
        let drops = Rc::new(Cell::new(0));
        let returned = row(&drops);
        let address = returned.value.as_ptr();
        let result = run_observation::<Row, Whole>(|bag| {
            let parsed = bag.parse_once(|| {
                super::super::retain_manifest_parse_prefix(Vec::new(), |rows| {
                    rows.push(returned);
                    Ok::<(), Body>(())
                })
            });
            attempt.gap(Gap::TerminalPublication);
            let first = match invocation.settle_local_in(&attempt, parsed) {
                Err(failure) => failure,
                Ok(()) => panic!("late dirt must suppress the native unit value"),
            };
            assert!(!owner.finish());
            let later = match invocation.settle_local_in(&attempt, Ok::<(), Infallible>(())) {
                Err(failure) => failure,
                Ok(()) => panic!("cleanup remains failed"),
            };
            Err(Whole { first, later })
        });
        let failure = match result {
            Err(failure) => failure,
            Ok(_) => panic!("late failure must survive"),
        };
        assert_eq!(
            failure.observation.state,
            CheckedCommittedEvidenceParseState::Parsed
        );
        assert_eq!(
            failure.observation.records.as_ref().unwrap()[0]
                .value
                .as_ptr(),
            address
        );
        assert!(failure.source.first.body().is_none());
        assert_eq!(failure.source.first.report().attempted(), 0);
        assert_eq!(failure.source.first.report().live(), 1);
        assert_eq!(failure.source.later.report().attempted(), 1);
        assert_eq!(failure.source.later.report().unknown(), 1);
        assert_eq!(closes.get(), 1);
        assert_eq!(drops.get(), 0);
        drop(failure);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn not_parsed_is_distinct_from_a_genuine_empty_rejected_prefix() {
        let before_parse = Attempt::default();
        let not_parsed = match run_observation::<Row, _>(|_| {
            Err(before_parse.failure(Some(Body("before parse".to_owned().into_boxed_str()))))
        }) {
            Err(failure) => failure,
            Ok(_) => panic!("the actual early error must survive"),
        };
        assert_eq!(
            not_parsed.observation.state,
            CheckedCommittedEvidenceParseState::NotParsed
        );
        assert!(not_parsed.observation.records.is_none());
        assert!(not_parsed.source.body().is_some());

        let rejected_attempt = Attempt::default();
        let rejected = match run_observation::<Row, _>(|bag| {
            match bag.parse_once(|| {
                super::super::retain_manifest_parse_prefix(Vec::new(), |_| {
                    Err(Body("empty prefix source".to_owned().into_boxed_str()))
                })
            }) {
                Err(body) => Err(rejected_attempt.failure(Some(body))),
                Ok(()) => panic!("the genuine empty-prefix parser result must fail"),
            }
        }) {
            Err(failure) => failure,
            Ok(_) => panic!("the parser body must survive"),
        };
        assert_eq!(
            rejected.observation.state,
            CheckedCommittedEvidenceParseState::RejectedPrefix
        );
        assert!(rejected.observation.records.as_ref().unwrap().is_empty());
        assert!(rejected.source.body().is_some());
    }

    #[test]
    fn successful_observation_returns_same_non_clone_rows_once() {
        let drops = Rc::new(Cell::new(0));
        let returned = row(&drops);
        let address = returned.value.as_ptr();
        let parses = Cell::new(0);
        let result = run_observation::<Row, Body>(|bag| {
            bag.parse_once(|| {
                parses.set(parses.get() + 1);
                super::super::retain_manifest_parse_prefix(Vec::new(), |rows| {
                    rows.push(returned);
                    Ok::<(), Body>(())
                })
            })
        });
        let rows = match result {
            Ok(rows) => rows,
            Err(_) => panic!("clean production dispatch returns rows"),
        };
        assert_eq!(parses.get(), 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value.as_ptr(), address);
        assert_eq!(drops.get(), 0);
        drop(rows);
        assert_eq!(drops.get(), 1);
    }
}
