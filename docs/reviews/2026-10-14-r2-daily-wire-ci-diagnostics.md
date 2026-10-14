# R2 daily input CI failure diagnostics — run 38500926595

## Scope and result

This report analyzes only completed GitHub Actions run [38500926595](https://github.com/sihioov/Lagrange/actions/runs/38500926595), attempt 1, for candidate `779b3faa787922032c682b9cf3fe07675a3d976f`. The run ended `completed / failure`; its terminal update was `2026-10-14T02:33:49Z`. Three named jobs and their logs were inspected. The previous CI report and receipt were not modified.

## Proven failure phases

| Job | Earliest failed phase | Evidence and actual exit |
| --- | --- | --- |
| clippy (115647446770) | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Four compile errors listed below plus an unused-mutability warning promoted by `-D warnings`; command step exit **101**. |
| workspace-tests (115647446703) | Owner Equity V2 chart acceptance command `cargo test -p api-server --test http_owner_equity_v2_chart --locked` | Compilation of `market-data` failed with the same four compile errors below; command step exit **101**. The chart test binary did not reach test execution. The later full-workspace command was **skipped**. |
| postgres-integration-validation (115647446742) | DB-gated test phase `api-tenancy-rls` | Validator logged `DB-gated test failed: api-tenancy-rls`; its retained `test-api-tenancy-rls.log` contains the same four `market-data` compile errors below. The enclosing validation step exited **1**. The inner Cargo process numeric exit was not emitted. |

The shared compile diagnostics were:

- E0369 at `crates/market-data/src/daily_price_only/contract.rs:2359:24`: `<` is not implemented for `DailyWireTimestampV1`.
- E0282 at `.../contract.rs:2600:9`, `3050:9`, and `3645:9`: `Option<_>` requires type annotation.

Clippy additionally promoted unused mutability at `.../contract.rs:3988:9` to an error. The workspace and PostgreSQL commands each logged four compiler errors; the clippy invocation also treated the warning as an error. The evidence establishes that the chart compile failure and `api-tenancy-rls` compile failure share the same four defects reported by clippy.

## PostgreSQL phase and guard classification

At the candidate SHA, `.github/workflows/ci.yml:156-158` invokes `bash deploy/db/integration-validation/validate.sh`. That source defines `run_db_test` at lines 1009-1016 and calls `api-tenancy-rls` at line 1318. Its test list runs after the migration and guard stages (lines 1314-1328). The log records earlier DB-gated phases `migration-contract` (36 passed), `auth-audit-readiness` (4 passed), and `auth-router` (1 passed), then identifies `api-tenancy-rls` as the failed phase.

The SQLSTATE messages are not the recorded terminal failure. The sanitized validation evidence reports `conflict_probe=PASS_SQLSTATE_23505` and `0039_down_undelivered_guard=PASS`; source guards for those probes are at `validate.sh:1172-1178` and `1212-1223`. SQLSTATE 23514 appeared with `catalog_preflight` probe messages. The job proceeded to the later `api-tenancy-rls` test phase, whose retained failure log shows the compiler diagnostics. No SQLSTATE is attributed as the cause of the job failure.

After the validation step failed, the sanitized evidence printer, failure-log printer, post-checkout action, and job completion step all succeeded. These later reporting/cleanup steps do not change the validation job’s failure conclusion.

## Provenance and checks

The candidate workflow source was read at the requested SHA (Git blob `8b2d45f792674bfc2e80806762f48276fd92d989`). The candidate validator source was read at the same SHA (Git blob `5b089f5f9e2a0b30ffaf04199e0428c38820f43b`). The private receipt records each original log response’s byte count and SHA-256:

- clippy: 82,465 bytes; `01a068b9975b7a3c2b9954cc33c8cffc463d0da753992c5caaea7212da838291`
- workspace-tests: 103,859 bytes; `6cd376588143063eab7f93f20b56f48ed4281610d3aea5596663f060efe8e5df`
- PostgreSQL validation: 150,206 bytes; `c44aba5bba7d23c3c24e157a07195eba8133abe2d0fdf7943677de69f1cb13ff`

Raw logs were processed in memory only. The sanitized private receipt is [failure-diagnostics.json](../../target/r2-daily-beta/ci/779b3fa/diagnostics/failure-diagnostics.json). Receipt SHA-256: `355c446a89808d08d4c5197a83a098f5e9cacf312b3691edf06c27bdb5f4d74f`.

## Unresolved / not verified

- The PostgreSQL job’s inner Cargo numeric exit code was not emitted; the enclosing validation-step exit code 1 is verified.
- No chart test case or `api-tenancy-rls` individual test result was produced because compilation failed first.
- Later DB-gated test phases after `api-tenancy-rls` were not reached.
- This evidence does not establish new direct-login/GUC/pool acceptance, migration 0060, runtime acceptance, or full I0/C_R2 readiness.
- No further decomposition is needed for this evidence extraction. Coordinator/source-fix ownership remains unchanged.
