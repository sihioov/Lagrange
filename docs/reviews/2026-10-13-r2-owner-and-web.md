# R2 Owner and Web Review — WP-5 cadence follow-up

Date: 2026-10-13
Scope: independent Owner V2 WP-5 branch, based on the supplied baseline `cea7baa6298402db79d655140ad4c199f57f8697`. The U2 ETF contract remains pending; later WP-4 integration is still required.

## Source evidence

The recovered source files have these SHA-256 hashes:

| File | SHA-256 |
| --- | --- |
| `crates/job-queue/src/bin/owner-equity-v2-runner.rs` | `bcf25c6f693a401b2b7a883841300bb7012b461e42ea161d95a5f65dd28687e0` |
| `crates/api-server/tests/owner_equity_v2_db.rs` | `a73dc87550808b45070c2ac796ab740ba15e9a0a4a80605834a6e427b7486665` |

Current line anchors for the recovered changes:

- Runner cadence definition: `ScheduleCadence` begins at line 445; `pending_key` and `record_attempt_finished` are its decision points.
- Runner cycle result path: the due check and awaited scheduling cycle begin at line 920; the single post-attempt deadline update precedes the result branches.
- Runner unit tests: the new cadence cases begin at lines 1278, 1292, 1315, 1335, and 1360.
- Disposable DB test: `owner_equity_schedule_rechecks_late_calendar_after_older_job_settles` begins at line 1072 and contains the late-calendar, settlement, idempotency, current-coverage, admission-join, and CLOSED-day assertions.

`crates/job-queue/src/owner_equity_v2/schedule.rs` was not changed. No API, DTO, Web, schema, migration, provider, WebSocket, shared configuration, roadmap, or other contributor files were changed for this WP-5 source diff.

## Behavior recorded

The runner no longer makes an eligible civil date permanently complete after one scheduling cycle. `ScheduleCadence` tracks a monotonic next-attempt instant. After every completed cycle, including successful zero-work/older-close outcomes and errors, the runner sets the next due instant to completion plus 60 seconds. A task that takes a long time therefore incurs a full 60-second delay after it finishes. The cutoff remains 16:30 KST, with no inferred weekdays; a process restart begins with an immediate poll. The scheduler continues to rely on its existing SQL admission, idempotency, and active-job checks. This cadence change adds no provider call and changes no schedule SQL semantics.

The added DB fixture exercises same-day calendar publication while an older incremental job is still active, then settles that job and calls the cycle again. It asserts progress for the current eligible date, one job per close, no duplicate active job, no enqueue for already covered current generations, the existing five-column admission join, and CLOSED/no-work behavior. It also checks rejection of an unregistered rights pin and carries the configured code commit, entitlement reference, and entitlement hash through both incremental payloads. Its adapter is deterministic and in-memory; the test does not call a provider.

## Cadence test cases added

- `schedule_cadence_initial_poll_and_restart_are_immediate`
- `schedule_cadence_successful_zero_work_old_close_retries_same_key`
- `schedule_cadence_failed_attempt_uses_the_same_sixty_second_throttle`
- `schedule_cadence_cutoff_change_does_not_bypass_monotonic_throttle`
- `schedule_cadence_long_attempt_waits_sixty_seconds_after_completion`

Each test supplies monotonic instants directly; none uses a wall-clock sleep.

## Checks and execution status

Static checks recorded as run during source work:

- `rustfmt --edition 2024 crates/job-queue/src/bin/owner-equity-v2-runner.rs crates/api-server/tests/owner_equity_v2_db.rs` — passed.
- `git diff --check && rustfmt --check --edition 2024 crates/job-queue/src/bin/owner-equity-v2-runner.rs crates/api-server/tests/owner_equity_v2_db.rs` — passed.
- Source was statically inspected for the shared result-path deadline update, monotonic cutoff behavior, existing schedule/SQL semantics, and DB fixture assertions.

No Rust test, compiler, database, Docker, provider, production, build, or install command was executed. The pre-fix behavior was not demonstrated with an offline pure-model trace.

The following checks remain unexecuted and require coordinator serialization:

- Cadence tests: `cargo test --locked -p job-queue --bin owner-equity-v2-runner schedule_cadence`.
- Exact DB test: `cargo test --locked -p api-server --test owner_equity_v2_db owner_equity_schedule_rechecks_late_calendar_after_older_job_settles -- --exact`.

The DB test requires `DATABASE_URL` to point to an isolated disposable QA PostgreSQL instance where the harness connection can bootstrap its test roles and create/drop a fresh scratch database, apply embedded migrations, and exercise the actual `app` and `worker` roles. The run must not target production. The harness skips when `DATABASE_URL` is absent, so QA must confirm the test actually executes rather than reports a skip. No weaker mock substitutes for this database test.

Static review found a likely fixture error to resolve before dispatch: the test first verifies that `bad_rights_pins` is rejected, then later passes the same unregistered pin to the no-lineage schedule call and expects success. No rights registration change is apparent between those calls, so that assertion may fail at rights validation before reaching its intended no-lineage check. This has not been executed; correct the fixture within the authorized source scope before treating the DB test as validation-ready.

## Acceptance state and remaining work

No source acceptance or runtime success is claimed. Rust and DB tests remain pending, and scheduler behavior under the executable runner has not been verified. The same-day progression fixture has not yet been executed against the disposable actual-role database. The frozen U2 ETF contract still blocks later WP-4 integration; that integration is required before end-to-end acceptance. These are unresolved checks, not a claim that no issues remain.

