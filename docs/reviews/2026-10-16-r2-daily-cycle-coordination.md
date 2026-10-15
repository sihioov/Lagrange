# R2 I4a daily cycle coordination

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09, WP-4/I4.
Status: coordinator source preparation; hosted verification pending. This is a
bounded part of I4, not the complete daily worker or runtime installation.

## Scope and boundaries

The three new modules under `crates/job-queue/src/owner_beta/runner/` implement
process mode, dedicated admission/schedule sessions, and calls to the already
accepted 0060 SQL authority. The new schedule integration target has 14 test
definitions; 12 require a scratch PostgreSQL database and two exercise mode
handling without opening a connection. Every definition is WRITTEN_UNEXECUTED
for this package. Synthetic SQL policy/evidence fixtures are not approved
filesystem inputs or real provider evidence.

The I4 allowlist is extended only to the new test support file
`crates/job-queue/tests/owner_beta_daily_support/mod.rs` and the dedicated PG
test selection in `deploy/db/integration-validation/validate.sh`. Support is
shared so later I4 publication/recovery tests can use the same actual-role
fixture without editing the existing general scratch helper. No new worker,
provider call, production operation, SQL migration or privilege grant is part
of this package. Historical runner execution remains unchanged.

## Direct source disposition

- [Process boundary](../../crates/job-queue/src/owner_beta/runner/daily_runtime.rs):
  absent/`disabled` mode returns before acquiring a connection. Only exact
  `owner_only` enables a cycle. Each enabled cycle checks both `session_user`
  and `current_user`, rejects privileged roles, then obtains a nonblocking
  session advisory lock for its stage. `close_on_drop` is set before the first
  query. Explicit completion unlocks and closes; cancellation or an error
  closes the dedicated connection instead of returning a locked session to
  the pool. The deployment pool and bootstrap contract still own full role
  membership/isolation validation.
- [Scheduler](../../crates/job-queue/src/owner_beta/runner/daily_schedule.rs):
  the DB-clock minute slot commits before discovery or reservation. Discovery
  is latest-only through 0060; 1,025 returned rows fail the 1,024 resource bound
  before any reservation. Each candidate uses the current canonical strategy
  snapshot and its observed epoch; SQL rechecks current authority and returns
  exactly RESERVED, REPLAY, DEFERRED or BLOCKED. A lost commit reply produces
  CommitUnknown without an in-cycle retry. The permanent reservation key is
  responsible for convergence on a later permitted cycle.
- [Admitter boundary](../../crates/job-queue/src/owner_beta/runner/daily_admission.rs):
  a DB minute receipt commits before the caller scans files. Calendar SQL
  accepts a verified projection and binds Owner/policy for the cycle. Admission
  requires a reopened sealed parent and sealed source frontier with matching
  admission/policy/end; both are revalidated before and after the SQL call.
  ADMITTED, REPLAY and BLOCKED are committed outcomes. Unattributable failures
  complete only INPUT_PENDING, INPUT_BLOCKED or IO_FAILED without an invented
  frontier/event hash. An abandoned cycle leaves an incomplete receipt and
  releases its session lock.
- [Test support](../../crates/job-queue/tests/owner_beta_daily_support/mod.rs):
  actual worker/admitter/app logins are separate from the scratch supervisor.
  App writes carry the fixture Owner actor. A completed test body closes its
  role pools, drops the scratch database, and independently checks that the
  database is absent. Error diagnostics expose only a finite runtime code or
  a five-character SQLSTATE. The copied 0060 evidence fixture retains its
  synthetic status and bounded calendar span.

## Written distinctions

[The 14 definitions](../../crates/job-queue/tests/owner_beta_daily_schedule.rs)
cover default-off/invalid configuration, no connection while disabled,
absent/disabled control, canonical reservation and durable minute gate,
permanent replay across sessions, two concurrent schedulers, stage lock before
poll receipt, direct role rejection including supervisor `SET ROLE worker`,
admission lock beyond an expired minute slot, abandonment/lock release with
incomplete receipt, missing fresh input, competition with four manual jobs,
current opt-in/config epoch, and rejection of an older verified scan after a
new INPUT_PENDING receipt.

The lock lifetime test advances only the privileged fixture's persisted
`next_poll_at`; it does not replace the production clock or wait for a minute.
The abandonment probe has a five-second deadline and checks the real advisory
lock. A green workspace target with absent DATABASE_URL is insufficient:
dedicated PG must execute all 14 named results once, exit zero, and reject any
SKIP marker, while preserving the existing 13 DB selections.

## Limits and remaining I4 work

This source disposition is direct coordinator work, not independent review.
No local compiler, tests, SQL, project helper, checker, Docker or runtime was
executed. Only explicit-file edition-2024 rustfmt is permitted locally.

The binary does not yet orchestrate a complete producer/admitter cycle. Actual
source attribution/block events, lost admission reply readback, approved-base
filesystem traversal, retained snapshot revalidation around calendar SQL,
distinct UID/ACL deployment and crash/fsync/rename evidence remain open.
Daily success/failure settlement, claim dispatch and strict daily recovery
binding also remain. I5-I7, integrated independent review, diagrams before
main, concrete G4 supply changes, runtime, Owner use, three trading days and
final C_R2 are not accepted by this package.
