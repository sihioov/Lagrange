# R1 Web latest-delta independent source review — 2026-10-12

**Findings: none. Source review passes within the accepted BODY1/BODY2/CLOCK1/CLOCK2 scope.** This is not browser acceptance, proof that the clock/loading hypotheses are resolved, or overall R1 approval. No implementation or runtime command was executed by this reviewer.

## Identity and inspected scope

Repository `/data/worktrees/3puw275b/orange-dugong`, HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate preserved. Review comparison is the retained preimage versus the latest spec, not HEAD's historical Web rewrite.

| Evidence | SHA-256 |
| --- | --- |
| target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i/before.spec.ts | `6c071b4b903952fc37f88f21e0aca2bf6c999e329b4506d27fab5c3a6303faec` |
| apps/web/tests/e2e/stock-beta-intraday.spec.ts, start/end | `f68a8acb9282e6f1ec922cefdc4b9b24c66c1788a32278b0271c512bc05540fe` |
| docs/reviews/2026-10-12-r1-web-followup-repair.md | `ce56eddc44ee85c6b8ad01702c74170caa86a7327f5e6358220b1b52334ea833` |
| docs/reviews/2026-10-12-r1-web-browser-followup.md | `a36e79802ac0de496524f866b3aa8147065f3e9c4601b9d0c99f29cd490c9245` |

All **23 latest-delta hunks** were read, including every new helper/call site and every replaced assertion. Current source spans: imports 8–15; helpers 672–739; case3 1295–1307; case4 1362–1490,1548–1557,1578–1590,1615–1627; case6 2010–2047,2081–2098,2140–2147,2157–2164,2174–2181; case7 2344,2387–2399; case8 2522–2532,2561–2571. Surrounding admission, frame/reset, scope, terminal and cleanup logic was inspected where needed; unchanged historical bodies were not reopened.

Independent stdlib text comparison confirms 12 identical titles and **byte-identical** complete cases 1,2,5,9,10,11,12. No .skip/.fixme/.only appears in either version. Existing assertProviderFree/assertNoFallback/cleanupFixture call counts remain 6/16/17; clock.install/pauseAt remain one each; waitForTimeout remains absent. The normalized reviewer diff (headers `before.spec.ts` / `stock-beta-intraday.spec.ts`) SHA-256 is `087d3fe7e1f4fc028bb5a4b73da7fcfa3d34ca977b82d1bba9a65ba133580204`.

Worker evidence directory: `target/r1-runtime-fxi6ltmv/web-followup-repair-rmtemx9i`. Its source.diff body matches the independent preimage delta exactly after the two filename headers. These artifacts were read/hashed, not executed:

| Artifact | SHA-256 |
| --- | --- |
| source.diff | `23ce4cb8252e2901dcd6e40ccc8ccf97a4e418515abfbbed67658a09ac3293b7` |
| static-audit.json | `a823470f2e08330ba58f7a58f21732b2dabe7a53188de037254038dc8c85b9b1` |
| static-audit.log | `69a03c7fd48665cd52f620ee5b39ea9c26866ce1261e9e186ba6d571c1828ee6` |
| source-hashes.json | `75297706415221b62927e921d8cbac6b22408b8169e87f283026764ae148e763` |
| audit.cjs | `2d4cb13c4078f1edda78da4d32415a11c3b46079ff39b0ca6101f350564cbcbe` |
| biome-check.log | `e9800e4487cdcaf57a6ad16441aefb58057377bee2c17c1f1b777a1c3dd16029` |

The worker's AST assertion counts (414 original,401 retained,13 mapped replacements,37 additional) are retained static evidence. Review independently checked the replacement logic through the complete diff; it did not rerun the author's parser/audit/formatter/typechecker.

## Contract and race assessment

**BODY1:** case7:2344 now requires exactly `+1,300.00 (+1.30%)`. Current formatter at `components/stock-beta/quote/current-quote-view.tsx:20–31` preserves the supplied fraction and sign. Only the signed-text literal changed; exact quote, receipt/epoch/version/replay/rollback and later recovery assertions remain.

