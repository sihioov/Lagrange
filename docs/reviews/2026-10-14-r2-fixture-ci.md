# R2 fixture CI evidence

## Run identity and terminal outcome

Observed at 2026-10-13 15:43:33 UTC / 2026-10-14 00:43:33 KST. Existing [GitHub Actions run 38420816275](https://github.com/sihioov/Lagrange/actions/runs/38420816275) used workflow `CI`, event `workflow_dispatch`, branch `excited-dolphin`, and exact commit `f8b9296c14cb48dacdf8cacefab6c2abff92bf24`. It completed successfully after starting at 15:17:43 UTC; terminal run metadata updated at 15:38:41 UTC. The `required` aggregate also completed successfully.

All seven checkout-bearing jobs completed checkout successfully. Each job log contained the exact full SHA after the checkout action's `git log -1 --format=%H` step. The checkout action ref was `3d3c42e5aac5ba805825da76410c181273ba90b1`.

| Job | Result | Step totals |
| --- | --- | --- |
| `policy` | success | 26 success |
| `workspace-tests` | success | 13 success |
| `postgres-integration-validation` | success | 6 success, 1 skipped conditional failure-log step |
| `web` | success | 14 success |
| `clippy` | success | 5 success |
| `python-tests` | success | 11 success |
| `format` | success | 5 success |
| `required` | success | 3 success |

The web `npm test`, web build and candidate E2E steps succeeded. Python `pytest` and backup-policy test steps succeeded. No separate safe test-count summary was extracted for the web or Python jobs.

## Database and test evidence

The exact-revision workflow declares a fixed disposable `DATABASE_URL` at `.github/workflows/ci.yml:106`; its value was not recorded. The QA Compose startup and cleanup steps both succeeded. The `Owner Equity V2 chart HTTP acceptance tests` step ran `cargo test -p api-server --test http_owner_equity_v2_chart --locked` and passed with 6 passed, 0 failed, and 0 ignored.

The full workspace command `cargo test --workspace --locked --no-fail-fast --features kis-client/test-support` passed. Its log contained 235 Cargo test-result rows: 2,892 passed, 0 failed, and 6 ignored. These are summed result rows, not unique test counts. The separate chart result adds one row with 6 passed, for 236 rows and 2,898 passed overall; no failed test names were found.

All seven requested Owner scheduler/fixture tests passed in the workspace log:

- `schedule_fixture_entitlement_window_covers_year_boundary_candidates` — `ok`
- `owner_equity_schedule_rechecks_late_calendar_after_older_job_settles` — `ok`
- `tests::schedule_cadence_failed_attempt_uses_the_same_sixty_second_throttle` — `ok`
- `tests::schedule_cadence_cutoff_change_does_not_bypass_monotonic_throttle` — `ok`
- `tests::schedule_cadence_initial_poll_and_restart_are_immediate` — `ok`
- `tests::schedule_cadence_successful_zero_work_old_close_retries_same_key` — `ok`
- `tests::schedule_cadence_long_attempt_waits_sixty_seconds_after_completion` — `ok`

## Policy and PostgreSQL validation evidence

The policy job succeeded, including `scripts/validate-foundation.sh`, `deploy/db/integration-validation/validate.sh --self-test`, `scripts/ops/production-ops-static-check.sh`, and `scripts.ci.test_ci_contract`. The captured policy log did not contain an explicit backfill/daily self-test result marker. I report the successful step states only and do not infer individual self-test results.

The PostgreSQL validation job and sanitized-evidence step succeeded. Evidence reported `verdict=APPROVED`, 59 migrations through version 59, 6 stages, 10 named checks, `0039_down_undelivered_guard=PASS`, and `skip_markers=ENFORCED`. The migration checksum field supplied 59 unique SHA-256 digests; they are retained in the private checksum file. `ENFORCED` records validator guard status and does not prove absence of every hidden skip mechanism. The one skipped failure-log step had a failure-only condition and the validation job succeeded.

## Provenance, scope, and limits

Private [sanitized summary](/data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/ci/f8b9296/summary.json:1) records run/job/step states, timestamps, selected results, checkout verification, and per-job original log-response byte counts and SHA-256 values. [Migration checksum evidence](/data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/ci/f8b9296/migration-checksums.json:1) retains the 59 checksums and their source-log provenance. Raw logs were streamed through in-memory filters and were not persisted.

This is CI evidence for the specified run. No run was dispatched, rerun, cancelled, or modified by this task. No local tests, Cargo, build, database, Docker, sudo, provider, production, or lifecycle commands were run. Coordinator review remains necessary; CI completion is not final R2 acceptance.

## Coordinator acceptance

On 2026-10-14 KST the coordinator recovered this report after actual worker idle
with no pending permissions. The original report SHA-256 was
`321b152b82d031ce1aa8c1f5f43b7da56ce1cb8029c5aad296ef0f27d0a6925f`;
the summary and checksum-file hashes matched the worker's receipts. The coordinator
rechecked terminal GitHub metadata, exact SHA, seven target results, aggregate
counts and private modes. This append also clarifies that the seven listed tests
are Owner scheduler/fixture tests; individual F1 shell results were not exposed
in the captured CI log. Their separately recorded local self-tests remain the
direct F1 execution evidence. CI and corrected-fixture source review are accepted
for this partial candidate, with the limits above. Final C_R2, deployment,
new daily-input admission and actual Owner/three-trading-day acceptance remain open.
