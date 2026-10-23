# R2 daily wire final CI evidence

## Run identity and outcome

Run [38509087712](https://github.com/sihioov/Lagrange/actions/runs/38509087712), attempt 1, was `workflow_dispatch` on `excited-dolphin`, exact SHA `6429d331bef1bd201c33d49047139587a55a1435`. It started 2026-10-14 03:59:36 UTC and completed with **failure** at 04:19:47 UTC. All seven execution-job checkout logs contain the exact candidate SHA. The `required` aggregate failed at 04:19:47 UTC because Clippy failed; the other six execution jobs succeeded.

| Job | Result | Started–completed UTC |
| --- | --- | --- |
| policy | success | 03:59:40–04:00:18 |
| format | success | 03:59:39–03:59:57 |
| clippy | failure | 03:59:40–04:01:45 |
| postgres-integration-validation | success | 03:59:39–04:10:46 |
| workspace-tests | success | 03:59:40–04:19:42 |
| web | success | 03:59:40–04:06:34 |
| python-tests | success | 03:59:40–04:01:36 |
| required | failure | 04:19:44–04:19:47 |

## Rust, chart, and Clippy evidence

The workspace Cargo step succeeded and cleanup completed successfully. Each of the 30 exact-candidate tests in the pinned inventory has one `ok` result row: 25 integration tests and five private-unit tests. This includes the synthetic golden, empty-cash/cash-code cases, typed proof-date bounds, 16-hop ordering, and collection/size/depth/seal guards. The receipt carries the full test-name/status matrix.

Across 237 Cargo `test result` rows (aggregate rows, not unique tests), the totals were 2,928 passed, 0 failed, 6 ignored, 0 measured, and 180 filtered. The six Owner Equity V2 chart acceptance tests each had an `ok` row. The workspace log contained 161 result rows matching the receipt’s `v2` plus `equity` name filter; all were `ok`. No `v3` plus `equity` result row matched that filter.

Clippy failed at exit 101 under `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`. Its log identifies `clippy::collapsible_if` at `crates/market-data/tests/daily_price_only.rs:913:17`. No compiler error code was reported. This report records the lint evidence without proposing a fix.

## Python, web, and PostgreSQL

Python pytest succeeded: 485 passed, 10 skipped, 7 warnings. The exact-candidate `test_runtime.py` log contains no file-progress line or result row for the three added readiness mocks or the existing SIGTERM test. Their individual outcomes are therefore **unknown at node level**; only the suite-level result is evidenced.

The web job succeeded: OpenAPI check, lint, typecheck, web tests, build, and candidate E2E steps all succeeded. The web test output reported 61 files and 487 tests passed. The candidate E2E runner reported 32 tests run; the workflow step succeeded.

PostgreSQL validation returned `APPROVED` for code commit `6429d331bef1bd201c33d49047139587a55a1435`. Both full-release passes reported 59 migrations, latest version 59, and identical 59-entry checksum lists (aggregate checksum-list SHA-256 `de2c1e6d2cdc7064e980afdaf6fac7f66d406ff1217f0642eb0d452fbbff5077`). The 0039 down-undelivered guard was `PASS`, skip markers were `ENFORCED`, and cleanup reported `PASS_NO_OWNED_CONTAINERS`. The evidence listed ten DB-gated test phases; it does not expose test row contents here.

## Limits and provenance

The Python suite reported 10 skips; workspace aggregates reported 6 ignored and 180 filtered. No claim is made that hidden skips are absent. The uncalled `daily_input_admitter_pool` helper and its direct-login/GUC/pool limits, migration 0060, first-readiness, production, Owner, and C_R2 acceptance remain unverified. A PostgreSQL `APPROVED` result and successful CI do not establish those conditions.

The private receipt contains exact job/step states, checkout-SHA checks, response byte counts and SHA-256 hashes, the full 30-test matrix, chart results, matching historical test rows, and sanitized PG evidence. Raw verbose logs and credential/provider/database row content were not persisted. Earlier CI/PY-D reports and receipts were hash-verified unchanged before writing. No workflow action was repeated and no local tests/builds were run. No further decomposition is needed for this evidence collection; coordinator acceptance remains separate.
