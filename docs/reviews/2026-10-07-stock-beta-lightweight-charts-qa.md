# Stock Beta Lightweight Charts — WP-3 QA

Date: 2026-10-07. **WP-3 complete; independent WP-4 review remains.** The final two-file browser run passed **34 tests, failed 0, skipped 12**. The skipped tests belong to the separate `owner_only` intraday suite; none supplies chart acceptance coverage. All required chart behaviors below have passing browser evidence.

## Scope and environment

- Workspace: `/data/worktrees/3puw275b/itchy-bulldog`; baseline HEAD `27ca8cf9f3c032daff32fb679f53ee54a78fdc96`. Product source was frozen after the coordinator accepted WP-2's layout/crosshair repairs.
- Direct worker execution; no delegation, product edits, package/lock edits, provider calls, production login, commits, merge or deployment. Only the two owned E2E files, chart fixture, this document and temporary tooling were edited by WP-3. Other worktree changes belong to WP-1/WP-2/coordinator.
- Node 24.13.1, npm 11.8.0, Next 16.3.0, React 19.2.8, Lightweight Charts 5.2.1, Playwright 1.61.0. Scripts ran from the repository root with `npm --prefix apps/web`; direct Biome ran from `apps/web` using `../../node_modules/.bin/biome`.
- Installed Next guides for Playwright and lazy loading were read before test preparation. No dependencies or browser runtime packages were installed.
- App: `http://127.0.0.1:33017`; synthetic API: `http://127.0.0.1:38182`; actual-component harness: `http://127.0.0.1:33018`. Exact PID/cwd/parent checks and HTTP 200 readiness preceded use. Unrelated port 33001 was never used.
- Intraday mode stayed **off**. HTTP guards permit only the owned app/synthetic origins; provider/account/order/Raw paths remain forbidden. Dev WebSocket handling permits only the exact app loopback `/_next/hmr`. Harness WebSockets are blocked, with only the exact local Vite HMR attempt excluded from the unexpected-traffic assertion. Production had zero WebSockets. Browser page-error assertions passed.

## Changed files and line ranges

| File | Lines | Changes |
| --- | --- | --- |
| [stock-beta.spec.ts](../../apps/web/tests/e2e/stock-beta.spec.ts) | 1–50, 91–668, 737–743, 864–872 | Response types, request/page-error/socket tracking, actual painted-pixel sampling, inner Canvas/axis geometry, visible hit targets, exact selected values, timed touch and scroll helpers; invariant chart locators. |
| Same spec | 1262–1416, 1570–1819 | Preserved response counts/dates/OHLCV and null fixture assertions; pointer/keyboard, zoom/pan/reset, no-JS route baseline, mount/tab, forced-colors and locale/license checks. |
| Same spec | 1920–1960, 2059–2145, 2258–2444 | Error removes old Canvas/details; five viewports and screenshots; real mobile page scroll, long-press values, horizontal pan and pinch. |
| [stock-beta-intraday.spec.ts](../../apps/web/tests/e2e/stock-beta-intraday.spec.ts) | 318–327, 391–415 | Exactly the approved off-mode `beforeAll test.skip` gate; retained `owner_only` assertion/test bodies; replaced the SVG candle assertion with ready state, exact HTML values and actual paint. |
| [stock-beta-fixture.mjs](../../apps/web/tests/e2e/support/stock-beta-fixture.mjs) | 306–313 | Optional five-observation internal SMA20 null interval; ordinary fixture behavior unchanged. |
| This document | Entire file | Acceptance evidence, commands, history, measurements and limitations. |

Temporary sources are recoverable in `/tmp/wp3-lightweight-harness/`; production measurement source is `/tmp/wp3-final-qa/production-probe.mjs`. No fake candle attributes or Canvas-as-candle counts were introduced. Canvas counts below describe instance lifecycle only.

## Final mandatory checks

Counts describe each invocation separately and must **not** be summed across reruns.

