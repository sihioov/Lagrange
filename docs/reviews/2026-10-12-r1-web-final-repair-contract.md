# R1 Web final bounded repair contract

2026-10-12 · **SPECIFICATION_READY** · R1-05 · source/evidence diagnosis only.
HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.
No implementation, runtime, resource probe or delegation performed. Actual idle.

## Exact inputs and receipts

`R = target/r1-web-e2e-1791811172834544000-4f7de1dd8126/`.
`A = R/market-stream/market-stream-results-playwright-artifacts/`.
`D = target/r1-runtime-fxi6ltmv/web-final-repair-contract-xiim195q/`.
Root provenance: `target/r1-runtime-fxi6ltmv/web-timer-root-20261012T1320/`.
Root `driver.exit` is 1; `R/diagnostic-outcome.json` records selected2/pass0,
skip0/flaky0/retry0, reservation/cleanup true. No fresh cleanup probe was made.
This focused run stops before owner-empty under unchanged fail-fast. Full selection
remains135 source/134 synthetic identities/135 executions; this is not a selection defect.

| Frozen source | SHA256 |
| --- | --- |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | `952eacf484595f2120fb4132e4e7ea3f3a3b02cd9c6443660e25095d939bc906` |
| `apps/web/tests/e2e/support/stock-beta-fixture.mjs` | `0aee2c8fd497ae1e041ce65b65a6114e2e5197b07c9d98e3c5e1a11f182a6822` |
| `apps/web/tests/stock-beta-fixture.test.ts` | `ea3e9ddbdc6517c5458fffdf33d06c096c92c3a05b1ece579f883fc99cad3d46` |
| `scripts/qa/candidate-web-e2e-runner.py` | `c8c7e32631fae2d392c19ea059e34314d46d10fbda517738abce92398b23afb4` |
| `target/r1-runtime-fxi6ltmv/web-timer-probe/focused-driver.py` | `84e8c85cb7c913ef59323f0f61bb8d2d48c5476b756d791dc7acb78974bf68fe` |
| `apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | `ecf788b3c64868fcf5a87cbfbc0814805ec7d2277bebce7fd5ed757aa82826fa` |

Artifact directories are `stock-beta-intraday-provid-556d1-trument-with-a-new-identity-chromium`
(case4) and `stock-beta-intraday-provid-d5aa2-econnect-and-latches-logout-chromium` (case6).

| Artifact under A | SHA256 |
| --- | --- |
| case4 `trace.zip` | `0d2ca9b8df17a9c84a6edf3286766d1a0f1b5977e420bbd47f40d5a477e782ba` |
| case4 `error-context.md` | `4ca5adabed9f103da1f88af553b5eb20187a8c635406d47c7ce7162941d6da7d` |
| case6 `trace.zip` | `571fad4ebb7c076e58ad9f5f12b6b6885e21bb1529fcd16f9c4fc00679afff4f` |
| case6 `error-context.md` | `94831fd86076058e98d26532c41a5720ca4d33c6104f008b67e3212a0daf4430` |

`D/hashes.json` also hashes supporting product/installed source, root provenance and run receipts.
`D/finite-receipts.json` SHA256 `9dbd14474cdb43b9aaa3cb7ffa45898bde470a88997d33af58254472ad4569ae`
contains only finite phase/admission/timer metadata; no identities, payloads or credentials.

## Case4: proved stale ledger defect; high confidence

1. Both real admission receipts record REQUESTED/gen0 and DOM VALIDATING,
   BACKFILLING, MATERIALIZING, READY/gen1. The controlled lifecycle block passed.
2. Trace `test.trace` expect@669 starts3535.496ms, fails8545.860ms at helper631,
   caller1728: the **first old-membership invalid delta**, not disable or wrong generation.
   Pre-delivery receipt has connections2/creates2/releases1/renewals0.
   Assertion-end browser snapshot8543.439ms still shows selected LIVE/106030.00.
   The later error-context is after cleanup and must not establish pre-cleanup purge.
3. Spec450–467 obtains a ledger copy and computes nextSequence from its `sent` array.
   Stream fixture774–782 explicitly `structuredClone`s that array. It is not a live view.
4. Spec1700–1714 captures `secondConnection` **before** its first snapshot. Fixture248–254
   starts sequence0/sent[]; emit645–690 writes snapshot1 and updates the internal ledger.
   Spec1716–1727 then computes the invalid delta from the old empty copy: **sequence1 again**.
5. Product `intraday-stream-state.ts:143–147` ignores sequence<=accepted sequence before
   membership checking254–260. Thus the test invokes duplicate-delivery behavior instead
   of the intended fresh-event stale-identity rejection. Remaining LIVE is consistent.
6. The unreached second probe repeats the defect: recoveryConnection1730 is captured
   before reset1/snapshot2; wrongGenerationSequence1753 uses its empty copy -> sequence1.
   This is a source-proved latent defect, not an executed second failure.

The stream HAR has no retained SSE body, and no browser receive acknowledgement exists;
the exact wire payload/receive branch is not independently observed. The deterministic
source construction is proved against the frozen spec; delivery is still a runtime gate.
No evidence requires changing production fail-closed semantics. Control updates flush
directly through provider100–108/coalescer43–70,110–113; treating this as an rAF-only
purge defect would be unjustified. State191–196 clears all rows on a fresh invalid event;
controller345–361/395–403 closes the source and schedules the existing retry.

## Case6: diagnostic premise disproved; bounded readiness remains conditional

Safe receipts from the exact trace, counters shown as creates/releases/opens:

| Phase | Fixture/Date ms | performance | board/loading | Counters |
| --- | ---: | ---: | --- | --- |
| before-return | 1791774006796 | 6302 | 0/1 | 2/2/2 |
| response-complete ordinary receipt | 1791774006796 | 6302 | 1/0 | 2/2/2 |
| response-complete timer receipt | 1791774006796 | 6302 | 1/0 | 3/2/3 |
| after two frames / timer capture | 1791774006828 | 6334 | 1/0 | 3/2/3 |

The same named commit timer created6302/due6602 remains present throughout. Remaining268
passes, but spec2203 fails expecting board0. The original board1 and fresh third scope
are already satisfied observationally. Different counters between adjacent receipts show
real async progress at frozen virtual time; receipts are not one atomic browser/server snapshot.
**High confidence:** existence of this timer does not imply the desired board is blocked.
Which React root/lane owns it is unknown; private timer metadata cannot prove causation.

Installed React DOM18082–18119/29817 supports a300ms retry fallback throttle;
Scheduler185–190/220–226 combines fake performance with runnable MessageChannel tasks.
Public Clock runFor advances due callbacks (decoded coreBundle.js11434, lines103–125,
257–287; decoded SHA `8fc4074777a88a3c61f5e2291703daa4393703c68fa26964bc9ff2250d1d1e34`).
This supports a finite clock-driven render opportunity, not a guarantee that every
navigation waits300ms or that300ms cures every possible blocker. Historical loading
failure remains evidence for checking readiness, not for asserting that this timer blocks it.

## Closed implementation contract — one source file only

Allow **only** `apps/web/tests/e2e/stock-beta-intraday.spec.ts`; no fixture/product/runner edits.

### A. Case4 fresh event construction (1700–1784)

- Immediately before each of the two invalid emissions, reread `fixture.connectionLedger()`
  and find the **exact captured connection ID**. Assert present, still open, same lease ID
  and stream ID; do not substitute whichever connection happens to be active.
- Compute nextSequence and envelope/event ID from that fresh copy. Assert current sent max1
  then candidate2 for old membership; current sent max2 then candidate3 for wrong generation.
  Assert candidate>every prior sent sequence. Both body/event-ID sequences must agree.
- Keep the old disabled membership row and replacement+1 generation row unchanged, including
  prices9/8, real `emitInvalid`, immediate existing `expectNoCapturedQuote`, resets/full snapshots,
 1000/2000ms reconnect bounds and all final exact metrics. **No extra frame/wait or clock step.**
- Add finite pre-emission metadata receipts: phase, prior/next sequence, exact-connection and
  same-lease/stream booleans; no UUID/body. After existing purge assertion require the exact
  source's closedAt nonnull using the existing5000ms assertion budget, with unchanged creates,
  releases/renewals and no new connection before the existing reconnect helper.
- If fresh frames still fail purge, stop with delivery/close/DOM receipts; do not weaken
  duplicate suppression, identity rejection, UI purge or production logic.

### B. Case6 public bounded readiness (704–765,2149–2233)

- Delete the temporary private timer helper and all its calls/assumptions, including
  deadline-minus-one/timer-consumed/board-must-still-be-zero checks. Retain ordinary receipts.
- Preserve real Link, exact RSC response predicate, successful response.finished, URL assertion
  and the existing two16ms synchronized frame pumps. No new request or navigation replacement.
- Add a case6-only bounded readiness helper after those pumps: observe actual board count1
  AND exact Loading workspace count0 first; return without advancing when already ready.
- If not ready, perform at most17 synchronized public `runFor` steps: sixteen16ms steps then
  one12ms step, checking the DOM after each and stopping on readiness. Additional cap268ms;
  total virtual budget from response-complete is300ms including the original32ms. Fail closed
  at exhaustion. This is a finite observed-condition budget, not a unconditional300ms delay.
- Read both DOM counts in one browser evaluation; retain finite phase/elapsed/Date/performance
  and lease counters at entry/exit/failure. Keep the existing5000ms assertion wall budget for
  this readiness step; no increased global/test timeout, retry, clock resume or private API.
- After the helper keep explicit Loading0/board1 and **exact** third fresh lease/source checks,
  then every offline/online, retained-price,1/2/4 retry bound, exhaustion and logout assertion.
  The helper is confined to Link return before those backoff phases; never use it inside retries.
- Bound300 is an evidence-supported candidate contract, not established runtime acceptance.
  If it fails, retain finite evidence and stop; do not enlarge the budget opportunistically.

## Minimum later validation (separately authorized slot)

1. Static diff: all12 titles, controlled case4 lifecycle, all tails and other cases unchanged;
   no `__pwClock`, `_timers`, temporary callback matching, skip/retry/only or new dependency.
   Scoped installed Biome check from apps/web, `git diff --check`, then existing Web typecheck.
2. Reuse unchanged pinned driver: `python3 -B target/r1-runtime-fxi6ltmv/web-timer-probe/focused-driver.py --execute`.
   Use approved local Node/PATH and explicit existing `PLAYWRIGHT_BROWSERS_PATH` only as needed.
   Require both complete bodies plus separate owner-empty:3/3, exit0, no skips/retries/flaky,
   native ownership/cleanup true. Receipts must prove fresh invalid2/3, both purges/recoveries,
   readiness elapsed<=300 and every case6 tail. Preserve failures if any.
3. Then unchanged `bash scripts/qa/candidate-web-e2e.sh --api-port 38182 --web-port 33020`.
   Require wrapper regressions, exact complete inventory, off121 + WS13 + owner-empty1 =135
   successful executions/134 identities, no skips/retries/flaky and owned cleanup. No @actual.

## Work record and limits

Actual checks: bounded `sed`/`rg` source reads; stdlib ZIP/JSON/AST-literal/SHA parsing,
source-proven sequence derivation, receipt extraction and report whitespace/hash checks.
No project module was imported/executed. A search under nonexistent `apps/web/src` failed;
corrected to real `lib`/`components` paths. Some combined output truncated; decisive spans
were reread separately. No test/runtime exit is claimed from these read-only checks.
Inspected spec190–231/441–469/590–645/673–811/1330–1376/1410–1784/2090–2245;
stream fixture245–278/570–742/764–782; product spans cited above; relevant controlled/probe
reviews and installed navigation guide. Unrelated source and case6 later tails were not
re-reviewed; their preservation is mandatory in implementation review.
Writes: this report only (tracked scope), plus D's private0700 directory/0600 sanitized files.
Deviations: none. Further decomposition: none. Missing required input: none.
Unknown/unverified: actual fresh-frame delivery/purge, complete case4/case6 tails, cold/slow
navigation under the proposed bound, owner-empty follow-up and full135 execution acceptance.
Unread evidence: videos/screenshots not needed; ZIP DOM/receipts provide decisive observations.
Worker480's evolving report was not consumed. No production defect or full CI success claimed.
