# R2 role and Raw CI evidence

Collected from existing [GitHub Actions run 38530582876](https://github.com/sihioov/Lagrange/actions/runs/38530582876), attempt 1, exact candidate `f9dfffb9a5d4bfc38a9df347af0de154dbe6d11b`, branch `excited-dolphin`, `workflow_dispatch`. It started at 07:46:54Z and completed with **failure** at 07:57:15Z. All seven execution-job logs contain the exact checkout SHA. No workflow action was issued during collection.

## Job outcomes and failures

`format` and `python-tests` succeeded, as did `web`. `policy`, `clippy`, `workspace-tests`, and `postgres-integration-validation` failed. The `required` aggregate failed. All job and step terminal states/times, checkout evidence, and original log response byte counts and SHA-256 values are in the private [evidence receipt](../../target/r2-daily-beta/ci/f9dfffb/ci-evidence.json).

Strict Clippy failed with exit 101 and Rust `E0004` non-exhaustive `StoreError` matches at `data-pipelines/collectors/src/pipeline.rs:280` and `:337`, and `data-pipelines/collectors/src/worker.rs:582`; the enum definition is anchored at `crates/market-data/src/storage.rs:117`. The chart acceptance Cargo step failed with the same compiler error before its test harness ran (exit 101). The subsequent `cargo test --workspace --locked --no-fail-fast --features kis-client/test-support` step was skipped. The QA database teardown step succeeded.

The policy job's `validate.sh --self-test` step succeeded and emitted `PG_VALIDATION_SELF_TEST: PASS`; no individual nested case result rows were printed. The next `deploy/db/migrate-static-check.sh` step failed, reporting `MIGRATE_STATIC: db-role-bootstrap provisioner entry count changed (expected 7)` (exit 1). The later runtime-secrets and ops static-check steps were skipped; `SECRETS_RUNTIME_STATIC` and `OPS_STATIC` markers were absent. No fakeroot or numeric-fixture skip notice appeared in the collected logs.

## Test and hosted validation evidence

The supplied inventory and all 23 contained source hashes matched the local source bytes; the exact-candidate workflow hash also matched the inventory. The inventory's 30 wire tests and 18 Linux Raw tests all have explicit `NOT_EXECUTED` outcomes because the workspace Cargo step was skipped. The separate non-Unix test `fails_closed_without_unix_descriptor_relative_opens` is `EXCLUDED_LINUX`, not a pass. The six chart test names are listed in the receipt; none has a direct result row. No direct historical V2/V3 rows or workspace Cargo result-row aggregate were available.

Python reported 485 passed, 10 skipped, and 7 warnings. Quiet output had no per-node result rows for the three readiness mocks or `test_sigterm_stops_runtime_and_releases_account_lock`; individual results remain unknown. Web reported OpenAPI in sync with 92 operations linted, 61 test files / 487 tests passed, and candidate E2E `state=passed`, exit 0, complete coverage.

PostgreSQL validation returned `DENIED` for the exact code commit and identified `DB-gated test failed: api-tenancy-rls`. Its separate `tests_run` list was `migration-contract`, `auth-audit-readiness`, and `auth-router`; the summary does not map that list to the named failure. The first and second full-release runs each reported 59 migrations/latest version 59 with matching 59-entry checksum lists (list SHA-256 `de2c1e6d2cdc7064e980afdaf6fac7f66d406ff1217f0642eb0d452fbbff5077`); rerun was `NO_OP`. The 0039 undelivered down guard passed, skip markers were enforced, and cleanup reported `PASS_NO_OWNED_CONTAINERS`. The SQLSTATE 23505/23514 negative probes and 55000 rollback guard are recorded separately as guard evidence, not as the validator's stated failure.

Both staged and full-release service-role login lists include `daily_input_admitter`; the corresponding login log groups emitted the role name. The catalog marker `daily_input_admitter_catalog=PASS` appeared twice. The role-GUC output appeared twice as `daily_input_admitter=daily_input_admitter=none`; its components are not further interpreted and this is not `app.actor_id` evidence. Detailed effective-privilege tuples and a typed bootstrap credential-gate verdict were not exposed in the selected sanitized fields; the credentials field was not inspected. The uncalled `daily_input_admitter_pool`, its direct helper behavior, and maximum-two-pool behavior remain unverified.

## Scope, deviations, and limits

Collection used read-only GitHub run/job/log queries, exact-candidate workflow/chart-source reads, and local inventory/hash/JSON checks. Raw logs were filtered in memory and not persisted. Two read-only `rg` searches exceeded the requested narrow-source boundary while locating the chart and database-validation references; one search result included a test-fixture credential literal, which was not copied to disk or the report/receipt. No scripts were executed and no files were changed by those searches. No local tests, builds, validators, databases, providers, or operational actions were run.

The CI failures above require coordinator review; this evidence does not infer their broader cause. Hidden-skip absence, migration 0060, production/runtime/provider behavior, Owner acceptance, and final C_R2 acceptance remain unverified. No permission waits or background work remain. Further decomposition is not needed for this bounded collection.
