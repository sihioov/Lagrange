# R1 — two remaining Web failures, read-only diagnosis

2026-10-12 · **DIAGNOSTIC_RERUN_NEEDED; actual idle**.
HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.
Case4 has a demonstrated fixture/read-order defect; case6 needs a finite timer diagnostic.
No source implementation, runtime/module execution, network or resource probe occurred.
**Evidence and identity.** `R = target/r1-web-e2e-1791807229487834819-653045ecbfe2/`;
`D = target/r1-runtime-fxi6ltmv/web-final-diagnostic-d8kawyzy/` (0700; files0600).
The exact run agrees with `web-validation.md:175–188`. Results are JSON lists:
off121 passed; WS11 passed/2 failed; each has one attempt/retry0. Actual134 is
121+13. Existing fail-fast at runner1779–1780 stops before separate WS owner-empty1.
This is **not a new selection defect**. Preserve fail-fast and the final135 gate.

| Verified frozen input | SHA256 |
| --- | --- |
| Intraday spec | `f68a8acb9282e6f1ec922cefdc4b9b24c66c1788a32278b0271c512bc05540fe` |
| Runner | `c8c7e32631fae2d392c19ea059e34314d46d10fbda517738abce92398b23afb4` |
| Runner regressions | `592039b4047f0ef5e39d6ea76826786619f16833619868cc02a0d92dd537633f` |
| Earlier `web-followup-repair-rmtemx9i/before.spec.ts` | `6c071b4b903952fc37f88f21e0aca2bf6c999e329b4506d27fab5c3a6303faec` |

Artifacts below are under `R/market-stream/market-stream-results-playwright-artifacts/`.
Case4 directory suffix is `stock-beta-intraday-provid-556d1-trument-with-a-new-identity-chromium/`;
case6 is `stock-beta-intraday-provid-d5aa2-econnect-and-latches-logout-chromium/`.

| Artifact | SHA256 |
| --- | --- |
| Case4 trace.zip | `d8b6c5b3577f6099982fa79203a415ae48bf05f3ae320b5a9bd2d1cb70085b6d` |
| Case4 error-context.md | `5f8acbeb08226baa7c302c8ba85eacd73073a05777be895439f90a44855b1272` |
| Case6 trace.zip | `3861deddc39d3f7379150ef0194e984480ad347d940eb3cb674095ffd1b60dc0` |
| Case6 error-context.md | `674895673e10160aedc55f940ee426be8be0a76e10f558f06a34aecbe91b0824` |

**Case4 — observed sequence and cause.** Title: “adds 005930 to READY, disables it,
and re-registers the same instrument with a new identity”; test ID
`d6c1cbb812c314790e36-41fe09a7c5631fe102aa`, declaration1344, failure1436.
Times below are trace monotonic milliseconds, not browser virtual time. M1/M2
are anonymized distinct membership IDs from the two actual POST receipts.

| Trace time | Actual event |
| ---: | --- |
| 26581.139 | First POST202: M1 REQUESTED/gen0. Four browser GETs and DOM checks complete VALIDATING→BACKFILLING→MATERIALIZING→READY/gen1; retained initial-admission receipt confirms all four. |
| 27362.821 | Disable POST202: M1 DISABLED/gen1; browser membership refresh agrees. |
| 27415.377 | Last test-side membership GET: M1 DISABLED. No APIRequestContext membership read occurs after the second POST. |
| 27526.198 | Second POST202: different M2 REQUESTED/gen0, same005930.KRX. |
| 27533.914 | Browser GET returns M1 DISABLED + M2 VALIDATING/gen0; DOM VALIDATING check succeeds at27575.796–27579.328. |
| 27587.676 | Browser signals/latest GET returns404, before the RSC refresh. |
| 27600.607 | RSC `/stock-beta` GET starts; its completed payload contains M1 DISABLED + M2 MATERIALIZING/gen0. Duration44.016ms. |
| 27603.176 | Concurrent browser membership GET starts; payload contains M1 DISABLED + M2 BACKFILLING/gen0. Duration35.836ms. |
| 27705.174–32706.699 | Queued GET-based nth1 BACKFILLING assertion finds MATERIALIZING and fails after5000ms. Context retains gen0,200/261,min121. |

Fixture `support/stock-beta-fixture.mjs:96–113,411–420` advances all pending
memberships on **every list or per-ID GET**, independent of caller. Each GET
increments a global pollCount; POST resets it (425–441). The server page
`app/(authenticated)/stock-beta/page.tsx:82` also fetches that list while producing
RSC. The retained initialMemberships prop is MATERIALIZING, not merely a debug record.
Browser polling uses `stock-beta-workspace.tsx:419–431`; the RSC prop update
assigns `initialMemberships.memberships` at260–264, overwriting the earlier UI list.
Thus the RSC read can advance state again and deliver MATERIALIZING while the test
still expects a prior GET's BACKFILLING. Start timestamps do not establish server
processing order; the two retained payloads establish the distinct successive states.
No broker, identity-schema or generation defect is shown.

