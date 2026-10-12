# R1 CASE4 controlled lifecycle fixture repair

- Date: 2026-10-12 KST
- Status: `READY_FOR_REVIEW` — source/static phase complete; focused runtime validation is pending a separately granted slot.
- Repository: `/data/worktrees/3puw275b/orange-dugong`
- HEAD observed: `ac0a08364788e6c2f234cf85025a8d975f362d31`.

## Changes

- `apps/web/tests/e2e/support/stock-beta-fixture.mjs` (lines 4, 15–38, 114–115, 150–174, 199–247, 471–477): added the opt-in `stockBetaLifecycle: "controlled"` fixture mode. List and per-ID reads keep pending memberships unchanged in that mode. The synthetic-only advance path validates the exact three-key body, checks owner/state/generation, computes one next lifecycle on the server, updates only the selected membership, and returns the existing membership-status shape. Normal reset/read-driven advancement remains the default.
- `apps/web/tests/stock-beta-fixture.test.ts` (lines 8, 24, 29, 39–79, 183–373): added direct fixture regressions for read stability and the exact four transitions/coverage/generations; same-instrument re-registration and disabled identity isolation; malformed, extra-key, unknown, stale, out-of-order, terminal, and non-owner rejection without mutation; and controlled-route/reset/default-auto behavior.
- `apps/web/tests/e2e/stock-beta-intraday.spec.ts` (CASE4 changed lines 1349–1521): opted CASE4 into controlled mode. Both initial and replacement admissions still use the real UI POST and production mutation schema checks. The test advances each of four states through the synthetic fixture, holds it for the existing browser GET/RSC refreshes, discards earlier queued observations, and requires the same admitted identity and exact held lifecycle/generation in the typed UI list and real DOM. The POST receipt and four DOM states remain asserted as the five-state sequence. The later disable/re-register/real EventSource/stale-identity/recovery assertions were preserved.
- Added this report with bounded preimage, hash, and formatter evidence under `target/r1-runtime-fxi6ltmv/web-controlled-lifecycle-20261012/`.

The task preimages match all three supplied starting SHA-256 values. A stdlib-only source audit confirmed the CASE4 test is the sole byte-different region in the E2E spec: all bytes before CASE4 and from CASE5 onward match the preimage. All 12 E2E test titles still match, and the other 11 test bodies are byte-identical. The existing synthetic API dispatcher and direct fixture consumers were inventoried with `rg`; no dispatcher or production source change was needed.

## Verification

All commands below ran without Cargo, project tests, browser/server startup, HTTP, Docker, or DB access.

| Command/check | Result |
| --- | --- |
| `node --check tests/e2e/support/stock-beta-fixture.mjs` (from `apps/web`) | Exit 0. |
| `../../node_modules/.bin/biome format --write tests/e2e/support/stock-beta-fixture.mjs` (from `apps/web`) | Exit 0; no source fixes were needed. The formatter output for the two TypeScript files was inspected and only formatting within this task's additions/CASE4 block was applied. |
| `../../node_modules/.bin/biome check --max-diagnostics=none --reporter=json --reporter-file=../../target/r1-runtime-fxi6ltmv/web-controlled-lifecycle-20261012/biome-check-final.json tests/e2e/support/stock-beta-fixture.mjs tests/stock-beta-fixture.test.ts tests/e2e/stock-beta-intraday.spec.ts` (from `apps/web`) | Exit 0; 3 files checked, 0 errors, 1 warning, 47 informational style suggestions. The warning is the pre-existing unused `ensureCurrentUiRows` at `stock-beta-intraday.spec.ts:840`; remaining diagnostics are style suggestions, including unchanged CASE4 tail lines. No new fixture-test diagnostic remained. |
| `git diff --check -- apps/web/tests/e2e/support/stock-beta-fixture.mjs apps/web/tests/stock-beta-fixture.test.ts apps/web/tests/e2e/stock-beta-intraday.spec.ts` | Exit 0. |
| Python 3 standard-library SHA and CASE4 boundary/title audit | Exit 0; verified supplied preimage hashes, current post-edit hashes, exact unchanged prefix/suffix, 11 unchanged E2E test titles/bodies, and 12 total cases. No project imports were used. |
| `rg -n 'stockBetaResponse|resetStockBetaFixture|stockBetaMembershipForIntraday|stockBetaLifecycle' apps/web/tests/stock-beta-fixture.test.ts apps/web/tests/stock-beta-intraday-catalog.test.tsx apps/web/tests/e2e` | Exit 0; confirmed the unit suite, catalog suite, synthetic API dispatcher, intraday fixture reader, and CASE4 as direct fixture consumers. |

Biome does not accept `format --check`; the CLI rejected that option. The supported scoped `biome check` command above performed the final format/lint check. An earlier scoped check identified formatting in this task's additions, which was corrected before the passing check. The 1 warning and 47 informational suggestions remain untouched because they are outside the accepted repair contract.

## Source hashes

| File | Supplied/preimage SHA-256 | Final SHA-256 |
| --- | --- | --- |
| `apps/web/tests/e2e/support/stock-beta-fixture.mjs` | `21d0855452b9a180eeca088dfc21e48c3b88333021a5064f523b1c0c21b560e4` | `0aee2c8fd497ae1e041ce65b65a6114e2e5197b07c9d98e3c5e1a11f182a6822` |
| `apps/web/tests/stock-beta-fixture.test.ts` | `27f3cbf39251c9cca4fa835b4a8cbf80b20051dc1641c104a063a0fa56e8a245` | `5697fbd3b0029117416f9b49e385783f138e89ab9d2bcc36bc8bf746f42979c1` |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | `f68a8acb9282e6f1ec922cefdc4b9b24c66c1788a32278b0271c512bc05540fe` | `8eeb373cdddbec4e0b1bce1086cc522f83257a274409dba82afd7799127776c4` |

The three preimages and `hashes-before.sha256`, `hashes-after.sha256`, `biome-check-final.json`, and formatter outputs are retained in the task-local evidence directory noted above. No task-owned changes were made outside the three source files and this report.

## Deviations and follow-up

- Implementation deviations: none. No dispatcher, production route, other testcase, retry/skip/title, or fixture infrastructure was changed.
- Verification still pending: execute the existing fixture unit tests and the focused Web full-smoke/browser selection after the exclusive validation slot is granted. This source/static assignment did not run those checks or establish runtime behavior.
- Unresolved specification or caller conflicts: none found. Further decomposition is not indicated.
- Not found: additional direct fixture consumers beyond the inventory above; any need to change production or dispatcher code.
- Not verified: TypeScript type-checking, unit-test execution, browser/Next runtime behavior, and the full smoke suite; all were explicitly prohibited in this phase.
