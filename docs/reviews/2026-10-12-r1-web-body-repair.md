# R1 WP-4d Web body repair — source/static evidence, 2026-10-12

## State and boundary

Current coordinator disposition: source correction below resolves the detail target;
independent review and browser validation remain pending. The worker handoff follows
unchanged as historical evidence.

**SOURCE_STATIC_PARTIAL — WEB7_DETAIL_EOD_TARGET_UNSPECIFIED; actual idle.**
The implemented portions pass the scoped static checks below. The required detail
EOD605020 assertion has no identified target in the current detail contract, so
this report does **not** declare READY_FOR_HEAVY_VALIDATION or browser acceptance.
A narrowly scoped clarification was requested; no replacement assertion or
product change was invented.

Repository: `/data/worktrees/3puw275b/orange-dugong`, branch `orange-dugong`, HEAD
`ac0a08364788e6c2f234cf85025a8d975f362d31` plus preserved shared R1 modifications.
Roadmap context: R1-05/R1-08; roadmap, prior reports and diagrams remain coordinator
owned. This assignment changes only the two authorized sources and this new
report, with private static evidence under the task directory below. No source
from the accepted WEB1/WEB2 supervisor repair was changed.

Read: `/home/l1nnx/.codex/AGENTS.md`, root/app instructions, relevant roadmap,
all167 lines of `2026-10-12-r1-web-test-contract.md`, prior `r1-web-tests.md`, all
WEB3–WEB8 findings/minimum corrections at `r1-review.md:1315–1438`, and supporting
source/coverage evidence. Installed Next testing guidance was read at
`node_modules/next/dist/docs/01-app/02-guides/testing/playwright.md:120–160`.
No native delegation was used.

## Changed files, hashes and mapping

Hashes are relative to the exact WP-4d preimages supplied by the coordinator,
not a claim that the entire shared worktree equals HEAD.