**BODY2:** helper:1362–1469 registers same-origin, exact query-free membership POST before the real Add click, requires202, parses the production mutation schema, and checks REQUESTED/005930.KRX/generation0/nonduplicate. Its two callers at1479/1548 bind both later READY identities to their respective POST UUIDs; replacement must differ from the initial identity and remain generation1. Existing disable, stale membership/lease404, snapshot proof and final counters remain.

The GET/ordinal technique is source-supported for this bounded single-admission scenario. `stock-beta-workspace.tsx:254–258` assigns the typed membership list directly; 582–597 passes that same array in the dashboard view model; dashboard stock-beta-dashboard.tsx:117–126 passes the model unchanged to its widget; membership-status-widget.tsx:140–143 maps in the same order and keys by membership.id. The card:54–64 exposes lifecycle/instrument/generation but no UUID. Therefore nth(index) is an externally observable correlation with the actual UI response, not a fabricated DOM ID or React/fixture bypass.

The helper also checks list/card cardinality, exact POST identity in the response, unique active matching instrument, instrument text, lifecycle and stage-appropriate generation. Replacement's old DISABLED card stays distinguishable from the new active card. Fixture `stock-beta-fixture.mjs:68–72,96–113,411–441` and contracts:525–543 establish pending0→READY1. No new membership GET, mutation, DOM attribute or fixture override was added. REQUESTED is honestly POST evidence; the four later stages are actual DOM assertions, combined into the unchanged five-state contract.

Stale/out-of-order response risk was checked explicitly: the listener starts before mutation, so an earlier list can be queued. A list lacking the admitted ID is tolerated only when its active universe is empty; unrelated active IDs fail. Pending polling in workspace:408–440 awaits each refresh before scheduling the next; observing the DOM before the next virtual step prevents this test from intentionally running past unseen intermediate states. However neither response arrival nor response.json alone proves React applied that version. The subsequent DOM assertions provide that evidence; if a superseded response can never render, they fail within the existing expect bound rather than falsely mark that stage as covered. Ordinary serialized polling supports the implementation; slow/reordered client or router-refresh application remains a **browser hypothesis**, not a reproduced source defect or a generalized UUID-in-DOM proof. Do not resolve a future failure by dropping queued stages or relaxing identity assertions.

Listener cleanup at1467–1469 is in finally, including receipt/JSON/DOM failure. The helper emits only lifecycle/generation/stage sequence and an identity-match boolean, not UUID/body/auth. The 90s virtual observation structure and250ms steps remain bounded as in the preimage.

**CLOCK1:** helper:706–739 checks exact previous connection/get counts, runs delay−1, checks no early source, then1ms, then **at most one conditional250ms** opportunity. It retains the10000ms real poll bound and asserts exact final ledger/get counts plus unchanged accepted create/release/renewal counters. There is no loop, timer reinstall, backward reset, fastForward, real sleep, deadline shortening or timeout increase. Existing runFor/runForPages:193–225 still advances all participating contexts and the fixture together.

The controller's reconnect:394–401 establishes the deadline synchronously before emitting Connecting; drive:249–256 checks it on250ms cadence. Case6 now observes Connecting before each paused1/2/4s step, replacing the old pre-bound16ms pumps rather than adding16ms to the backoff. Malformed-delivery cases wait for the existing atomic purge assertions before entering the helper. No virtual advancement was introduced by finite phase capture itself. This supports the intended phase alignment, with runtime timing still pending.

All ten call sites were checked against their cumulative source targets:

| Current line | Case/phase | Delay / expected cumulative connections |
| --- | --- | --- |
| 1307 | 3 wrong generation | 1000 /2 |
| 1590 | 4 stale membership | 1000 /3 |
| 1627 | 4 wrong generation | 2000 /4 |
| 2088 | 6 lifecycle reconnect | 1000 /5 |
| 2144 | 6 retry2 | 1000 /2 |
| 2161 | 6 retry3 | 2000 /3 |
| 2178 | 6 retry4 | 4000 /4 |
| 2399 | 7 rollback | 1000 /2 |
| 2532 | 8 zero price | 1000 /2 |
| 2571 | 8 NaN price | 2000 /3 |