| Command | Exit | Final result / evidence |
| --- | --- | --- |
| `npm --prefix apps/web test` (native browser permission) | 0 | 57 files, **436 passed / 0 failed / 0 skipped**; `/tmp/wp3-final-qa/unit-native.log`. Includes adapter and renderer contracts. |
| `npm --prefix apps/web run typecheck` | 0 | Full check after the final E2E edits; `typecheck-accepted.log`. The earlier WP-2 `number \| undefined` E2E error is fixed in the owned range-count typing. |
| `npm --prefix apps/web run lint` | 0 | 267 files; 4 pre-existing warnings and 1 config deprecation info; `lint-accepted.log`. |
| Final focused two-file Playwright command below | 0 | **34 passed / 0 failed / 12 skipped**, 46 declared, 1.3m; `combined-final.log`. |
| Production build command below | 0 | Compiled in 2.5s; TypeScript in 3.9s; 4 static generation tasks completed; `build.log`. Dev Next was stopped first. |
| `git diff --check` | 0 | No whitespace errors. |

Exact final browser and build commands:

```sh
PLAYWRIGHT_BASE_URL=http://127.0.0.1:33017 SYNTHETIC_API_ORIGIN=http://127.0.0.1:38182 npm --prefix apps/web run test:e2e -- tests/e2e/stock-beta.spec.ts tests/e2e/stock-beta-intraday.spec.ts --project=chromium --workers=1 --output=/tmp/wp3-final-qa/combined-final
API_INTERNAL_URL=http://127.0.0.1:38182 NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS=1 NEXT_TELEMETRY_DISABLED=1 OWNER_INTRADAY_QUOTES_MODE=off npm --prefix apps/web run build
```

The 12 skips are intentional: the standalone intraday suite requires a separate `owner_only` build. The coordinator explicitly authorized its off-mode gate. Runtime activation and quote tests were not changed. The earlier combined result **33 passed / 2 failed / 11 did not run** is historical, not final. Its failures were the off/owner-only gate mismatch and vertical swipe while native long-press tracking was still active.

Lint warnings are `components/shell/language-toggle.tsx:8` and `theme-toggle.tsx:9` (`document.cookie`), and `dashboard.module.css:2047,2069` (`!important`). The info is the existing deprecated recommended-rules config in `biome.json`. These were left unchanged.

## Acceptance matrix