This document is the only file written in the reporting follow-up. No Git action was performed, and no background work remains.

## Coordinator delta before executable validation

The initial report's rights-pin question is not a confirmed production defect:
`run_owner_equity_schedule_cycle` invokes the rights-checking SQL function only
for memberships missing admitted coverage. To make the fixture's purpose
unambiguous, the no-lineage call now uses the valid pins; the separate invalid
rights assertion remains. Two concurrent same-key cycles after the older job
settles must return exactly `[0, 1]` insertions, extending the sequential replay
check to concurrent admission. No scheduling SQL or other production file changed.

Updated DB test SHA256:
`62004e8603aee37f095f9cf3f778f274440af14675fba2069c34f63d0624020f`.
`rustfmt` passed after this delta; executable unit/DB validation remains pending.
The independent source reviewer receives these current bytes. Hosted CI uses
its disposable QA database, avoiding a Rust compile on the memory-constrained
production host. This is partial F1/F2 validation, not the final R2 release candidate.

## WP-5/F2 dynamic entitlement fixture repair

Baseline: `3ee453fe38a54ae92cc098daf61b0eaf56f833cb`. This repair is limited to the
WP-5 disposable DB fixture and this appended record. The current test source SHA-256 is
`0bcdabfb1bb7462e216e2fa2f6f29dfeeec881bc7726e40a6233703c4c7d16db`.

The DB-clock eligible date now passes through `schedule_fixture_date_window`. The seam
derives the prior close, initial request session, and first date in the 121-date initial
candidate. Before adding the membership, the test seeds an ACTIVE entitlement into the
fresh scratch database under the unique reference
`wp5-owner-equity-scheduler-date-window`. Its inclusive effective window starts at that
initial candidate date and ends on the eligible date. The fixture keeps its existing
synthetic document hash, dataset, and use set; no production rights policy or schema is
changed.

This test's initial mutation pins and schedule pins now use that same reference and hash.
The fake adapter already copies the job payload pins to `OwnerEquitySourcePins`; an added
initial-payload assertion joins the existing incremental payload assertions to check that
the test-specific pins propagate. The separate unregistered-rights rejection remains.
The coordinator's valid-pins unadmitted-generation case and concurrent same-key result
assertion `[0, 1]` are preserved.

The offline date-window test covers the requested year boundary:

| Eligible date | Prior close | Initial session | Initial candidate start | Entitlement end |
| --- | --- | --- | --- | --- |
| 2026-12-31 | 2026-12-30 | 2026-12-29 | 2026-08-31 | 2026-12-31 |
| 2027-01-01 | 2026-12-31 | 2026-12-30 | 2026-09-01 | 2027-01-01 |

For each date, it checks containment of the initial candidate start/session, prior close,
current candidate start, and eligible date inside the seeded entitlement interval. The
pure Rust test was added but not executed.

Current source locations are anchored by `ScheduleFixtureDateWindow` at line 226,
`seed_wp5_schedule_entitlement` at line 458, the year-boundary test at line 476, and
`owner_equity_schedule_rechecks_late_calendar_after_older_job_settles` at line 1153.
The changed DB call site begins at line 1178. In this report, the new section begins at
line 82. This bounded change edits only the DB test and this report.

Static checks run: `rustfmt --edition 2024 crates/api-server/tests/owner_equity_v2_db.rs`
passed, and `git diff --check` passed. No Cargo test, compiler, database, Docker, provider,
production, build, or install command was run. The exact focused validations remain
unexecuted:

```text
cargo test --locked -p api-server --test owner_equity_v2_db schedule_fixture_entitlement_window_covers_year_boundary_candidates -- --exact
cargo test --locked -p api-server --test owner_equity_v2_db owner_equity_schedule_rechecks_late_calendar_after_older_job_settles -- --exact
```

Hosted CI run `38415230019` uses the earlier pinned source and does not validate this
repair; the coordinator owns execution of the focused commands against the resulting
source. Compilation, the offline Rust test, actual-role DB behavior, and runtime behavior
remain unverified. No source acceptance or runtime success is claimed. No further task
decomposition is needed for this bounded fixture repair.

Final worktree inspection also showed `docs/reviews/2026-10-13-r2-daily-repair.md`
modified outside this task's allowlist. It was not edited here and was left untouched.

## Coordinator source acceptance — 2026-10-14 KST

The corrected source is pinned at `f8b9296c14cb48dacdf8cacefab6c2abff92bf24`.
[CI 38420816275](https://github.com/sihioov/Lagrange/actions/runs/38420816275)
completed successfully, including Rust compilation, workspace tests, PostgreSQL
validation and the required aggregate. This supersedes the earlier unexecuted
source-verification status for the corrected fixture, while preserving its history.
[Independent delta review](2026-10-14-r2-fixture-review.md) closes P2-F2 at the exact
`0bcdabfb1bb7462e216e2fa2f6f29dfeeec881bc7726e40a6233703c4c7d16db` bytes.
F2 source acceptance does not establish deployed scheduling, real Owner/API/UI
acceptance or WP-4 daily-input integration. Those R2-05 conditions remain open.
The [accepted CI evidence](2026-10-14-r2-fixture-ci.md) records the year-boundary
test, actual-role late-calendar regression and all five cadence tests as `ok`.