| Source | Pre SHA256 | Post SHA256 |
| --- | --- | --- |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | `85886e96741b1dc8f6a9680d8488db5085f5af4d8f47778f95aea57d90d10d45` | `5c36a7f344dbd5482665fb2117977aa5c2c83426a024c33bcc6ed60947dd9939` |
| `apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | `b3c5474885e82ebda3b4682571c14a5e826c63230974a25ad97d20c42f97d126` | `ecf788b3c64868fcf5a87cbfbc0814805ec7d2277bebce7fd5ed757aa82826fa` |

Current source spans (inclusive; exact insertion/replacement spans are in
`source-final.json`):

- Imports/types/bindings: spec2–9,17,79,112. WEB5 clock helpers:182–225,365–425.
- WEB3 + case3 WEB6: spec1120–1146,1227–1232. A-only positive delivery checks A's
  board101300 and preserves selected B198400 and its proof. Wrong generation and
  wrong instrument normal deltas throw the production contract error without
  changing metrics or the connection ledger. Explicit invalid B still uses
  `emitInvalid` and reaches the existing browser path. Recovery checks A's LIVE
  board101200 independently, selected B identity, then B's board/panel198400 via
  the existing helper. Original lease/source/reset/purge counts remain.
- WEB4: spec3142 inner auxiliary close wait is `extraOpens`. Final111 auxiliary
  closes while UI remains open,112 after unmount,111 sequential real GET probes,
  peak≤2, lease/demand/renewal and board proof assertions remain unchanged.
- WEB5: clock install/fixed/pause/system-time sequence precedes navigation at one
  recorded fixture instant. Fixed Date ends before app navigation; no hydration
  cushion or later pause remains. Reused contexts assert the same binding and
  exact Date without resetting. Existing `runFor` advances the fixture and every
  participating context once per step, including other tabs; no fastForward.
  Two synchronized16ms frames support initial RAF painting. Finite clock receipts
  record the actual browser/fixture navigation and post-paint instants when run.
- WEB6 fixture:669–679 normal delta authority now matches all three lease tuple
  fields for every row and still permits valid subsets. Guard precedes sequence,
  ledger, counter and response-write mutations. Negative emitter and all other
  fixture functions are token unchanged.
- WEB7 case2 partial: spec892–912 restores the original HEAD397–415 varied-canvas
  pixel algorithm and >5 colors;954,970,1000–1012 repeat the exact matched candle's
  hover/OHLCV and raw605020 after delta, alongside retained request/snapshot/
  generation assertions. **Detail EOD assertion remains unspecified**, below.
- WEB7 case7:2086–2091,2099,2108–2115,2122 checks nonnull R1, emitted R2 strictly
  greater than R1, and exact emitted R2 in board attribute, board time and selected
  time. Existing proof/replay/recovery assertions remain.
- WEB7 case10:2802,2806–2833,2857,2873 checks visible board and selected retained
  good101200, LAST_KNOWN, identical epoch/version/ordinal/receipt through both
  unhealthy and healthy statuses. Valid fresh-delta recovery remains.
- WEB8: case5 acquisition/finalization and two internal failure probes at
  spec1481–1601,1651–1764; minimal existing cleanup-helper hook at696,716. Cleanup
  scope begins before fixture acquisition; each context is recorded before page
  creation/guard setup. Nested independent finalizers close the adapter and
  attempt every acquired context even after release assertions or adapter close
  fail. Primary body error takes precedence; cleanup failure also surfaces when
  there is no primary error. Original normal-subphase assertions/guards remain.
  Within the existing case5 declaration,30s bounded steps inject failure after
  the first owned context and at a release assertion after genuine app release.
  They require original failure propagation, all acquired contexts/pages closed,
  fixture close completed, zero active sockets/leases and exact release/source
  counts. These browser failure probes have **not** been executed in this phase.
- This report: new file, all lines. Private evidence contains preimages, diffs,
  bounded logs, command receipts and static audit output; no raw runtime output.

## Required WEB7 clarification

Contract70 and review1410–1416 require unchanged detail EOD605020. Current detail
catalog `apps/web/components/stock-beta/detail/widget-registry.ts:24–95` contains
instrument-header, returns, risk, activity, snapshot, policy-boundary and
current-quote, with no EOD chart/close widget. `stock-beta-detail.tsx:49–99` supplies
the signal/snapshot view model to that catalog. Instrument header
`detail/widgets/instrument-header-widget.tsx:14–54` exposes generation, rank,
score, condition and as_of; returns/activity expose return/SMA/volume metrics.
No existing detail605020 selector or detail EOD response target was identified.

The required assertion cannot be honestly aimed at a nonexistent detail element.
No guessed selector, extra API read, or product/widget addition was made. The
coordinator must identify the intended existing assertion target or issue a
bounded contract correction. The current detail quote/readability assertions
remain intact. This is the one unfinished source requirement.

## Static verification and commands

Root cwd below means `/data/worktrees/3puw275b/orange-dugong`; Web cwd means
`/data/worktrees/3puw275b/orange-dugong/apps/web`. Installed Node24.13.1,
Biome2.5.7, TypeScript5.9.3 and Playwright1.61.0; no dependency or lock update.
No browser tests, unit tests or full suite were executed here; dynamic test
count/exit is **not verified**.

| Exact check (serial) | Cwd | Actual result |
| --- | --- | --- |
| `NO_COLOR=1 node_modules/.bin/biome format --write apps/web/tests/e2e/stock-beta-intraday.spec.ts apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | Root | exit1: nested root/app Biome configuration conflict; no config change |
| `NO_COLOR=1 ../../node_modules/.bin/biome format --write tests/e2e/stock-beta-intraday.spec.ts tests/e2e/support/stock-beta-market-stream-fixture.ts` | Web | exit0, formatted2, fixed1 in32ms |
| `NO_COLOR=1 ../../node_modules/.bin/biome check tests/e2e/stock-beta-intraday.spec.ts tests/e2e/support/stock-beta-market-stream-fixture.ts` | Web | exit0, checked2 in88ms,2warnings/52infos,34 diagnostics beyond default output limit |
| `../../node_modules/.bin/biome check --max-diagnostics=100 tests/e2e/stock-beta-intraday.spec.ts tests/e2e/support/stock-beta-market-stream-fixture.ts` with explicit NO_COLOR/locale | Web | final source exit0; checked2 in99ms, no fixes;2warnings/52infos; complete55638-byte log retained |
| `node target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/source-audit.cjs` | Root | initial exit1: literal-only inventory33≠39 in frozen stock-beta.spec.ts; next exit1: static parameter parser missing numeric401; corrected literal-array/template/numeric parser final exit0 |
| `python3 -B target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/source-finalize.py` | Root | exit0: source hashes/spans, five frozen hashes, full30 universe and exact installed clock source verified. Earlier inline metadata check exit1 from an overly literal pause-method text assertion; reread source126–131 and corrected that audit expression to the actual await/toConsume/shiftTicks form; no test-body clock change |
| `git diff --check -- apps/web/tests/e2e/stock-beta-intraday.spec.ts apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | Root | exit0, no whitespace diagnostics |
| `git diff --no-index --check target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/stock-beta-intraday.spec.before.ts apps/web/tests/e2e/stock-beta-intraday.spec.ts` | Root | exit1 denotes source difference, zero whitespace diagnostics |
| `git diff --no-index --check target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/stock-beta-market-stream-fixture.before.ts apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | Root | exit1 denotes source difference, zero whitespace diagnostics |
| `rg -n --glob '*.ts' --glob '*.tsx' stock-beta-market-stream-fixture apps/web` | Root | exit0, only importer is intraday spec25 |
| `rg -n 'MARKET_STREAM_INSTRUMENTS\|instrument_count !== 30\|instruments.length !== 30' apps/web/lib/products/intraday-stream-universe.ts apps/web/tests/e2e/support/stock-beta-market-stream-fixture.ts` | Root | exit0, production full30 import/check and fixture membership validation retained (actual argv uses regex alternatives, not shell pipe) |