| Plan goal / requirement | Passing evidence |
| --- | --- |
| 1 — original data and source indices | Actual response lengths **1M 24 / 3M 67 / 6M 130 / 1Y 261**, ascending dates, latest 2026-08-28; default/latest selection, snapshot request pinning and freshness cases. First 1Y bar 2025-08-29: O/H/L/C/V **10100/10120/10080/10100/0**; last: **12045/12140/12025/12120/106000**. Pointer, keyboard and touch details match raw response values. Full unit suite covers the pure adapter. |
| 2 — actual price/volume paint, shared X axis | Both native pane canvases contain multiple nontransparent colors. Every inner Canvas, axis and table fits the host/surface; zero-width left-axis cells stay ≤1px; price/volume left and right edges agree within 1px. Assertions catch the old compressed surface and 112px gutter. |
| SMA internal null gaps | Route retains five internal nulls and actual paint. Additional real-component pixel probe proves **SMA20: 428 pixels before / 0 across gap / 480 after; SMA60: 195 / 0 / 220**. Each has two separate actual LineSeries; no connection across the null interval. `/tmp/wp3-lightweight-harness/artifacts/sma-gaps.json` and screenshots. |
| 3 — ranges, wheel zoom, drag/pan/reset, no extra fetch | Final route tests and production probe: at a fixed plot fraction, full **2026-01-30**, wheel zoom **2026-02-09**, drag **2026-07-09**, reset **2026-01-30**. Production browser chart-request count **0 → 0** because the initial 1Y data is server seeded. No request caused by gestures. Reset restores the full loaded range. |
| 4 — pointer, keyboard and touch inspection | Verified visible Canvas hit targets; first/latest pointer OHLCV and source indices at every required size; ArrowLeft/Right/Home/End, focus and HTML live summary/details. Mobile uses actual CDP touch events with exact values. Accepted WP-2 probes additionally inspect final committed crosshair state, including rejected controlled pointer/key requests. |
| Controlled/equal-array/display-only viewport contract | Coordinator-accepted `/tmp/wp2-chart-repair-evidence/{controlled,uncontrolled,fitted-data}.json`: request 136 with committed summary/crosshair 260; no repeated same-date requests; rejected boundary keys keep committed state; already-selected offscreen Home/End reveal; equal-valued arrays and locale/copy preserve range 60..95; reset and first/latest paint verified. These are explicitly reused WP-2 results, not new WP-3 runs. |
| 5 — rapid contexts/ranges and failure removal | Delayed A→B and rapid range changes, snapshot refresh pinning, loading retains current data, unavailable/integrity/missing/forbidden states, and subsequent range failure removing prior Canvas/tooltip all pass in the final suite. |
| Isolated library-import failure | `/tmp/wp3-lightweight-harness/import-failure.mjs`: ready chart first, then fresh document blocking only `/chart-wrapper.js`; surrounding React control still mounts; renderer `error`; zero Canvas/tooltip, old prices hidden, disabled reset, retained static instructions/legend/attribution. Exit 0, `import-failure-final.log`. |
| SSR / JavaScript disabled | `/tmp/wp3-lightweight-harness/ssr-check.mjs`: real `PriceChart` `renderToString` HTML, exact latest OHLCV, readable summary/details, five legend entries, keyboard instructions and attribution, disabled reset, zero Canvas, browser request only `/ssr`; no library module loaded by Node SSR. Exit 0, `ssr-check-node.log`. App route separately passes its pre-existing visible “Loading workspace” assertion. |
| 6 — responsive layout, overflow, focus | Required 375×800, 640×360, 768×1024, 1280×720, 1440×900; existing 200% equivalent uses 640×360. Resize, visible plot hit, unclipped surface/axes and each summary/tooltip/legend/control/attribution area after scrolling; focus and first/latest values; no horizontal page overflow. Final screenshots below were visually inspected. |
| Real mobile scroll/pan/pinch | 375×800 mobile/touch context, force=1, ten touch moves at 20ms, verified visible plot. Vertical chart swipe changes document/main/nested scroll signature. Separate swipe outside the profile changes page scroll with nested offsets unchanged. Long press 600ms inspects a real observation; a real ordinary tap exits native `OnNextTap` tracking before subsequent pan/pinch. Both horizontal pan and two-touch pinch change observed dates and retain raw OHLCV with no chart fetch. Focused `mobile-1.log` and `mobile-state-final.log` pass, as does the final combined run. |
| 7 — lifecycle/Strict Mode and integration | Five actual unmount/remount cycles under React StrictMode: mounted **1 active chart / 11 Canvas / 62 active-global-connected listeners / 12 observer targets**; after each unmount **0 / 0 / 0 / 0**; zero native crosshair delegates and no callbacks during the post-unmount wait. Mounted counts remain identical. `lifecycle-connected.log`, `artifacts/lifecycle.json`. Full unit/type/lint/build/browser gates pass. WP-4 remains separate. |
| 8 — attribution, localization and measured JS | Korean/English reset and attribution, TradingView link, local NOTICE/LICENSE URLs and successful served content; accepted WP-2 pinned-license comparison. Actual Canvas forced-colors palette changes, not only HTML CSS. Production chunk and timing measurements below. |

### Lifecycle interpretation

The probe uses WeakRef targets/callbacks and distinguishes connected/global listeners from detached-node listeners. Detached counts after successive removals were **48, 55, 103, 151, 199**; these anonymous/bound element-local library handlers are attached to disconnected nodes. There are no remaining operational listeners or observers. Instrumentation retains disposed chart records, and this is **not proof of heap collection**. No library/product internals were changed to unregister collectible node-local handlers.

### Visual observations

