# R1 WP-4 bounded Web source follow-up

Date: 2026-10-12. Status: **SOURCE_READY_FOR_REVIEW; actual idle**.
Target: shared dirty `orange-dugong`, HEAD
`ac0a08364788e6c2f234cf85025a8d975f362d31`.
R1-05 / R1-08 Web recovery source evidence; browser acceptance remains pending.
No heavy slot was used. No runtime process, listener or browser was launched.

## Scope and source identity

Only the intraday spec and this new report were written as tracked paths.
Private evidence is in
`target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/` (directory0700,
files0600). Prior reports and shared changes were preserved.
Coordinator owns ENV1 and accounting; this worker did not edit the supervisor,
Python regressions, fixtures, product sources, other Web specs or roadmap.

| Path | Changed current line spans / purpose |
| --- | --- |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | 8–15 imports; 672–739 finite clock receipts and boundary helper; 1295,1307 case3; 1362–1490,1548–1557 case4 admission observations; 1578,1590,1615,1627 case4 recovery; 2010–2047 case6 Link return; 2081–2098,2140–2147,2157–2164,2174–2181 case6 reconnect; 2344 signed literal; 2387,2399 case7 recovery; 2522,2532,2561,2571 case8 recovery. Exact contextual hunks retained in `source.diff`. |
| `docs/reviews/2026-10-12-r1-web-followup-repair.md` | New report, entire file. |

Spec input SHA256 (verified before edits):
`6c071b4b903952fc37f88f21e0aca2bf6c999e329b4506d27fab5c3a6303faec`.
Final SHA256:
`f68a8acb9282e6f1ec922cefdc4b9b24c66c1788a32278b0271c512bc05540fe`.
The exact accepted preimage is retained as `before.spec.ts` with the input hash.

## BODY1 — formatter precision

Case7's existing signed-text assertion is now exactly
`+1,300.00 (+1.30%)` (2344). Positive sign, percentage, exact quote value,
R2 receipt/epoch, duplicate rejection, rollback purge and recovery remain.
No formatter or product behavior changed.

## BODY2 — actual admission receipt and rendered identity

Both real Add actions call the same case-local helper with separate
`initial` / `replacement` labels. Before clicking, it registers the exact
same-origin, query-free `membershipPath` POST response observation, requires202,
parses production `ownerEquityV2MutationSchema`, and asserts REQUESTED,
005930.KRX, generation0 and no duplicate active admission. It creates no request.
Both later READY identities must equal their respective actual POST IDs;
the replacement must have a different UUID and retain generation1.

**Source-bound identity decision:** the actual card has no membership-ID DOM
attribute (`membership-status-widget.tsx:54–64`); it exposes lifecycle,
instrument and generation. `stock-beta-workspace.tsx:254–258` assigns the typed
UI list directly; the widget renders that same order and keys rows by ID
(`membership-status-widget.tsx:140–143`). The helper therefore passively records
only the app's existing exact local membership GET responses, parses the
production list schema, finds the exact POST ID in each response, and binds its
ordinal to the rendered card. It asserts list/card count, unique active
instrument, exact instrument, lifecycle and generation. There is no added GET,
ID inferred from instrument alone, DOM mutation, React access/mock, or new app
attribute. An earlier response lacking the new ID is accepted only if its active
universe is empty; any unrelated active identity fails. Listener removal is in
`finally`, including failure paths.

Generation is deliberately0 in REQUESTED and the pending rendered stages, then1
in READY: production schema comment at `equity-signals-contracts.ts:537–543` and
fixture `stock-beta-fixture.mjs:68–72,96–113` establish this transition. It would
be incorrect to require the receipt's generation0 unchanged at READY. The same
ID is retained throughout, with exact stage-appropriate generation assertions.

DOM evidence must be exactly VALIDATING,BACKFILLING,MATERIALIZING,READY; actual
POST REQUESTED plus those four rendered states must equal the unchanged full
five-state list. Receipts label POST and DOM separately and persist only lifecycle,
generation, state sequence and an identity-match boolean, no UUID/body/auth.
The existing90s virtual observation limit and250ms synchronized steps remain.
Disable, stale-ID/lease404 probes, READY snapshot proof and final counters remain.

A clarification about ID exposure was offered while inspecting the source.
The implemented choice is the source-proved existing-response/ordinal binding
above, within the accepted test-only scope; direct ID-attribute selection is
unavailable. This binding still needs browser verification and coordinator review.

## CLOCK1 — finite phase alignment hypothesis

`attachClockPhase` records fixture time, browser Date/performance, cumulative
connection/open and accepted create/release/renewal counters, Loading and board
counts. It asserts the existing Date/fixture binding. No payload, authentication,
UUID, arbitrary DOM text, environment or external request is attached.