Warnings are `lint/correctness/noUnusedVariables` for the unchanged
`membershipCards` and `ensureCurrentUiRows` helpers (current320,772). No lint
suppression was introduced; style infos were not broadly autofixed.
The static audit uses `ts.createSourceFile` and a scanner, never a TypeScript
program/typechecker, Web import, Playwright collection or app execution. It parses
17 distinct current/preimage files with zero parse diagnostics, preserves all12
titles and exact accepted title map, and verifies token-identical bodies1/4/6/8/9/11.
Only cases2/3/5/7/10/12 change. All32 original case5 expectation expressions remain.
No new test declaration/skip/fixme/only/retry/EventSource override/fastForward was
added. Static literal parameter expansion yields135 source identities, exactly
one dedicated@actual,134 synthetic identities, off121, WS14 and solely the named
owner-empty overlap. This is **source inventory**, not real JSON collection or135
passing executions.

Installed clock source was decoded as text only from
`node_modules/playwright-core/lib/coreBundle.js:11434`. Evidence source77–99
shows set-time clearing fixed Date, fixed-time setting it, and advancement
respecting that flag;126–138 shows pause preceding the time difference and
stopping real-time sync. Server implementation11845–11910 confirms context scope
and navigation init scripts. The installed source supports the accepted
install/fixed/pause/system-time order; actual cold/slow hydration remains pending.

Read-only investigation misses were resolved: guessed detail-view path and
stock-beta glob; guessed root `biome.json` (app config exists); guessed
`intraday-stream-connection.ts` (actual controller is in components/quote).
Their `rg`/`cat` exits2/1 were inspection failures, not passing validations.
`git show HEAD:apps/web/tests/e2e/stock-beta-intraday.spec.ts` supplied the original
canvas assertion. No unresolved missing instruction/document remains.

## Retained evidence

Private directory (0700, files0600, ignored task target):
`target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/`.
It is additional retained disk evidence, not a `/tmp`-only record.

| Artifact | SHA256 |
| --- | --- |
| `biome-format-root.log` | `94246a37b44d0dff77e53c72bcde5ed16729d4a7454529275cca51a689fccd8e` |
| `biome-format-web.log` | `06cd4d4334c8f348407673492d5345a591b119557ad53cf47799d335250cccc3` |
| `biome-check-first.log` | `da388151dc7154443c4b3f15238ce58ca4423f523e155b4795e3efb69fe64fb3` |
| `biome-check-complete.log` | `f32cb532f6183b8279bf4d8aa16fe8886de403188c4ec6983f8da6b5635e8bb7` |
| `source-audit-unsupported-number.log` | `1e7cfc51dc5eed9e3d4b4a7088cd426e33a2fa86427e1a876681948369403b54` |
| `source-audit-pass.log` | `24390d5c2392ba0505ee7bc3cc01cc8fa2a0fbfc0bc8883f6e1e7c4485682ada` |
| `installed-clock-source.txt` | `8fc4074777a88a3c61f5e2291703daa4393703c68fa26964bc9ff2250d1d1e34` |
| `stock-beta-intraday.spec.diff` | `646283ee6a16affded16d207bf188464d7fb5e753e10ede73720b99acc7ee3d6` |
| `stock-beta-market-stream-fixture.diff` | `81f14f4f72f07b8b3b88102caa7b309760551db231191a517e6935f7db4689f8` |

The matching `.json` receipts retain exact argv/cwd/exits/bytes/hashes; preimages,
`before.json`, `runner-selection-contract.json`, `source-audit.json` and
`source-final.json` retain inventory and source-change evidence. Five hashes match
their WP-4d starting values: runner, runner regression file, wrapper,
synthetic-api.mjs and stock-beta.spec.ts (owner-empty branch). Universe JSON count30
and SHA256 `2a0d55143df0274fcfa357f2824ed752e2969469f93254ed7dfa64766a00dde1`
remain unchanged by this assignment.