The complete price/volume surface and right-axis canvases fit at all five sizes; HTML details and controls are reachable without crop. Native time-axis labels at the plot's extreme edges can be partially clipped inside their own Canvas, as visible in the screenshots; the selected date label and HTML date remain complete. This is disclosed for WP-4 visual review, distinct from the repaired outer-layout clipping. No assertion was relaxed to accept an overflowing Canvas or compressed host.

## Screenshot and artifact index

Final chart screenshots:

| Viewport | Artifact |
| --- | --- |
| 375×800 | [price/volume chart](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/stock-beta-price-chart-375x800.png) |
| 640×360 / 200% equivalent | [price/volume chart](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/stock-beta-price-chart-640x360-200-percent-equivalent.png) |
| 768×1024 | [price/volume chart](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/stock-beta-price-chart-768x1024.png) |
| 1280×720 | [price/volume chart](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/stock-beta-price-chart-1280x720.png) |
| 1440×900 | [price/volume chart](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/stock-beta-price-chart-1440x900.png) |

Additional evidence:

- [Forced-colors Canvas](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-1d86f-pplies-Canvas-forced-colors-chromium/stock-beta-chart-forced-colors.png), [mobile long press](/tmp/wp3-final-qa/combined-final/stock-beta-provider-free-S-6e24d-ts-usable-with-coarse-input-chromium/stock-beta-mobile-long-press.png), [production chart](/tmp/wp3-final-qa/production-price-chart.png).
- [Import failure](/tmp/wp3-lightweight-harness/artifacts/price-chart-import-failure.png), [no-JS SSR](/tmp/wp3-lightweight-harness/artifacts/price-chart-ssr-no-js.png), [Strict Mode remount](/tmp/wp3-lightweight-harness/artifacts/price-chart-strictmode-remount.png), [SMA20 gap](/tmp/wp3-lightweight-harness/artifacts/price-chart-sma_20-null-gap.png), [SMA60 gap](/tmp/wp3-lightweight-harness/artifacts/price-chart-sma_60-null-gap.png).
- [Lifecycle JSON](/tmp/wp3-lightweight-harness/artifacts/lifecycle.json), [gap pixels JSON](/tmp/wp3-lightweight-harness/artifacts/sma-gaps.json), [production measurements and physical chunk hashes](/tmp/wp3-final-qa/production-measurements.json).
- Playwright's existing line reporter does not persist JSON bodies attached to passing tests as separate files. Geometry/palette/mobile assertions are evidenced by the passing tests and saved PNGs; their in-memory attachment bodies are not represented as recoverable JSON files. Historical failed runs retain traces/videos under their own output directories.
- [Accepted pre-repair recovery](/tmp/wp3-final-qa/recovery-accepted.md), [offline preparation](/tmp/lightweight-charts-wp3-preparation.md), [WP-2 repair report](/tmp/lightweight-charts-wp2-report.md). Pre-repair layout/crosshair failures remain historical evidence and are not presented as final failures.

## Production bytes and observed timings

Measurement command: `node /tmp/wp3-final-qa/production-probe.mjs` — **exit 0**, log `production-probe.log`. Browser Chromium **149.0.7827.55**, three fresh contexts against a warmed local production server, viewport 1280×900, provider-free guards. It measures actual requested URLs and physical `.next/static/chunks/*.js` files; gzip is level 9 per file, excluding response headers. These are not estimates of compressed network transfer.

| Scope | Raw bytes | Gzip bytes |
| --- | ---: | ---: |
| Dynamic library `3ebegv39ml6g9.js` (actual pinned 5.2.1 marker/exports, requested by route) | **192,287** | **61,262** |
| Chunk containing PriceChart/route code `2pt1viywk6t73.js` (not chart-only) | 114,461 | 31,580 |
| Those two chunks combined | 306,748 | 92,842 |
| All JS requested by the observed Stock Beta route, 13 unique chunks | **1,166,596** | **331,381** |
| All built browser JS, 21 chunks | 1,415,845 | 412,887 |