The disable path sets pendingSignalRemovalInstrument (workspace552–553); its500ms
initial poll (51,445–463) handles the observed no-snapshot404 and calls router.refresh.
That is the source-consistent initiating chain. The trace has no application JS
initiator stack, so the exact router.refresh callsite attribution remains inference.
The later READY refresh at394–405 is another reader, not evidence of READY here.
`readyIdentities` (spec134–139) is also a state-consuming GET: stream fixture422,
610 and235 use it for lease validation, routed GET and real SSE validation through
callbacks at spec1352. Its six test-side GETs are traced; none overlaps this
replacement failure. Do not blame it for this occurrence, but account for it in a repair.

**Smallest robust case4 contract proposed for adoption, not implemented:**
allow edits only to `apps/web/tests/e2e/stock-beta-intraday.spec.ts`,
`apps/web/tests/e2e/support/stock-beta-fixture.mjs`, and
`apps/web/tests/stock-beta-fixture.test.ts` for fixture regression coverage.
Use a case4-only explicit scenario option, e.g. `stockBetaLifecycle: "controlled"`.
In that mode list/status GETs are read-only; hold each phase until the test observes
it. Add a synthetic-only exact POST `/__test/stock-beta/membership-advance`, handled
by the existing fixture dispatcher (synthetic-api215–226 needs no edit), accepting
only membership_id + expected_generation + expected_lifecycle. Compute the single
next state server-side: REQUESTED→VALIDATING→BACKFILLING→MATERIALIZING→READY.
Require owner/controlled mode; reject malformed, stale/disabled/unknown identity,
wrong generation, out-of-order or terminal advancement without mutation. Return
production membership-status shape; pending gen0/READYgen1 and existing coverage
values remain. Outside that explicit mode the control route is unavailable and
legacy GET advancement stays intact for other callers.
Both real UI POST202 receipts/new UUID checks remain. For each admission, advance
one stage, let the real UI poll/RSC consume it, assert exact ID-bound rendered
instrument/generation/lifecycle, then permit the next stage. Keep all five lifecycle
assertions, disabled M1, fresh M2/gen1, real EventSource,404 probes and later counts.
Do not synthesize a DOM state, remove BACKFILLING, or replace it with “at least this
stage.” Repeated reads must not perform work. A response-finished wait does not prove React committed.
Regression scope: repeated list/status reads, two-admission identity isolation,
invalid controls have no effect, coverage/generation, reset clears mode, and existing
automatic behavior. Direct module consumers are fixture test13–37, intraday catalog
test22–34 and synthetic-api11/163/215. Browser/server/stream readers above remain
unchanged; the new mode makes their reads harmless. No production endpoint changes.
**Case6 — observed facts and unresolved mechanism.** Title: “clears hidden/offline
scopes, bounds reconnect and latches logout”; ID
`d6c1cbb812c314790e36-28cabdf7f73ebf74512e`, declaration1957, failure2045.
The Strategies Link completes its RSC200 response (43245.769,46.465ms); before
return the URL is Strategies but Loading is already present. The real return Link
RSC GET starts43406.335, completes200 in35.274ms, and response.finished succeeds.
Its decoded StockBetaWorkspace initialMemberships prop contains READY/gen1 membership;
no Flight error record was found. This rules out a missing READY payload as the
observed explanation; response completion does not prove client decode/commit.

| Finite receipt | Fixture/Date ms | performance ms | Board / Loading | Creates / releases / opens |
| --- | ---: | ---: | --- | --- |
| Before Link return | 1791774006796 | 6302 | 0 / 1 | 2 / 2 / 2 |
| RSC response complete | 1791774006796 | 6302 | 0 / 1 | 2 / 2 / 2 |
| After two16ms frames | 1791774006828 | 6334 | 0 / 1 | 2 / 2 / 2 |

The board check43517.665–48519.222 fails; no third lease has been created.
Installed React DOM source18082–18119 delays retry-lane commits until
`globalMostRecentFallbackTime + 300` (constant29817); Suspense commits update that
time at16095–16099. Scheduler now uses performance.now (scheduler185–190), and
React captures setTimeout (29970). Next supplies Suspense at page154–168 and
layout-router477–498. Playwright1.61 pauses real-time advancement and runs only
eligible timers during runFor (coreBundle11434, decoded103–138/206–287).
A300ms pending commit would outlast32ms. **This is a supported hypothesis, not a
proved timer invocation.** No pending timer/React lane/fallback timestamp was
captured. Suspended decode/module/style work is not excluded. The matching current
.next chunk contains this throttle, but trace JS bodies have no retained hashes;
its exact served bytes cannot be authenticated from the trace.

