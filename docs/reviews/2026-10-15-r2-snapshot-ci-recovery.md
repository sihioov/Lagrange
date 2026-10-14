# Wave30 snapshot CI recovery and failure disposition

Candidate `245f4d9a911d03d58936f177904588826031d83e` failed
[CI 38594682124, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38594682124).
There are two compilation defects at three source locations. Source/compiler/test
acceptance is withheld. This supplement preserves the
[collector report](2026-10-15-r2-snapshot-ci.md) and its private receipt unchanged.

## Lifecycle and recovery scope

Collector `6c5cc4eb-7247-415a-be5b-6cf83a189730` was actually idle at
2026-10-14T16:17:06.762Z, activeTurn null and pending requests 0. The full
20-line substantive final was recovered at 16:17:20 UTC and persisted with SHA256
`45fe5121c38eefd5d10217087e8a1bc461190b798621ebe827492fcd4b24ee28`.
Heartbeat `de67d496` was deleted at 16:18:05 UTC after that recovery.
No worker was redirected or delegated further work.

The original report is 51 lines,
SHA256 `0e2647ec3bff95adae411d938297e05bcddfad35f4080e4b799ebb57acbcdce6`;
the original evidence receipt is 2,760 lines,
SHA256 `0a386ae0abf14ad557b3bee25c9c205c297b04fc1b5b85b6de7fd27b708628e8`.
Its explicit extraction gaps are historical evidence, not accepted as unavailable
GitHub evidence. The coordinator retrieved each completed job log once and reused
its content in memory. No raw log body was written to disk.

## Failure findings and bounded next correction