Library SHA-256: `a077dff75a1db3b1409dddf2a010c303d1fa1fc277e021b2fd8e03706946d95d`. Route-containing chunk SHA-256: `b34153fff9b45a5aeeca1cbae7f0ffbed03ed16360367bb62850df5b1bb7a4fe`.

| Fresh context | HTML root seen (ms) | Renderer ready (ms) | First nonblank plot paint (ms) |
| --- | ---: | ---: | ---: |
| 1 | 53.6 | 305.7 | 307.5 |
| 2 | 38.1 | 239.9 | 245.5 |
| 3 | 31.2 | 208.2 | 246.4 |

Times are from browser navigation, including route/data/hydration work, not isolated library construction. Sample 1 pointer selection took **37.3ms** and ArrowLeft **5.6ms** from an armed browser timestamp to the selected-index DOM mutation, including automation/input scheduling. Wheel/drag/reset assertions passed with no additional chart request and no page errors. These few observations are not a benchmark. **No comparable prechange build exists; no measured delta or performance gain is claimed.**

## Exact run history on the frozen product

The rows below preserve setup failures and overlapping reruns. For E2E rows, `E` expands to this exact prefix:

```sh
PLAYWRIGHT_BASE_URL=http://127.0.0.1:33017 SYNTHETIC_API_ORIGIN=http://127.0.0.1:38182 npm --prefix apps/web run test:e2e -- tests/e2e/stock-beta.spec.ts --project=chromium --workers=1
```

All logs are under `/tmp/wp3-final-qa/`. “NR” means Playwright **did not run**, not an intentional feature skip. Only the final 12 intraday tests are intentional skips.

| Log | Exact command / suffix after `E` | Exit | Passed / failed / skipped or NR; reason |
| --- | --- | ---: | --- |
| `unit.log` | `npm --prefix apps/web test` in sandbox | 1 | 435 / 1 / 0; 56 files passed, 1 failed. Existing DOM test's Chromium launch hit sandbox `Operation not permitted`; native rerun below resolves it. |
| `unit-native.log` | `npm --prefix apps/web test` with native browser permission | 0 | 436 / 0 / 0, 57 files. |
| `typecheck.log`, `typecheck-final.log`, `typecheck-accepted.log` | Three separate `npm --prefix apps/web run typecheck` invocations | 0 each | Full checks passed; no test counts. |
| `lint.log`, `lint-final.log`, `lint-accepted.log` | Three separate `npm --prefix apps/web run lint` invocations | 0 each | 267 files each, 4 existing warnings / 1 info each. |
| `focused-1.log` | `--output=/tmp/wp3-final-qa/focused-1 -g 'updates the tooltip and live summary\|uses aligned Canvas panes\|preserves the existing authenticated loading shell\|keeps a single renderer\|reflows at mobile\|keeps touch targets'` | 1 | 0 / 6 / 0; initial HMR guard prevented reliable hydration, plus no-JS/gesture setup assumptions. |
| `pointer-isolated.log` | `--output=/tmp/wp3-final-qa/pointer-isolated -g 'updates the tooltip and live summary'` | 1 | 0 / 1 / 0; hit within 0.5px of the rounded right-axis boundary. |
| `pointer-final.log` | `--output=/tmp/wp3-final-qa/pointer-final -g 'updates the tooltip and live summary'` | 0 | 1 / 0 / 0; actual Canvas hit verified with 1.5px latest-edge inset. |
| `focused-2.log` | `--max-failures=1 --output=/tmp/wp3-final-qa/focused-2 -g 'uses aligned Canvas panes\|preserves the existing authenticated loading shell\|keeps a single renderer\|reflows at mobile\|keeps touch targets'` | 1 | 3 / 1 / 1 NR; minimal scroll left a 1.34px surface crop. Center-scroll setup corrected; strict geometry retained. |
| `focused-3.log` | `--max-failures=1 --output=/tmp/wp3-final-qa/focused-3 -g 'reflows at mobile\|keeps touch targets'` | 1 | 1 / 1 / 0; all five viewport checks passed, ordinary short tap did not establish native inspection. |
| `mobile-1.log` | `--output=/tmp/wp3-final-qa/mobile-1 -g 'keeps touch targets'` | 0 | 1 / 0 / 0, 8.3s; full mobile case, preserved after interruption. |
| `combined.log` | Final two-file command above, with output `/tmp/wp3-final-qa/combined` | 1 | 33 / 2 / 11 NR; off/owner-only precondition and swipe during long-press tracking. |
| `mobile-state-final.log` | `--output=/tmp/wp3-final-qa/mobile-state-final -g 'keeps touch targets'` | 0 | 1 / 0 / 0, 8.5s; scroll before inspection plus real tap exit. |
| `combined-final.log` | Exact final two-file command above | 0 | **34 / 0 / 12 skipped**; approved off-mode intraday gate. |
| `import-failure.log` | `node /tmp/wp3-lightweight-harness/import-failure.mjs` | 1 | Script assertion initially classified local Vite HMR as unexpected; chart error checks already reached correct state. |
| `import-failure-final.log` | Same exact command | 0 | Isolated import failure and static fallback passed. |
| `ssr-check.log`, `ssr-check-final.log` | Two separate `node /tmp/wp3-lightweight-harness/ssr-check.mjs` invocations | 1 each | Temporary Vite SSR inlined CommonJS React incorrectly (`module is not defined`), HTTP 500. |
| `ssr-check-node.log` | Same exact command, direct Node/React SSR loader | 0 | JS-disabled actual-component contract passed. |
| `lifecycle.log` | `node /tmp/wp3-lightweight-harness/lifecycle.mjs` | 1 | Base Canvas hover intercepted by the overlay; real overlay hit target corrected. |
| `lifecycle-final.log` | Same exact command | 1 | Instrumentation incorrectly treated detached-node handlers as surviving operational listeners. |
| `lifecycle-connected.log` | Same exact command, WeakRef/connected/global distinction | 0 | Five cycles, strict active/global/connected/delegate/observer cleanup passed. |
| `sma-gaps.log` | `node /tmp/wp3-lightweight-harness/sma-gap.mjs` | 0 | Two actual painted/disconnected null gaps; no page errors. |
| `build.log` | Exact production build command above | 0 | Production web build passed. |
| `production-probe.log` | `node /tmp/wp3-final-qa/production-probe.mjs` | 0 | Three contexts; actual bytes, paint, pointer/key and gestures passed. |