**Minimal case6 diagnostic, before another scheduling fix:** temporary
spec-only metadata observation, same real Links/response wait and paused clock.
Read the pinned clock's `globalThis.__pwClock.controller._timers` (exposed by
coreBundle11911–11912), without wrapping timers or touching React/controller state.
At existing before-return/response-complete/after32ms points, retain bounded counts,
Date/performance and only type,createdAt,callAt,delay plus a whitelisted callback
classification (`bound completeRootWhenReady`, otherwise OTHER); never args,
callback source, raw stacks, payloads or sessions. Abort diagnostics on missing
private shape or excessive entries. If exactly one matching pending commit timer
has remaining time in(0,300]ms, advance synchronized runFor only to its deadline−1,
record board/source state, then1ms to the deadline. Keep existing5000ms assertions,
require that timer removed, Loading cleared, board1 and the fresh third lease/source.
If no such timer, multiple candidates, early board commit or continued Loading,
retain the finite evidence and fail; do not add a300ms cushion or another loop.
This distinguishes a pending throttled commit from “no such timer” and “timer fired
but another blocker remains.” All subsequent reconnect/logout assertions still run
on success. Those tail phases and case4's post-replacement recovery remain unverified.

**Later validation, only after review and a new Web slot.** No commands below ran.
Fixture coverage: `npm test --workspace @lagrange/web -- tests/stock-beta-fixture.test.ts tests/stock-beta-intraday-catalog.test.tsx`;
then `npm run typecheck --workspace @lagrange/web` and scoped Biome checks.
`D/two-cases.test-list` comes from the exact two normalized result identities;
reconfirm titles/current collection before use. The existing wrapper has no focus
flag. A reviewed task-private diagnostic driver must reuse its lock, port reservations,
minimal ENV1 environment, ownership/nonce handshakes and final writer cleanup for
one WS API/Next pair. Under that supervisor, cwd apps/web, the focused child command is:
`node ../../node_modules/@playwright/test/cli.js test --project chromium --workers 1 --retries 0 --forbid-only --reporter line,json --test-list "$D/two-cases.test-list" --output "$D/playwright" tests/e2e/stock-beta-intraday.spec.ts`.
Use an absolute fresh evidence path/browser cache; remove raw reporter JSON after
owned writers stop. Do not run this child against an independently existing server.
Focused acceptance: both exact bodies complete, no skipped/retried/flaky results,
all lifecycle/proof/counter assertions and owned cleanup pass; timer hypothesis
classification must be explicit. Then run the unchanged complete command
`bash scripts/qa/candidate-web-e2e.sh --web-port 33020 --api-port 38182` serially.
Final gate:121+13+1 actual executions,134 unique synthetic identities, both owner
modes, no failures/skips/retries/flaky/provider attempts, and cleanup_ok. Keep
ENV1, BODY1 and passed CLOCK1 cases3/7/8 intact; no continue-after-failure change.

**Checks, retained evidence, limits.** `python3 -B D/extract.py` exited0 (stdlib
ZIP/JSON projection only). Inline stdlib hash/chronology/redaction assertions exited0;
all four supplied source/preimage and four failure-artifact hashes match. Local
cat/sed/rg reads and HEAD check only; guessed standalone Playwright clock and
other source paths produced missing-path output, then actual bundled sources were
located/read. No necessary source remains unfound; no modules were executed.
`D/case-4.json` SHA256 `861682cd7dbdfcc77006fbb139c440b6531256035aab3713f671244277f93b2b`;
`D/case-6.json` `252d4c9fdc4f8f46fda889311ba5d746ad9a94322aa0880b5f629578df1e1c3a`;
`D/source-hashes.json` `776dace90b8cbca6dd2789165b362d6434696e3e8b60de3d13737cb1ab539c77`;
`D/verification.log` `ef3d83d3f3251fd8260800720c9732d362d776f4ee9920318d7b612641717aa2`.
Only this new report (entire file) and sanitized private evidence changed.
Deviations: none. Further decomposition: none. Missing: case6 pending timer/lane
observations and exact served JS bodies; screenshots/video were not visually reviewed.
Not verified: proposed fixes/diagnostic, either remaining full body, separate owner WS
execution or final CI acceptance. Owned runtime resources created: none; actual idle.
Scoped report diff check and final stdlib whitespace/source-hash audits exited0; this report spans lines1–180.
Required memory finish: sandbox STATE_ERROR/exit1, identical narrowly approved invocation exit0, SKIPPED/savedfalse; no durable memory saved.
