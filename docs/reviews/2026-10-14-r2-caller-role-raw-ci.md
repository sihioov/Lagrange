# R2 caller/role/Raw hosted CI evidence

Run [38536460035](https://github.com/sihioov/Lagrange/actions/runs/38536460035), attempt 1, checked out candidate `c0ac599cb1ede900e155f724687c57410b4bd0d9` on `excited-dolphin` via `workflow_dispatch`. It completed successfully at `2026-10-14T08:59:29Z`; the required aggregate also succeeded. The exact candidate appeared in each of the seven execution-job logs. The final inventory hash matches the launch receipt, and all 26 inventory source hashes match the pinned candidate blobs.

| Job | Result |
| --- | --- |
| format | success |
| clippy | success; no primary compiler error headers |
| policy | success |
| Python tests | success |
| web | success |
| PostgreSQL integration validation | success |
| workspace tests | success |
| required aggregate | success |

The receipt contains full step states and the 49-name matrix: **49/49 direct Linux Rust result rows passed** (30 wire, 18 Raw, and the qualified collector worker regression). The non-Unix Raw test is `EXCLUDED_LINUX`. Six chart tests each have a separate direct passing row. The workspace QA DB Compose teardown step succeeded. Cargo aggregate result-row sums are reported by invocation and explicitly are not unique-test counts. Seven `v3_historical_input` rows and 87 history-named direct rows were observed passing; V2 namespace row evidence is recorded without claiming a complete historical-V2 acceptance subset.

Policy logs report `PG_VALIDATION_SELF_TEST`, `MIGRATE_STATIC`, `SECRETS_RUNTIME_STATIC`, `OPS_STATIC`, and `PRODUCTION_OPS_STATIC` as PASS. Foundation validation, DB integration static check, Compose candidate/backtest/code-commit static checks, migration static check, nginx/systemd static checks, and ops/image-build static checks all have successful step outcomes. The pinned migration checker declares bootstrap count 8; the log does not separately print the measured count. No explicit fakeroot or numeric-fixture skip notice appeared in the captured policy log. Wrapper success does not establish individual physical-LF, literal-parsing, or shared credential-gate self-test subcase outcomes.

PostgreSQL reports `APPROVED`, candidate `code_commit`, two 59-migration snapshots at latest version 59 with identical 59-entry checksum lists, rerun `NO_OP`, 0039 down-undelivered guard PASS, enforced skip markers, and successful cleanup. Both direct-role login lists include `daily_input_admitter`. The safe summary does not separately identify per-role catalog evidence, role GUC, or bootstrap credential-gate outcome; credential fields were excluded.

Python reported 485 passed, 10 skipped, and 7 warnings. The three readiness mocks and SIGTERM/account-lock test have no named node result rows in the quiet log, so their individual outcomes remain UNKNOWN. Web reports OpenAPI check PASS, 61 test files / 487 tests passed, and E2E `passed`, exit 0, complete coverage.

Unverified: hidden-skip absence; nested self-test subcases; individual quiet pytest node outcomes; role GUC and app.actor_id; max-two-connection helper; bootstrap credential-gate result; migration0060; full snapshot/runtime/production/Owner acceptance and full C_R2. No local tests or operational actions were performed. No scope deviation; the in-progress log endpoint was unavailable once, then final logs were retrieved after terminal completion.

Structured evidence, source hashes, test names/statuses, step outcomes, and log response byte counts/SHA-256 values are in [ci-evidence.json](../../target/r2-daily-beta/ci/c0ac599/ci-evidence.json). Raw logs were processed in memory only.