Diagnostic-only `node /tmp/wp3-final-qa/{inspect-boundary,inspect-clipping}.mjs` runs both exited 0 and saved `boundary-targets.json` and `clipping-1280x720.{json,png}`; they are not additional acceptance-test counts. Owned Biome formatting invocations exited 0; final two-file command from `apps/web` was `../../node_modules/.bin/biome check --write tests/e2e/stock-beta.spec.ts tests/e2e/stock-beta-intraday.spec.ts` (2 files, no remaining fixes). The approved temporary patch had a malformed hunk count (`git apply --check` exit 128); its exact four-line skip gate was applied directly, without changing the approved code. Two initial production ownership probes exited 1 because npm start's actual parent was the npm shell and npm's cwd was the repository root; exact observed chain/cwd was then matched before any production browser request. None was a product failure.

Earlier pre-repair invocations/counts remain verbatim in `recovery-accepted.md`; they are not silently replaced with current passes. That original recovery did not record every process exit code, so unknown historical exits are not reconstructed.

## Deviations and reasons

- Coordinator-approved off-mode skip gate preserves the standalone intraday `owner_only` suite; quote activation would violate this task's boundary.
- Blocking a Next entry chunk stopped hydration. Equivalent import-failure evidence uses the real component in a temporary harness and blocks only its library wrapper. Route error-removal coverage remains intact.
- The app's authenticated streaming/loading shell is pre-existing. The amended plan permits JS-disabled actual-component `renderToString` proof; no route/data-loader change was made. Temporary direct Node transpilation of only actual chart sources avoids the Vite/CommonJS SSR loader failure.
- Long-press is native tracking mode, exited on the next ordinary tap. Scroll is tested independently before inspection, and later gestures explicitly exit tracking. Movement assertions remain strict.
- Native browser/loopback permissions were used only for the specific authorized local checks after sandbox Chromium launch was proven blocked; no permission mode was relaxed.
- No further decomposition was needed.