## Deviations, follow-up and not verified

- Specification deviation: none (없음) in implemented portions. One required
  assertion is withheld for the explicit WEB7 target gap, not replaced/weakened.
  Supporting audit-parser/config invocation corrections above are disclosed.
- Unresolved source requirement: identify/approve detail EOD605020 target. No
  further task decomposition is required for the other bounded repairs.
- Pending later granted heavy slot: scoped typecheck/lint as coordinated,
  real JSON collection/135 executions with134 unique synthetic cases, all12
  genuine intraday bodies, and existing off121/WS14 lanes. Specifically verify
  cold/slow hydration, reused contexts/multitab clocks, RAF painting, selected B
  recovery, full-identity rejection before mutation, exact R2 receipt increase,
  retained good price through both statuses, case5's normal and induced failure
  cleanup,111 real auxiliary probes/final112 closes, and the completed case2 EOD
  semantics once clarified. No passing count, skip count or CI success inferred.
- Actual browser/Next/Playwright collection, npm typecheck/test/build, Rust,
  Docker, DB, full Python and provider/network checks: not run/not verified in
  this source/static phase. Previously passed source revisions do not verify
  these edits. Functional research smoke/raw ownership remain outside this slot.
- Missing/unread required documents: none (없음). Missing assertion target: the
  detail EOD605020 target above. Other not-found items: none (없음) unresolved.
- Owned browser contexts/listeners/servers/children: none (없음); none were
  launched. Static commands completed. Foreign33001 was not contacted,
  signaled or reused. No lifecycle/resource cleanup was needed.
- No staging, commit, dependency/lock, runtime/environment/credential, product,
  supervisor, owner-empty, other report, roadmap or diagram edits.

Final read-only checks: `git diff --check --` the two sources plus this report,
`git check-ignore -- target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/`,
`git status --short --` those three files, and `git branch --show-current` each
exit0. The source is modified; the prior fixture and this report are untracked;
branch is orange-dugong. All private files are0600, the directory is0700, both
post hashes and all five frozen hashes still match. `source-audit.json` SHA256 is
`fcf9d776d183b45623af663fc8bb0a4b4de5ad0b6f48339f395fe8ef534a9196`.
Final manifests retain report/artifact hashes without modifying earlier evidence.
Required AGENTS.md memory finish: exact supplied helper/envelope initially exit1
STATE_ERROR (bookkeeping database unavailable under sandbox); the identical
narrowly approved invocation exit0, SKIPPED/no_durable_change, saved=false. No
durable memory was written.

The worker returns actual idle with **WEB7_DETAIL_EOD_TARGET_UNSPECIFIED**.


## Coordinator detail correction — 09:10 UTC

The case2 detail requirement was corrected against the existing widget catalog in
[the test contract](2026-10-12-r1-web-test-contract.md#coordinator-correction--case2-detail-target-2026-10-12).
No detail EOD price/chart exists. The existing dashboard EOD605020 and exact
OHLCV/canvas/hover gates are preserved. Coordinator added27 lines in case2 at
`stock-beta-intraday.spec.ts:1025–1077`: five nonempty provenance fields matching
the dashboard snapshot ID/as_of, the same READY generation and header, and three
numeric raw returns must remain identical after the existing LIVE snapshot and
at both900/390 widths. Existing frames, counters and12 test titles are unchanged.
No other source or product widget changed.

Pre-source SHA-256: `5c36a7f344dbd5482665fb2117977aa5c2c83426a024c33bcc6ed60947dd9939`.
Final spec SHA-256: `6c071b4b903952fc37f88f21e0aca2bf6c999e329b4506d27fab5c3a6303faec`.
Fixture remains `ecf788b3c64868fcf5a87cbfbc0814805ec7d2277bebce7fd5ed757aa82826fa`.
The exact27-line diff SHA-256 is
`a999b65efd56676b12a8caae23011999277ae30340e8a46757b0c89c2605002b`.

Scoped Biome format and check exited0; full worktree `git diff --check` exited0.
Biome retains2 warnings/41 info notices. Commands, hashes and preimage are in
`target/r1-runtime-fxi6ltmv/web-body-repair.8f69f693db94/coordinator-detail-correction.json`.
An initial broad-anchor patch attempt stopped at a uniqueness assertion before
writing source; the successful patch was explicitly bounded to case2.

Deviation: only the documented correction of the unsupported detail target.
No missing source inputs remain. Typecheck, browser behavior, slow hydration,
cleanup fault probes and complete candidate E2E remain unverified; this is source
readiness for independent review, not runtime or overall R1 acceptance.
