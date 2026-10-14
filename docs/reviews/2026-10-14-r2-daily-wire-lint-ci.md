# R2 daily-wire lint-correction CI evidence

Collected 2026-10-14 from existing GitHub Actions run [38511544759](https://github.com/sihioov/Lagrange/actions/runs/38511544759), attempt 1. The run used `ci.yml`, `workflow_dispatch`, branch `excited-dolphin`, and exact candidate `d6bb23a6c516c8a9ebffbd93229c0b92190db48b`. It started at 04:28:16Z and completed at 04:49:18Z with **failure**. No workflow action was issued by this collection.

## Job outcomes

All seven execution jobs checked out the requested SHA in their logs. `policy`, `format`, `workspace-tests`, `web`, `python-tests`, and `postgres-integration-validation` completed successfully. `clippy` failed; the `required` aggregate then failed with `CLIPPY_RESULT: failure` (exit 1). The full sanitized job and step statuses, timestamps, checkout evidence, and original-log byte counts/SHA-256 values are in the private [evidence receipt](../../target/r2-daily-beta/ci/d6bb23a/ci-evidence.json).

Strict Clippy ran `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and exited 101. Its diagnostic is `method daily_input_admitter_pool is never used` at `crates/job-queue/tests/common/mod.rs:360`; `-D dead-code` was promoted by `-D warnings`. The logs show the method was unused in that Clippy target. This run therefore does not establish its direct-login, GUC, or pool behavior.

## Test and validation evidence

- The `daily_price_only` candidate inventory contains 30 named tests (5 private unit tests and 25 integration tests). Each name had a direct `... ok` row in the workspace job: **30 passed, 0 failed, 0 ignored**.
- Owner Equity V2 chart HTTP acceptance produced direct `... ok` rows for all six expected tests. The QA database setup and teardown steps both succeeded.
- Across 236 Cargo `test result` rows in the workspace-test step, the sums were 2,922 passed, 0 failed, 6 ignored, 0 measured, and 180 filtered out. These are sums across result rows, not a unique-test count. Direct historical evidence included 55 `owner_equity_v2::tests` rows and 7 `v3_historical_input::tests` rows, all `ok`.
- Python ran `uv run --project nt pytest -q`: **485 passed, 10 skipped, 7 warnings**. Quiet output contained no per-node result rows for the three new readiness mocks or the existing SIGTERM test; their individual outcomes cannot be identified from this log. The six backup policy tests also passed.
- Web checks passed: OpenAPI was in sync and linted 92 operations; web tests reported 61 files and 487 tests passed; candidate web E2E reported `state=passed`, exit 0, complete coverage.
- PostgreSQL validation reported `APPROVED` for this exact code commit. The full-release first and second runs both reported migration count/latest version 59 and identical 59-entry checksum lists (list SHA-256 `de2c1e6d2cdc7064e980afdaf6fac7f66d406ff1217f0642eb0d452fbbff5077`); rerun was `NO_OP`. The `0039_down_undelivered_guard` was `PASS`, skip markers were `ENFORCED`, and the sanitized validation summary says DB-gated tests passed without `SKIP`. The isolated upgrade sequence recorded baseline `0038_schema_only` and migrations 0039–0041. This is hosted validation evidence, not production database evidence.

## Scope and limits

Evidence came from read-only `gh api` requests for the existing run, jobs, and job logs, plus local reading of the supplied test inventory. Raw logs were filtered in memory and not saved. The private receipt preserves selected outcomes, exact candidate test names, typed Clippy location, timestamps, and original log response sizes and hashes.

This CI run does not establish absence of hidden skips, direct behavior of the unused `daily_input_admitter_pool` helper, migration 0060 readiness, production/runtime/provider behavior, Owner acceptance, or full C_R2 acceptance. No further decomposition is needed for this bounded evidence collection; any source correction or acceptance decision remains with the coordinator.
