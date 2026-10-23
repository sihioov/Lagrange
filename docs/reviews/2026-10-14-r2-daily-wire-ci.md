# R2 daily input CI evidence — candidate 779b3fa

## Outcome

GitHub Actions run [38500926595](https://github.com/sihioov/Lagrange/actions/runs/38500926595) reached terminal state **completed / failure**. It was `workflow_dispatch` on `excited-dolphin`, exact head `779b3faa787922032c682b9cf3fe07675a3d976f`. Created and started at `2026-10-14T02:25:19Z`; terminal state was observed at `2026-10-14T02:33:49Z`.

This evidence applies only to this candidate. It does not establish I0 runtime readiness, full C_R2 acceptance, or production behavior.

## Job outcomes

| Job | Result | Started → completed (UTC) |
| --- | --- | --- |
| policy | success | 02:25:26 → 02:25:57 |
| format | success | 02:25:25 → 02:25:44 |
| clippy | failure | 02:25:26 → 02:27:25 |
| web | success | 02:25:26 → 02:33:42 |
| workspace-tests | failure | 02:25:26 → 02:28:03 |
| python-tests | success | 02:25:26 → 02:26:58 |
| postgres-integration-validation | failure | 02:25:26 → 02:32:14 |
| required | failure | 02:33:45 → 02:33:48 |

Each of the seven execution-job logs contains the checkout action's `git log -1 --format=%H` result equal to the full candidate SHA. The `required` aggregate job has no checkout step and failed after its dependencies.

## Workspace and compiler evidence

The Owner Equity V2 chart HTTP acceptance step failed. The following full workspace command was marked **skipped**: `cargo test --workspace --locked --no-fail-fast --features kis-client/test-support`. The QA compose up and down steps were successful, but this does not turn the skipped test command into a pass.

The exact candidate source `crates/market-data/tests/daily_price_only.rs` declares 16 tests. **All 16 were not run in this CI job** because the workspace command was skipped; no test-level result was present. The required four cash/fixture tests are among these 16. The separate `owner_equity_schedule_rechecks_late_calendar_after_older_job_settles` test and all five requested `schedule_cadence_` results were likewise not run/observed. Historical V2/V3 regression summaries and the earlier F1/F2 test results were not supplied by this run.

Clippy failed with compiler diagnostics in `crates/market-data/src/daily_price_only/contract.rs`: E0369 at 2359:24 (`DailyWireTimestampV1` comparison), E0282 at 2600:9, 3050:9, and 3645:9 (`Option<_>` type annotations), plus an unused-mutability warning at 3988:9 promoted to failure by `-D warnings`. The workspace log also ended on market-data compilation errors; the hosted job outcome remains the authoritative failure state. The chart-step failure had no more specific safe diagnostic in the captured evidence.

## Web and Python

Web completed successfully: OpenAPI check passed; 61 web test files and 487 tests passed; candidate web E2E reported `passed`, `exit=0`, and `coverage=complete`.

Python completed successfully: pytest reported 482 passed, 10 skipped, and 7 warnings; backup policy tests reported 6/6 passed. These are reported counts only; no claim is made that every hidden or conditional skip was absent.

## PostgreSQL validation evidence

The PostgreSQL validation job failed. Its sanitized evidence reported migration count 59 and latest version 59. First and second migration checksum inventories each contained 59 entries and had the same SHA-256 inventory digest `de2c1e6d2cdc7064e980afdaf6fac7f66d406ff1217f0642eb0d452fbbff5077`; the reported rerun was `NO_OP`.

Selected emitted checks were `0039_down_undelivered_guard=PASS` and `skip_markers=ENFORCED`. Direct and full-release service-role login validation listed `migration_owner,app,worker,audit_writer,research_writer,admin`. The sanitized failure log also contained SQLSTATEs 23505, 23514, and 55000. These are recorded observations only; no cause is inferred. No explicit `APPROVED` marker was supplied. Migration version/checksum evidence and selected guards do not override the job's failure conclusion.

Policy job steps for foundation, integration-validation static/self-test, compose candidate/backtest/code-commit checks, migration static check, and listed deployment/ops static checks completed successfully. This run did **not** explicitly evidence completion of backfill or daily self-tests. The logs do not prove the absence of all hidden skips. The newly added `daily_input_admitter_pool` helper's direct-login/GUC/pool behavior was not established because the full workspace test command did not execute.

## Evidence provenance

The private sanitized receipt [ci-evidence.json](../../target/r2-daily-beta/ci/779b3fa/ci-evidence.json) records job IDs, step-level terminal states, timestamps, exact checkout identity, original log-response byte counts and SHA-256 hashes, selected test and validation summaries, and safe compiler locations. Raw logs were processed in memory only and were not persisted. Receipt SHA-256: `aa44df387554f6f3c76a047f97404a679dcc819ce1e9085c83dc5f4b5ddc14ca`.

## Unresolved / not verified

- CI is terminal but failed; clippy, workspace-tests, PostgreSQL validation, and required aggregate remain failures for coordinator review.
- Chart acceptance failure detail was not available in safe selected output; no cause is asserted.
- The 16 daily-price-only tests, five schedule-cadence tests, late-calendar regression, historical V2/V3 suites, and earlier F1/F2 tests have no execution evidence from this run.
- PostgreSQL's sanitized output did not include an `APPROVED` marker; no full test-level or root-cause conclusion is made from the failure logs.
- Backfill/daily self-test completion, absence of hidden skips, I0 runtime readiness, and full C_R2 acceptance remain unverified.
- No further decomposition is required for this evidence collection; coordinator owns correction and acceptance decisions.
