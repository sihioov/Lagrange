# R2 I4c daily success boundary and web evidence

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Status: direct coordinator source review; fresh compiler/tests pending.
The [previous candidate failure](2026-10-16-r2-daily-settlement-ci.md) is preserved.

## Daily success publication

[publish.rs](../../crates/job-queue/src/owner_beta/publish.rs) adds a separate
daily success API requiring both an opaque reopened admitted-input capability
and an opaque daily computation. Serialized factor/target values cannot enter
that API. The exact claim and daily pins are checked, and rebuilding the target
from the bound factor must reproduce the full target, including strategy config.

The transaction locks the queue claim and exact pending run before invoking the
existing SQL publication gate. A canceled claim uses the existing fenced daily
failure cleanup; an expired/lost claim cannot publish. File capability checks
repeat after the gate, before the run update and immediately before commit.
Eleven items, factor/target hashes, cash weight, the run, job and attempt commit
atomically. Excluded daily items use numeric zero and no rank, as required by
0060; historical publication retains its existing representation. The existing
SQL trigger owns publication timestamps and rechecks current authority using
clock_timestamp. Commit uncertainty remains non-retryable and distinct.

Only SQLSTATE 23514 from the explicit gate is mapped to the value-free daily
authority-denied error. It does not identify a particular revocation or infer
supersession. Constraint failures from writes retain the generic database
classification. No migration, privilege, queue or historical runner change occurs.

Eight written unit cases check exact computation, all four rehashed foreign
pins, stale hash/date, nonfinite values, different valid strategy targets,
changed discriminants, zero-weight excluded projection and redacted denial.
They use synthetic computation fixtures only. They cannot manufacture an
approved input or prove successful end-to-end DB/file publication.

## Hosted web failure visibility

[candidate-web-e2e-runner.py](../../scripts/qa/candidate-web-e2e-runner.py)
emits one allowlisted JSON record after each completed lane, including failure
and cleanup. It contains finite state/error codes, counts, cleanup status and
case file/line/status/attempt outcomes. Logical identities are hashed; titles,
tokens, annotations, process output, arguments, environment and bodies are not
emitted. Unknown strings are replaced with fixed markers. Diagnostic row caps
are explicit and do not change test execution or acceptance.

Two [supervisor tests](../../scripts/qa/test_candidate_web_e2e_runner.py) check
pass/failure location retention, JSON emission and rejection of a private sentinel
placed throughout free-text/token fields. All 32 earlier supervisor tests remain.
The lane selection, retries, timeouts, validation, private receipt and cleanup
code are unchanged except the new emission call. The earlier web failure's cause
is still UNKNOWN; diagnostic visibility is not a claim to have repaired it.

## Verification boundary

Source/diff review and explicit-file edition-2024 rustfmt/whitespace are the only
local checks. No compiler, test, Python project import/parser, SQL or helper ran
locally. Fresh hosted verification must recover eight new Rust names, all 34 web
supervisor names, both web lane receipts and preserved Rust/PG gates. No independent
review or executable acceptance is claimed here.

Daily success with an actual approved base and complete multi-day input,
superseded-session cancellation, full producer/admitter/worker orchestration,
integrated independent review, I5-I7, UID/ACL/crash/installer evidence, operational
activation, Owner use, three trading days and final C_R2 remain open.
