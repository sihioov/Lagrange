# R2 I4e-C01 — bounded runner and service-status fixture corrections

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09; Wave57.
Baseline: `93437069c7a5bfdf6a1f8d547eb1adb87530f472`.
The [full failed result](2026-10-16-r2-daily-runner-ci.md) was recovered before edits.
This is direct coordinator source review, not independent or executable acceptance.

## Exact scope

Only two non-documentation files change:

| File | Lines | SHA256 |
| --- | ---: | --- |
| `crates/job-queue/tests/owner_beta_daily_runner.rs` | 437 | `63699b740dc0ad37ff80fcd1b5589c111d1a2b81b88e2a4017fe1467d0d916c8` |
| `data-pipelines/collectors/tests/service_status.rs` | 1352 | `fb6e38f352293ffdbf4775b9e2def036947f61f08609b127890646a3df9060c5` |

The existing runner preflight test now uses the two shared fixture methods for
a bounded later scheduler cycle. It requires one replay of the permanent claimed
reservation, identical payloads, and unchanged jobs, attempts, runs, reservations
and items. Its existing control-writer lock-release assertion remains. No lint
allow, removed assertion, shared-helper or production edit was introduced.

The collector fixture now includes the existing immutable migration-contract
role and per-database bootstrap SQL. Cluster-global roles are initialized under
a transaction-owned canonical advisory lock followed by the legacy fixture lock,
with local lock/statement time bounds. Commit or rollback releases both locks.
Schema grants execute only in the freshly created disposable target database;
the supervisor database schema is not changed. A target setup failure closes its
pool, drops only the generated database and verifies its absence.

Fixed stage and SQLSTATE/error-kind diagnostics are retained on bootstrap and
setup cleanup failures. They do not print URLs, credentials or database prose.
The historical failure discarded its underlying error, so these identified source
defects are corrected without claiming its exact cause has been established.
Existing actual migration execution, six role connections, tests and teardown
remain. No production grants or migration bytes change.

## Static verification and next evidence

The complete two-file diff was reviewed. Standalone edition-2024 rustfmt with
`skip_children=true` and whitespace checks passed. All other runner tests after
the modified first test remain byte-identical: 9,382 bytes, SHA256
`89e8000dfadd3dccaa8c9ced40ac923f9f5898bd3016b89d08db34c26498f4b3`.
The collector suffix from `async fn connect` through EOF remains byte-identical:
43,586 bytes, SHA256 `4c845c73c7727ccfeb88ef68ccc5b4a4862364866e623ffad5ea0b80765a2c54`.
The frozen successor inventory carries the two pure and ten actual-DB runner
definitions, all previous groups, and explicitly adds these affected existing
collector DB names:

- `status_repository_is_fenced_best_effort_and_publication_bound`
- `actual_attempt_takes_over_only_a_strictly_stale_running_fence`
- `down_locks_before_testing_a_concurrent_first_insert`

The collector fixture requires DATABASE_URL and cannot substitute a skipped DB
suite. Fresh compile, strict lint, these three named results, all ten dedicated
daily runner results, every prior gate and complete job recovery are required.
All successor tests remain WRITTEN_UNEXECUTED until that evidence arrives.
One new exact candidate may be dispatched only after pin/blob verification,
all-event existing-run zero queries and remote equality. No old run is repeated.

No local project execution or new delegation occurred. Whole-flow approved-base
evidence, supersession, producer/admitter wiring, I5-I7, integrated independent
review, runtime, Owner, trading-day observation and final C_R2 remain open.
