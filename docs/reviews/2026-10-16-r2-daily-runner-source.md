# R2 I4e mixed daily consumer — source review

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09. Coordinator direct review;
not an independent review. Baseline `3073e9e64ca4f12b6a9e8b84398f1696ce99aef8`
has [recovered hosted evidence](2026-10-16-r2-daily-replay-ci.md).
No new compiler, Clippy, test, database or runtime acceptance at this point.

## Exact scope

| File | Lines | SHA256 |
| --- | ---: | --- |
| `crates/job-queue/src/owner_beta/runner.rs` | 668 | `782dcc40db6a4c11f18623c9ce520e0f7645d69cf46c9a35c107d97c63d50b2c` |
| `crates/job-queue/src/owner_beta/runner/daily_execution.rs` | 554 | `2803f3628a5bdcc95cf1a3108a1e6ed4123dfa1d3845a1ec90b9489d4b99a9d9` |
| `crates/job-queue/src/owner_beta/publish.rs` | 1558 | `81df056018a09041cc6486a41ec5e4c75419652171d02c6637232e2c3cced755` |
| `crates/job-queue/tests/owner_beta_daily_runner.rs` | 418 | `6414bad2ff31b700beff310c7e8440472deb90556ec4b3cef718da8bd259f42f` |
| `deploy/db/integration-validation/validate.sh` | 1972 | `9b818f2e221c8592b704fab6a9f9a6e2509b33eed10f2af6d2a28410642e8b76` |

## Implementation and direct disposition

1. [Mixed dispatch](../../crates/job-queue/src/owner_beta/runner/daily_execution.rs:63)
   claims the existing protected job type once, uses the strict union decoder,
   and delegates valid historical input to the extracted unchanged historical body.
   Existing `run_once` stays historical-only. Daily mode defaults to disabled;
   a daily claim in that mode settles with a bounded data-blocked code.
2. [DB preflight](../../crates/job-queue/src/owner_beta/publish.rs:980)
   checks a direct, unprivileged worker principal, exact live claim/attempt,
   immutable daily run and existing full SQL gate. Ready commits no row changes.
   An audited cancellation may finish its existing fenced cleanup atomically.
   Commit-unknown returns indeterminate; it never retries a settlement blindly.
3. [Execution](../../crates/job-queue/src/owner_beta/runner/daily_execution.rs:124)
   checks the installed policy pin, then current DB authority before approving base
   files. A retained global permit bounds started blocking base loads. Those loads
   are read-only and cannot publish; timeout aborts unstarted work. Dropping the
   caller closes the heartbeat stop channel.
4. Full sealed replay and computation run in a multithread runtime's
   `block_in_place`, retaining the actual approved base, policy borrow, tip file
   owners and compute permit through atomic publication. An explicit runtime
   check rejects current-thread use before claim. Deadline/monitor checkpoints
   surround bounded synchronous stages; an individual read/compute operation is
   not preemptible, and timeout is not a hard wall-time bound on all cleanup.
5. Before publishing, the runner joins its lease monitor, checks its deadline,
   heartbeats and revalidates the retained tip. The existing opaque success API
   repeats exact current SQL authority and file/target checks in its transaction.
   No detached task owns a publication future. No wire value constructs an approved
   capability, and no latest-path fallback or alternate registry was added.
6. I/O/interruption failures are retryable; integrity, policy, unsafe path and
   computation rejection remain terminal. New disabled/authority codes are
   data-blocked. Historical failure behavior and success SQL remain unchanged.

## Verification prepared

Two pure retry-class definitions and ten DB definitions are WRITTEN_UNEXECUTED.
The DB cases cover read-only preflight/lock release, disabled control, config
edit-and-restore, payload/attempt fences, audited cancellation and repeat cleanup,
disabled mixed dispatch, runtime/lease checks before claim, policy mismatch before
file approval, and direct app/admitter/elevated-role denial. They use synthetic SQL
fixtures with real role connections; those fixtures cannot approve filesystem data.

The hosted harness adds a seventeenth dedicated target. All ten exact named rows
must appear once with `ok`, numeric exit zero and no SKIP. Existing lanes and
guards remain. Five historical runner tests and all earlier inventories are carried.
Scoped standalone edition-2024 rustfmt and whitespace checks pass. Source-only
preservation comparisons precede the frozen candidate; no local project execution.

## Limits and next gate

No binary activation, SQL migration, queue behavior, approval factory or provider
change. This package does not prove actual approved-base daily success, interruption
during valid replay/computation/publication, superseded-session cancellation, the
producer/admitter executable flow, distinct UID/ACL or crash/installer behavior.
Source inspection is not compilation or execution. Freeze the final inventory and
pins, query existing exact-head runs, verify remote equality and dispatch once.
Full I4/I5-I7, integrated independent review, diagrams before main, operations,
actual Owner use, three consecutive trading days and final C_R2 remain open.
