# R2 daily wire corrections CI evidence

## Run identity and outcome

Run [38506113191](https://github.com/sihioov/Lagrange/actions/runs/38506113191), attempt 1, was `workflow_dispatch` on `excited-dolphin` at exact candidate `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff`. It completed with conclusion **failure** at 2026-10-14 03:36:21 UTC. This report uses only this run; prior candidate CI evidence was not reused.

All seven execution jobs that checked out source logged the exact candidate SHA. Job outcomes: policy success (03:25:29Z), format success (03:25:09Z), Clippy failure (03:26:59Z), Python tests failure (03:26:31Z), web success (03:33:02Z), workspace tests failure (03:32:47Z), PostgreSQL integration validation success (03:36:14Z). The `required` aggregate failed at 03:36:20Z because required jobs did not all succeed.

## Failure evidence and completed checks

- **Clippy:** `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` exited 101. The current-candidate log reports three collapsible-if lints at `crates/market-data/src/daily_price_only/contract.rs:1334:17`, `:1339:17`, and `:2007:9`, plus two type-complexity lints at `:4003:21` and `:4051:21`.
- **Workspace tests:** the Owner Equity V2 chart command passed all six tests (6 passed, 0 failed). The subsequent workspace Cargo command exited 101 at compile time: `E0618`, `crates/market-data/tests/daily_price_only.rs:2277:37`, “expected function, found Fixture.” The workspace cleanup step completed successfully afterward. None of the 24 integration or five private-unit tests in the recovery receipt produced a result row; all 29 are recorded as not executed. No separate historical V2/V3 regression result rows were found in this run.
- **Python:** pytest exited 1. The current log reports `nt/live-node/tests/test_runtime.py::test_sigterm_stops_runtime_and_releases_account_lock` failed; summary was 1 failed, 481 passed, 10 skipped, 7 warnings.
- **Web:** OpenAPI check passed; 61 web test files and 487 tests passed; candidate web E2E reported passed, exit 0, complete coverage.
- **PostgreSQL:** job and validation step succeeded. Sanitized evidence says `APPROVED` for this exact code commit; full-release migration count/latest version were 59 on both runs, and the 59-entry checksum lists matched. The 0039 down-migration undelivered-row guard passed; skip markers were enforced. Expected catalog conflict and guard probes were recorded as `PASS_SQLSTATE_23505` and rolled-back `PASS_SQLSTATE_23514`. The validator listed ten DB-gated tests as run. It does not publish row contents here. Policy job self-test and CI contract unit test steps also passed.

## Candidate test matrix

The receipt records the exact 24 integration and five private-unit test names from the frozen recovery receipt, each with outcome `NOT_EXECUTED_NO_RESULT_ROW`. Test definitions in source are not execution evidence. No direct-login/GUC/pool limits for `daily_input_admitter_pool`, migration 0060, runtime activation, or production acceptance is established by this ordinary CI run. A zero skipped count was not inferred: Python reported ten skipped, and the available logs do not prove the absence of other hidden skips.

## Provenance and limits

Raw verbose logs were processed in memory and not persisted. The private receipt contains run/job/step summaries, safe diagnostics, exact candidate test names/outcomes, selected evidence, byte counts, and SHA-256 hashes of each retrieved log response. It excludes raw bodies, credentials, provider text, and database rows.

- Receipt: `target/r2-daily-beta/ci/02a400e/ci-evidence.json`
- Exact candidate test-name source receipt SHA-256: `8b76b02d852bca1dedd8d3c1e791ed04b328244a4107abd16a82204a836b805f`
- Run status was terminal when collected; no retry, dispatch, cancel, or local test/build action was performed.

## Follow-up

The exact candidate’s Clippy lints, workspace compile error, and Python failure require coordinator/source-owner review. All 29 named tests remain unverified in CI. This evidence does not establish C_R2 acceptance. No further decomposition is needed for this bounded evidence collection.
