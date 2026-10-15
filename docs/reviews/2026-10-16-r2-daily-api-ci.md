# R2 Wave63 I5 daily API candidate CI recovery

- Roadmap: R2-04/09; package WP-4/I5. Source implementation is not accepted.
- Candidate: `68c6f648765dc9dc28c4818f92a1dbbd8f91c8c4`.
- [Run 38810733003 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38810733003):
  **FAILURE**, workflow_dispatch, CI/ci.yml, excited-dolphin.
- All eight terminal jobs and full log bodies were recovered in memory. All seven
  checkout SHAs matched. All 683 frozen pins and 371 committed blobs matched.
- Later documentation HEAD does not change the tested candidate. No rerun,
  cancellation or duplicate dispatch occurred.

| Job | Outcome | Evidence boundary |
| --- | --- | --- |
| format | PASS | Workspace cargo fmt check |
| policy | PASS | Exact hosted job |
| python-tests | PASS | 487 passed, 10 existing skipped, 7 warnings |
| workspace-tests | FAIL | API E0603 during chart compilation; workspace step skipped |
| clippy | FAIL | E0603 and empty_line_after_doc_comments under -D warnings |
| postgres-integration-validation | FAIL | api-tenancy-rls compilation exit101; cleanup passed |
| web | FAIL | OpenAPI check passed; lint format error; subsequent steps skipped |
| required | FAIL | Correctly retained failing execution jobs |

## Concrete correction disposition

1. **I5-C01a:** `crates/api-server/src/repos/owner_beta.rs:759` refers to
   private `daily_price_only::contract`. The same wire type is already publicly
   re-exported from `daily_price_only`; use that path without widening visibility.
2. **I5-C01b:** `crates/api-server/src/runtime.rs:554–558` placed the pre-existing
   price-input comment before the new daily-mode function, with a blank line.
   Restore the comment to its intended price-input function and describe daily mode.
3. **I5-C01c:** `apps/web/tests/r2-daily-api-contracts.test.ts:185–187,212`
   differs from Biome's assertion wrapping and final blank-line formatting.
   Apply only those layouts. The lint summary has one error, six warnings and
   62 infos; this does not claim unrelated warnings/infos were repaired.

The two diagnostic logs were re-read and their complete hashes matched the original
receipts. Only fixed diagnostics and source locations were persisted.

## Execution coverage

All 25 I5 Rust definitions and all carried workspace groups are **NOT_EXECUTED**:
the workspace Cargo step was skipped. Six chart cases also did not execute because
their API dependency failed compilation. Six new Web contract cases and subsequent
typecheck/build/E2E did not execute after the lint failure. Aggregate counts are not
used as unique named test evidence.

PG's first three targets (migration-contract, auth-audit-readiness, auth-router)
returned exit0/noSKIP. api-tenancy-rls returned101; the following 14 targets,
including all 15 required daily-API names, did not execute. Earlier PG daily32,
subcase73, bootstrap83, 62 migration checksum sets twice, no-op rerun, direct roles,
0039 guard, catalog probes and container cleanup passed. Unprinted nested helper
rows/inner exits remain UNKNOWN. The 18-target PG gate did not pass.

Private receipts: `target/r2-daily-beta/ci/68c6f64/ci-evidence.json`
(5393 lines, SHA256 `1c133b8bb1fb80caec3b7083f1de5e8e572b8275d2a315fd8db395b99b1a2df7`)
and `diagnostic-supplement.json`. Raw logs were not persisted.
The prior Wave62 success and all earlier failures remain preserved.

Next is the three-file I5-C01 correction and one new exact-candidate hosted run
after source review, successor freeze and all-event existing-run query. No local
project execution, provider/production action or delegation is authorized here.
I7 UI, I6 installer/diagrams, actual approved-base three-UID runtime, integrated
independent review, G4, Owner use, three trading days and final C_R2 remain open.