Every selected phase records before delivery/disconnect, after observed purge or
error handling, immediately before the full bound, at the bound, and source-ready.
The helper advances `delay - 1`, asserts no new source, then advances1ms. Only if
the exact expected source is still absent does it advance one250ms scheduler
opportunity, recording that conditional step. Final connection/open counts must
be exact, with unchanged accepted create/release/renewal counters and the existing
10000ms real poll bound. There is no repeated virtual retry, real sleep,
fastForward, shortened backoff or increased timeout. Existing synchronized
`runFor`/`runForPages` and all clock installation functions are token-unchanged.

| Case / current helper call line | Phase | Full first bound ms | Exact cumulative sources |
| --- | --- | ---: | ---: |
| 3 / 1307 | Wrong-generation malformed delta | 1000 | 2 |
| 4 / 1590 | Disabled old membership delta | 1000 | 3 |
| 4 / 1627 | Replacement wrong-generation delta | 2000 | 4 |
| 6 / 2088 | Lifecycle disconnect after fourth scope | 1000 | 5 |
| 6 / 2144 | First exhausted-retry subphase | 1000 | 2 |
| 6 / 2161 | Second exhausted-retry subphase | 2000 | 3 |
| 6 / 2178 | Third exhausted-retry subphase | 4000 | 4 |
| 7 / 2399 | Version rollback malformed delta | 1000 | 2 |
| 8 / 2532 | Zero-price malformed delta | 1000 | 2 |
| 8 / 2571 | NaN-price malformed delta | 2000 | 3 |

Case6's existing999+1 no-early check moves into the shared equivalent boundary
assertion. Its three pre-backoff16ms pumps are replaced by observing the real
Connecting status before clock advancement; this makes the full1/2/4s bounds
start after actual error handling, rather than16ms later. The terminal fourth
error's16ms pump and60s no-more-source checks are unchanged. All reset/full
snapshots, source identity changes, stale delivery, epochs, lease and final
counter assertions remain. The controller's250ms cadence/deadline evidence is
`intraday-stream-controller.ts:249–256,394–401`, read-only.

This is a bounded scheduling correction and diagnostic hypothesis. No runtime
cause is claimed resolved. A later failure must remain nonzero and require
reviewed diagnostics rather than more clock steps.

## CLOCK2 — real Link completion and two frames

Case6 retains its real Stock signal beta Link. The observation is registered
before click and requires exact local origin, `/stock-beta`, GET and the existing
`rsc: 1` request header. Installed Next source supplies that header
(`fetch-server-response.js:85–90`; `app-router-headers.js:95`), so the predicate
uses no invented request flag. Installed linking/navigation guide was read.
Response observation and actual click are awaited together; successful status and
`response.finished() === null` are required. The whole step and response wait
are bounded5000ms, the current configured expect timeout
(`playwright.config.ts:6–8`), with no timeout extension. No response body is read,
persisted or fetched again for this navigation.

It asserts the returned stock-beta URL, records finite before-click and
response-complete receipts, runs existing `pumpAnimationFrames` once (16+16ms,
all participating paused contexts), records the after-frames receipt, then
requires board1 and the existing fresh third scope. No goto/reload replacement,
clock reinstall/reset, product navigation or controller change was introduced.

## Static commands and outcomes

All commands were local; no test bodies were executed. Package commands used
cwd `/data/worktrees/3puw275b/orange-dugong/apps/web`.

- `NO_COLOR=1 ../../node_modules/.bin/biome format --write tests/e2e/stock-beta-intraday.spec.ts`
  exit0 on all four actual invocations; initial/intermediate formatting fixed the
  one allowed file; final invocation: one file,25ms, no fixes.
- Final `NO_COLOR=1 ../../node_modules/.bin/biome check --max-diagnostics=100 tests/e2e/stock-beta-intraday.spec.ts > /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/biome-check.log 2>&1`
  exit0, one file140ms; one pre-existing unused `ensureCurrentUiRows` warning,
 48 style/template infos. No suppressions or unsafe fixes. Earlier checks also
  exited0; final retained log is authoritative for final bytes.
- `node target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/audit.cjs > target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/static-audit.log 2>&1`
  final exit0: TypeScript createSourceFile/scanner only,0 parser diagnostics,
 12 identical titles, seven token-unchanged case bodies (1,2,5,9,10,11,12),
  all existing helpers unchanged except the explicitly replaced case4 helper,
  exactly ten listed reconnect callsites. No compiler/typechecker/Playwright import
  or collection.414 original terminal assertion calls:401 token-retained,
 13 explicitly mapped replacements,37 additional source assertion calls. The
  replacements are six malformed reconnect counters, four case6 reconnect counters,
  one case6 no-early assertion, lifecycle receipt/DOM authority, and decimal text.
  These are source counts, not successful test counts.
