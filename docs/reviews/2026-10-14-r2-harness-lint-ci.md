# R2 harness lint-correction CI evidence

Collected from existing [GitHub Actions run 38514204062](https://github.com/sihioov/Lagrange/actions/runs/38514204062), attempt 1, for exact candidate `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`. The run used `ci.yml`, `workflow_dispatch`, and branch `excited-dolphin`; it started at 04:58:56Z and completed successfully at 05:17:46Z. No workflow action was issued during collection.

## Jobs and tests

All seven execution jobs checked out the requested SHA in their logs. `policy`, `format`, `clippy`, `workspace-tests`, `web`, `python-tests`, and `postgres-integration-validation` succeeded; the `required` aggregate also succeeded. Job and step states/timestamps, checkout evidence, and original log response byte counts and SHA-256 values are in the private [evidence receipt](../../target/r2-daily-beta/ci/929fcd6/ci-evidence.json).

The exact source inventory matched 5 private and 25 integration tests. Each of the 30 names had a direct `... ok` row: 30 passed, 0 failed, and 0 ignored. Owner Equity V2 chart HTTP acceptance had six direct passing rows. QA database setup and teardown both succeeded.

Across 236 Cargo `test result` rows, workspace totals were 2,922 passed, 0 failed, 6 ignored, 0 measured, and 180 filtered out. These are summed result rows, not unique tests. The logs also contained 55 direct passing `owner_equity_v2::tests` rows and 7 direct passing `v3_historical_input::tests` rows. Strict Clippy passed with `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.

Python ran `uv run --project nt pytest -q`: 485 passed, 10 skipped, and 7 warnings in 58.66 seconds. Quiet output had no per-node result rows for the three readiness mock tests or `test_sigterm_stops_runtime_and_releases_account_lock`; their individual outcomes are not established by this log. Web evidence: OpenAPI in sync with 92 operations linted, 61 web test files / 487 tests passed, and candidate E2E reported `state=passed`, exit 0, complete coverage.

PostgreSQL validation reported `APPROVED` for code commit `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`. Both full-release passes reported 59 migrations/latest version 59 and identical checksum lists (list SHA-256 `de2c1e6d2cdc7064e980afdaf6fac7f66d406ff1217f0642eb0d452fbbff5077`); rerun was `NO_OP`. The 0039 undelivered down guard passed, skip markers were enforced, and the sanitized validator summary reports DB-gated checks passed without `SKIP`. The separate staged sequence recorded baseline `0038_schema_only` and migrations 0039–0041.

## Scope and limits

Collection used read-only `gh api` calls for the fixed run, jobs, and job logs, plus local source-hash, inventory, and receipt checks. Raw logs remained in memory and were not persisted. The receipt contains selected safe outcomes and original response sizes/hashes only.

This run verifies the scoped lint annotation compiles; it does not establish direct-login, GUC, or max-two-pool behavior of `daily_input_admitter_pool`. Hidden-skip absence, migration 0060 readiness, production/runtime/provider behavior, Owner acceptance, and final C_R2 acceptance remain unverified. No deviations or permission waits occurred. No further decomposition is needed for this bounded collection.