## Owned processes, reproduction and cleanup

All owned listeners are **stopped**. Ownership records: `process-verified.json`, `dev-stop.json`, `production-process.json`, `process-cleanup.json`, and `server-session-results.json` in `/tmp/wp3-final-qa/`.

| Service | Owned PID(s), port, cwd | Final state |
| --- | --- | --- |
| Next dev | listener 141061, parent 141049; 33017; `apps/web`; session 72870 | SIGINT after exact matching, before build; both absent, port closed; session exited 0. |
| Synthetic API | 150719; 38182; repo root; session 93198 | SIGINT after exact matching; absent, port closed; session exited 0. |
| Final temporary harness | 288060; 33018; repo root; session 24522 | SIGINT after exact matching; absent, port closed; exit 130. Log `harness-server-node-ssr.log`. Earlier owned harness PIDs 266924/277131 were also stopped. |
| Production Next | listener 360883, shell 360882, npm 360870; 33017; listener/shell `apps/web`, npm repo root; session 60452 | SIGINT listener after exact matching; entire chain absent, port closed; exit 130. Log `production-server.log`. |
| Unrelated Next | 1780113; 33001 | Same listener before/after cleanup; untouched. |

Start/reproduction commands from the repo root, after confirming ports are free and recording each new PID/parent/cwd plus URL readiness:

```sh
SYNTHETIC_API_PORT=38182 node apps/web/tests/e2e/support/synthetic-api.mjs
API_INTERNAL_URL=http://127.0.0.1:38182 NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS=1 npm --prefix apps/web run dev -- --hostname 127.0.0.1 --port 33017
node /tmp/wp3-lightweight-harness/server.mjs
node /tmp/wp3-lightweight-harness/import-failure.mjs
node /tmp/wp3-lightweight-harness/ssr-check.mjs
node /tmp/wp3-lightweight-harness/lifecycle.mjs
node /tmp/wp3-lightweight-harness/sma-gap.mjs
# Stop only the newly owned dev server before the build above.
API_INTERNAL_URL=http://127.0.0.1:38182 NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS=1 NEXT_TELEMETRY_DISABLED=1 OWNER_INTRADAY_QUOTES_MODE=off npm --prefix apps/web run start -- --hostname 127.0.0.1 --port 33017
node /tmp/wp3-final-qa/production-probe.mjs
```

Stop only the newly recorded owned PID/session with SIGINT after rechecking its exact command/cwd; the historical PIDs above must not be blindly reused. `npm start` emitted the existing `output: standalone` warning recommending the generated standalone server. It nevertheless served the actual optimized build for these local measurements; release packaging/deployment was not validated. Browser probes close their contexts/browser in `finally`. No Playwright or verification process remains running. Original server session output is recorded in `server-session-results.json` (dev output tail is truncated); no separate original synthetic stdout logfile was recovered.

## Unresolved / follow-up

- **Unresolved product blockers: none observed on the repaired source.** Earlier outer sizing, false gutter and committed crosshair defects are repaired and have final evidence.
- Independent WP-4 diff/evidence review remains; include the disclosed native boundary time-tick clipping in visual review. No product change is proposed by this worker.

## Not found / unverified

- **Required tools, fixtures and evidence not found: none.** A comparable prechange production build is unavailable; no bundle/performance delta is measured.
- **Required WP-3 chart behavior unverified: none.** Scope limitations: the separate 12 `owner_only` intraday tests were skipped; the actual app route retains its no-JS loading shell; heap collection was not proved; existing 200% coverage is viewport equivalence, not browser UI zoom. WP-4 review is pending.

**Final worker state: idle for WP-4. No commit or deployment.**
