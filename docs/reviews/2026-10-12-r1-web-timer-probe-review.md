# R1 WP-5 — private CASE6 timer probe source review

2026-10-12 · **REVIEW_COMPLETE — preparation source acceptable, conditional on
future integration hash/typecheck review; no execution authorization or dynamic acceptance.**
Repository HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.

## Findings

Actionable findings: **none**. No blocking or optional source correction identified
in the three frozen private inputs. Case6's React commit-throttle explanation
remains a hypothesis, not a confirmed cause or a production repair.

## Frozen scope and read coverage

`P = target/r1-runtime-fxi6ltmv/web-timer-probe/`.
All three private files were read in full; each supplied SHA256 matched before
and after review. The official runner also remained at its accepted pin.

| Input / inspected span | SHA256 |
| --- | --- |
| `P/focused-driver.py:1–91` | `84e8c85cb7c913ef59323f0f61bb8d2d48c5476b756d791dc7acb78974bf68fe` |
| `P/temporary-helper.ts:1–58` | `49765f069e9d7fa6dade97fc6c66160668bc920688632fc8d7cf88c111de1024` |
| `P/temporary-boundary.ts:1–27` | `e95330b0db4c464b9cb0fd331bd5f4ba5fe31dc15edae8c8facc0409a3c24435` |
| `scripts/qa/candidate-web-e2e-runner.py` | `c8c7e32631fae2d392c19ea059e34314d46d10fbda517738abce92398b23afb4` |

Relevant runner reads: 26, 33–53, 1478–1615, 1617–1630, 1650–1860,
1973–2110; reuse prior accepted ownership, environment and cleanup review.
Read diagnostic report `2026-10-12-r1-web-final-diagnostic.md:1–180`, its
source-hash/check records and clock excerpts. No mutable case4 source was read/hash-checked.

For insertion context, read only the retained preimage
`web-controlled-lifecycle-20261012/preimages/apps/web/tests/e2e/stock-beta-intraday.spec.ts`
(SHA `f68a8acb9282e6f1ec922cefdc4b9b24c66c1788a32278b0271c512bc05540fe`):
1–80, 183–230, 673–706, 1957–2250. Its clock binding, three phase anchors,
board/fresh-third-lease checks and subsequent lifecycle/retry assertions remain
the relevant unchanged integration contract. The earlier retained preimage's
170–238 was also read to corroborate the same clock helper.

## Driver ownership, selection and exit behavior

Driver24–34 checks the sole CLI form and exact runner bytes before real-path import;
the runner derives ROOT from its own file. AST parsing independently confirms
the subclass overrides only `run` and `_run_browser_phase` (36–82).
Full collection still executes the official inventory validation, including
the separate actual identity, both lane inventories and exact owner-empty selection.
Driver39–47 requires precisely the two named intraday cases and one owner case,
then emits the same project/file/full-title test-list grammar as the official helper.

Driver57–82 retains official lane startup, reservation/identity checks, raw-result
validation, strict expected-case matching and cleanup. Only the main browser
phase's file/list selection changes. The unchanged owner-empty phase runs after
main success; any nonzero exit or invalid results stop it. No off/actual browser
phase runs; their source identities are still collected with `--list`.
The aggregate lane deadline is 600 seconds, including startup and both phases.
Main's lock, subreaper, signals, final cleanup and original failure handling remain.
Configured33020 is reserved; the WS Next instance retains official33043.

Driver48–71/86 clearly labels scope/outcome as diagnostic-only, three executions,
and not complete Web acceptance. The inherited `coverage.json` describes full
*collection*, and inherited `run.json` may say passed for this focused run;
neither is full execution evidence. Read them with the diagnostic receipts.

## Timer shape, deadline and information boundary

Independently decoded installed Playwright `coreBundle.js:11434` without executing
JavaScript: SHA `8fc4074777a88a3c61f5e2291703daa4393703c68fa26964bc9ff2250d1d1e34` matched.
Read decoded43–64, 103–138, 206–287, 475–525, 607–650, 655–657 and bundle11900–11920.
`addTimer` normalizes finite delay for every timer kind; `_timers` is a Map.
`callAt` and performance use the same ticks; `runFor` consumes eligible timers
through the inclusive deadline and deletes one-shot timers before callbacks.
The retained synchronized helper advances each participating context once per
step; remaining1 means its first zero-duration advance is harmless.

Read React DOM18060–18130, 29805–29825 and scheduler175–198; located
now26606/setTimeout29970. Installed React/Scheduler/clock bundle hashes matched
the diagnosis manifest. React's bound `completeRootWhenReady` and 300ms throttle
support this probe's relevance, without authenticating the earlier served bytes.

Helper8–40 accepts at most64 timers, projects finite metadata and an exact callback
whitelist, and retains no timer args, source, stack, identity, session or payload.
Boundary2–27 requires exactly one candidate and integer remaining(0,300], verifies
Loading1/board0/counters2 at capture and deadline−1, then exactly1ms to consumption
and Loading0. Missing/ambiguous/early/continued-blocker outcomes fail; no added
retry, resume or guessed cushion. Consumption plus UI progress is diagnostic
correlation; it does not independently prove React lane/root attribution.

## Methods, limits and follow-up

Actual methods: read-only `rg`, numbered/ranged reads, Git HEAD read, stdlib
SHA256/JSON/AST parsing and literal decoding. No reviewed module was imported or
executed, and no tests/compiler/browser/runtime/resource/network operation ran.
One overlong bundled-source search was truncated; bounded decoding recovered the
required source. A source-excerpt range caused IndexError; corrected bounds passed.
These were reviewer read/parsing issues, not runtime results.

Missing required preparation inputs: none. Unread required preparation hunks: none.
Deviations: none. Further decomposition: none. Future integrated source is absent
by design: verify exact helper/case6-only placements, unchanged Link/response waits,
board1/fresh third lease and all remaining assertions, integration hashes and
typecheck before any diagnostic execution. Dynamic private shape/name behavior,
timer presence, actual integration, case6 tail/cleanup and complete Web/R1 gates
remain unverified. Mutable case4 changes and unrelated runner code are excluded.

Owned writes: this new report and `P/wp5-source-review.json`, SHA
`742874ccfefd5015a87a3762d75cf3689d620ccc5a1332eaac07b69d82b6c3ba`.
No source or previous report changed; no runtime resources/processes created.
Actual idle at completion. No overall WP-5/R1 acceptance.