- `git diff --check -- apps/web/tests/e2e/stock-beta-intraday.spec.ts`
  exit0, final source whitespace check. Final report is included in a subsequent
  scoped diff check.
- Inline `python3 -B` SHA256/title/preimage/hunk/evidence-mode audits exited0.
  `git rev-parse HEAD` exited0 and matched the target. Read-only cat/sed/rg
  covered app/root instructions, current R1 roadmap, accepted BODY/CLOCK findings,
  actual widget/workspace/schema/fixture/controller/config and installed Next docs.

**Preserved tooling failures:** one inline Python edit used a root-relative path
from apps/web and failed with FileNotFoundError (exit1); the following formatter
still ran and the enclosing shell exited0. No edit occurred; the path was corrected
in a separate successful root-cwd command. One guarded edit failed an indentation
match assertion (exit1) before writing; inspected formatted source and applied
one successful exact-match correction. Initial AST audit incorrectly required
case4's intentionally replaced helper to remain unchanged (exit1); its explicit
replacement was recorded, all other unchanged-helper checks retained, and final
full audit passed. No repeated implementation failure was hidden. Read-only
searches for guessed controller/workspace directories returned missing-path exit2;
actual files were located/read, and no required source remains missing.

## Retained private evidence

| Artifact in the private directory | SHA256 |
| --- | --- |
| `biome-check.log` | `e9800e4487cdcaf57a6ad16441aefb58057377bee2c17c1f1b777a1c3dd16029` |
| `static-audit.json` | `a823470f2e08330ba58f7a58f21732b2dabe7a53188de037254038dc8c85b9b1` |
| `static-audit.log` | `69a03c7fd48665cd52f620ee5b39ea9c26866ce1261e9e186ba6d571c1828ee6` |
| `audit.cjs` | `2d4cb13c4078f1edda78da4d32415a11c3b46079ff39b0ca6101f350564cbcbe` |
| `source.diff` | `23ce4cb8252e2901dcd6e40ccc8ccf97a4e418515abfbbed67658a09ac3293b7` |
| `source-hashes.json` | `75297706415221b62927e921d8cbac6b22408b8169e87f283026764ae148e763` |

## Deviations, unresolved and unverified

- Semantic/specification deviations: 없음 (none). Source-bound ID correlation
  and pending0→READY1 generation are explicitly explained above, not fabricated
  ID DOM attributes or immutable generation0 claims.
- Unresolved implementation boundaries requiring expanded scope: 없음 (none).
  No further decomposition is needed for this source package.
- Pending: coordinator review of these source decisions; typecheck and genuine
  browser validation after a separately granted Web slot and ENV1 integration.
  Both admissions' exact receipt/DOM correlation, slow/cold navigation completion,
  one-time two-frame paint, all ten1/2/4s no-early checks and conditional250ms
  source readiness, later reset/snapshot/proof/count subphases remain unverified.
- Current runtime tests/executions performed by this assignment:0. No new pass,
  full collection, CI success or resolved-clock-cause claim. Prior failures remain.
  All12 declarations remain; this change does not alter the adopted134 identities
  or121+13+1 selection contract. Existing fail-fast is preserved;135 actual
  executions including both owner modes remain the final gate, not a static claim.
- Missing/unread required source/artifacts: 없음 (none). Direct membership-ID DOM
  attribute: absent, handled through the actual list renderer as explained.
- Owned runtime resources/cleanup actions: 없음 (none); no process/listener/pidfd,
  browser, port probe or service operation was created. No foreign listener contact.
- Staging/commit/dependency/lock/provider/env/product changes: 없음 (none).

## Final administrative check

Scoped `git diff --check -- apps/web/tests/e2e/stock-beta-intraday.spec.ts docs/reviews/2026-10-12-r1-web-followup-repair.md` exited0.
The prescribed AGENTS memory finish assessment used the exact current hook
envelope with `no_durable_change`: default sandbox invocation returned
STATE_ERROR/exit1 (operational database inaccessible); the identical narrowly
approved native invocation exited0, SKIPPED/savedfalse. No durable memory was
stored; this administrative receipt does not change runtime verification.
Final inline SHA256/mode audit exited0: source hash unchanged, private directory0700 and files0600. `git check-ignore -- target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/static-audit.json` exited0; private evidence is ignored. Final scoped source/report diff check again exited0.