| ID | Observed compiler evidence | Concrete source condition | Required bounded correction |
| --- | --- | --- | --- |
| C03-01 | E0599 in Clippy, chart compilation and PG `api-tenancy-rls`; [source.rs:918](../../crates/market-data/src/daily_price_only/source.rs#L918) and [source.rs:926](../../crates/market-data/src/daily_price_only/source.rs#L926) | Both guards name `Errno::ACCES`. [Cargo.lock](../../Cargo.lock#L3789) pins rustix 1.1.4, whose cached Linux and libc errno definitions expose `ACCESS`; `ACCES` is only a documentation alias. | Correct those two identifier spellings to `ACCESS`; preserve the `PERM` branch and fieldless `PermissionDenied` mapping. |
| C03-02 | Clippy's lib-test compilation additionally reports E0425 at [source.rs:3839](../../crates/market-data/src/daily_price_only/source.rs#L3839) | The descriptor preflight regression calls undefined `component_count`. The only other occurrences are a local variable inside [root_components](../../crates/market-data/src/daily_price_only/source.rs#L1019). | Restore an independent private fixture component-count oracle while preserving the exact `C + 7` assertion, pre-open checks and lock-last cleanup assertions. |

These are observed compiler failures, not inferred lint findings. Clippy invoked
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
and exited 101 during compilation. Fixing these sites does not establish that
later lints or tests will pass. No source correction, push, new dispatch, rerun or
cancellation was performed during this recovery.

The earlier Wave28/C02 reports remain preserved source-review evidence. Their
scope did not provide compiler acceptance. The next correction must retain the
49 written S2 definitions, accepted P1–P10 contract and existing FR/R/R12/C01/T
conditions; no public API or runtime/permission expansion is needed.

## Exact candidate and job evidence

The coordinator matched all 51 frozen pins: 45 candidate Git blobs and six private
artifact references. The three non-document paths since `a888cd6` are the snapshot
source, integration test and module registration; all equal reviewed `bcc66d3`
bytes. One complete job page contains eight jobs and 84 steps. All seven execution
jobs printed the exact candidate SHA from checkout's `git log -1 --format=%H`;
the aggregate job has no checkout step.

| Job | Outcome | Evidence and limit |
| --- | --- | --- |
| format | PASS | Hosted `cargo fmt --all -- --check` succeeded. |
| clippy | FAIL | Two E0599 sites and the lib-test E0425 site; compiler exit 101. Complete lint acceptance remains unavailable. |
| workspace-tests | FAIL | Explicit chart command failed to compile `market-data`, exit 101, before a test runner. Workspace Cargo test step was skipped. QA compose cleanup succeeded. |
| postgres-integration-validation | FAIL | Validator exit 1; `DENIED: DB-gated test failed: api-tenancy-rls`. Its captured Cargo log has the same two E0599 sites. Nested Cargo exit number was not printed. |
| web | PASS | OpenAPI, lint, typecheck, tests, build and E2E steps each succeeded. |
| policy | PASS | Named static/self-test wrappers and 10 directly printed CI contract unit cases succeeded. Other unprinted subcases remain UNKNOWN; no fakeroot/skip marker was observed in this job log. |
| python-tests | PASS | Pytest: 485 passed, 10 skipped, 7 warnings. Backup-policy step succeeded. Quiet readiness/SIGTERM individual nodes remain UNKNOWN. |
| required | FAIL | Aggregate gate rejected the failed job results. |

All **99 expected Rust cases are NOT_EXECUTED** in the skipped workspace test
step: 30 wire, 18 S1, one collector, two helpers, 19 S2 private and 29 S2 integration.
The **six separate chart cases are also NOT_EXECUTED**: the explicit command
failed compilation before any test runner or named result. This resolves their
original UNKNOWN status using phase evidence, not absent-row inference alone.
Both platform exclusions remain EXCLUDED_LINUX. No named result for any of these
105 cases appears in the retrieved logs.

## PostgreSQL evidence before the terminal failure

The sanitized evidence JSON at job-log lines 174–192 identifies the exact
candidate and the failed `api-tenancy-rls` label. Its failure log group begins at
line 1189; the compiler diagnostics are at lines 1358 and 1369. The earlier
successful labels are `migration-contract`, `auth-audit-readiness` and
`auth-router`, with 36, five and one directly printed test rows respectively.
These are separate from the expected 99 Rust cases.

- Both full-release passes report 59 migrations, latest version 59 and identical
  checksums. The coordinator matched each of the 59 SHA384 checksums to the
  candidate's migration file bytes. The second pass is NO_OP.
- Staged 0038→0039/0040/0041 and reruns, the 0039 undelivered rollback guard,
  catalog install/conflict/negative probes, and identity-preserving rerun have
  fixed success evidence. This does not close broader negative bootstrap/rollback
  readiness or migration 0060.
- Both staged and full-release logs report `daily_input_admitter_catalog=PASS`
  and `daily_input_admitter=daily_input_admitter=none`. Their
  [source assertions](../../deploy/db/integration-validation/validate.sh#L1030)
  check role attributes, memberships/reachability and database/schema privileges.
  Seven direct service-role logins are recorded in each lane.
- Full-release cleanup reports PASS_NO_OWNED_CONTAINERS; its post-cleanup container
  listing is empty. Workspace compose cleanup also succeeded.
- The [DB test wrapper](../../deploy/db/integration-validation/validate.sh#L1377)
  supplies DATABASE_URL and rejects SKIP markers before adding a completed label.
  ENFORCED is a guard configuration, not proof that unprinted later tests ran.
- [Execution order](../../deploy/db/integration-validation/validate.sh#L1696)
  stops at `api-tenancy-rls`. Both `jobqueue-daily-admitter-pool` and
  `api-daily-admitter-pool` are NOT_EXECUTED in this candidate. Their earlier
  `a888cd6` results and UNKNOWN nested-row/exit limits remain separate history.
  Unprinted exit numbers for earlier completed wrappers remain UNKNOWN.

## Safe receipts and preservation

Private coordinator supplement:
`target/r2-daily-beta/ci/245f4d9/ci-recovery-evidence.json`
(3,490 lines, mode 0600), SHA256
`6a24281adf6a4fb6df85cc0654484e956192f3a9db87f7d88a4f0c5f54926ef3`.
It contains all eight job/84 step outcomes, eight decoded-log UTF-8 sizes/hashes,
seven actual checkout SHAs, sanitized diagnostics, named status rows, input pins
and migration checksum comparisons. The directory remains 0700.

The original workspace log's 100,583 figure was a string code-unit count;
the decoded log is 100,745 UTF-8 bytes, SHA256
`5f34be920ad2a616bfbda234adc301f0f9c6c62395180e4a9773e7ef081161ff`.
Previously retained web/PG/required hashes match the coordinator retrievals.
Earlier reports, receipts, partial successes and all failed candidates are unchanged.

Only static source/cached-dependency reads, Git reads, hashes, inert metadata
processing and read-only retrieval of this existing run were used. No local
Cargo/rustc/Clippy/tests/build, project scripts, SQL/DB, Docker, provider or
production actions occurred. No further worker delegation, permission wait or
background work remains.

R2-04/R2-09 remain DOING with C03 compilation corrections and subsequent
verification open. Provider/copy reconstruction/admission, I6 diagrams before
main, runtime, Owner and final C_R2 remain open; no new execution is authorized
by this evidence disposition alone.