Fixture stream-fixture.ts:245–260 records each accepted source once in both ledger and getOpens; close/disconnect:745–750 and connectionLedger:774–783 retain cumulative identities. The helper's two count checks agree with that model. All replacement-source identity changes, reset/full-snapshot requirements, no-REST guards and terminal fourth-error16ms/60s-no-more-source checks remain. Finite diagnostics at672–703 contain only clocks/counters and literal Loading/board counts. No payload/debug body or request authorization is attached.

**CLOCK2:** case6:2010–2047 retains the real Primary Link, observes exact local GET `/stock-beta` with the installed RSC header before click, requires successful response and finished()==null, and bounds the entire step and response wait to5000ms. Then it checks URL, pumps the existing two16ms frames once, and retains board1 plus fresh third lease/scope assertions. It does not substitute goto/reload, clear router cache, reset clocks or create another fetch. response.finished is network completion; board1 remains the separate application-commit assertion.

Cached-route assumptions were checked against shipped Next guides/source, not generic memory. Actual page.tsx:24–26 is force-dynamic/force-no-store/revalidate0; next.config.ts does not enable cacheComponents or staleTimes; Primary navigation:62 uses the ordinary Link, not history back/forward or prefetch=true. Installed staleTimes guide:26–35 specifies dynamic page caching0 and separately reusable layouts/loading states. Installed glossary:47–51 distinguishes page reuse on back/forward. Installed links.js:218–220 disables viewport prefetch in development; app-dir/link.js:351 also excludes development hover prefetch. Thus cached shared shell/loading content does not itself disprove the return page RSC observation in this current dev lane. Installed fetch-server-response.js:85–90 supplies rsc:1. No source-backed cached-route defect was found in this delta; a later response-less/cached navigation must be diagnosed without manufacturing an RSC GET.

## Supporting source identity and limits

Current supporting hashes match the worker's pinned source-hashes.json for workspace `fc3d2d14…ebbd6`, membership widget `28426d4f…ffb3`, stream controller `f6ad2f91…bf2b`, production membership contracts `685a0558…a0ca`, controlled stream fixture `ecf788b3…26fa`. Additional read fixtures/formatter hash to stock-beta-fixture.mjs `21d0855452b9a180eeca088dfc21e48c3b88333021a5064f523b1c0c21b560e4` and current-quote-view.tsx `4f117966d4de5731b483890ef0f97050926e722ffbe020c805b98d55b3480ebc`. Full worker hashes are retained in the pinned JSON rather than interpreted as runtime passes.

Actual reviewer checks: git rev-parse; SHA-256 start/end snapshots and worker artifacts; complete preimage diff; stdlib title/body/hunk comparison; targeted source/contract/fixture and bundled Next guide reads; scoped git diff --check (exit0). Ordinary diff exited1 because the expected delta exists. Guessed optional paths were resolved by source-file discovery. No project code, worker audit, test, typecheck, build, Playwright collection/inventory, browser, server, port, Cargo, Docker, provider or DB operation ran. ENV1/root environment and fail-fast policy were excluded as instructed.

Coverage source contract remains all12 titles and the adopted134 unique synthetic/135 planned execution selections; this review did not collect or execute them. Source edits in this package cannot establish135 actual attempts, especially while the accepted fail-fast boundary remains. Historical52 failures and prior trace results are not fresh proof of these bytes.

Unresolved source decision: **none**. Not verified: genuine browser receipt/list/DOM ordering for both admissions; all ten no-early/tick boundaries; cold/slow and cached-shell navigation completion/paint; later recovery/reset/lease/proof/counter assertions; typecheck and complete candidate browser executions. Missing required preimage/source/evidence: **none**. Intentionally unread/out of review scope: prior historical rewrite beyond necessary context, ENV1 implementation, unrelated worker changes and runtime artifacts/processes. No claimed inspection gap within the23 changed hunks.

Changed file: this new review report only, lines 1–82. Deviations: **none**. Further decomposition: **none for source review**; already-planned reserved browser validation remains necessary. No runtime resource was created, no cleanup/signal was performed and no owned process remains. **Actual idle.**
