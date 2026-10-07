# Stock Beta Lightweight Charts — WP-4 independent review

**Verdict: ACCEPT for coordinator acceptance of the bounded migration. Blocking findings: none.**

Review completed 2026-10-08 KST against the 2026-10-07 frozen handoff. This is a source/diff and existing-evidence review, with fresh integrity, whitespace, license-content and physical-bundle checks. I did not independently rerun the browser suites, full unit suite, typecheck, lint or build. No fixes, commits, deployment, production access or delegation were performed. Further decomposition is not needed.

## Pinned review state

- Workspace: `/data/worktrees/3puw275b/itchy-bulldog`.
- Baseline HEAD: `27ca8cf9f3c032daff32fb679f53ee54a78fdc96`.
- Manifest: `/tmp/lightweight-charts-review-manifest.json`; SHA256 `abef217ee140588ce7acfbfe425adcde5a1f620ef7cc1a10cd9843bfbb49cad8`.
- Frozen tracked diff: `/tmp/lightweight-charts-review-tracked.diff`; SHA256 `2cd229dd548badf438b7491a0703c917ae8735ca1bb91675444a9b4fbe5dd6ef`.
- At review start, independently verified HEAD, manifest hash, every one of the 19 file hashes and byte lengths, live tracked-diff hash, and byte equality between live/frozen tracked diffs: **PASS**. Actual new adapter, adapter test, LICENSE, NOTICE and QA files were read from the checkout.
- At review end, repeated the same complete verification: **PASS; drift none**. The plan and this review are the manifest's intentional exclusions.

## Blocking findings

**None.** No source/evidence contradiction requires a product fix before bounded acceptance. The earlier WP-2 full-typecheck failure is historical: the final owned E2E typing and WP-3 full-typecheck log supersede it. The separate owner-only intraday suite is explicitly outside the default-off chart acceptance run.

## Review conclusions and evidence

### Data and source-index contract

`lightweight-data.ts:48–61,112–181` validates actual calendar dates and strictly increasing dates among renderable bars, returning typed empty series/maps for invalid, duplicate or reversed dates. It reuses the unchanged `geometry.ts:142–165` numeric/OHLC/nonnegative-volume/SMA validity predicate; a numerically invalid observation remains excluded before date validation, as specified. There is no sorting, deduplication, interpolation, rounding or SMA recalculation. Candle OHLC and histogram volume are copied directly; both date maps preserve the caller's original source index.

`lightweight-data.ts:75–104` breaks SMA runs at nulls and excluded source indices. `price-chart.tsx:515–547` creates/reuses a distinct LineSeries for every run and removes surplus series, rather than relying on whitespace to break a line. Adapter tests cover 261 observations, zero volume/flat bars, leading/interior gaps, excluded numeric bars, date failures, maps and frozen inputs. The actual-component SMA probe source and JSON confirm two real series per tested SMA: SMA20 pixels **428 before / 0 across / 480 after**, SMA60 **195 / 0 / 220**. I visually inspected both gap screenshots; the lines are visibly disconnected.

### Canvas, selection and viewport

`price-chart.tsx:300–327,870–873,897–962` puts direction-separated candles in pane 0 and direction-separated volumes in pane 1 of one chart/time scale. Nonmatching direction series carry whitespace at the same dates; data values remain unchanged. Hollow down candles, filled up candles, flat color, solid SMA20 and dashed SMA60 follow the approved rendering simplification.

The latest valid source index is the default (`213–234,679–688`). Both callback aliases receive the original bar/index (`736–767`). Controlled requests leave the committed HTML selection intact; repeated hovering over the same date is deduplicated. Keyboard navigation uses the existing geometry helper; rejected controlled requests navigate from the committed index, and Home/End reveal even an already-selected offscreen boundary (`97–112,775–790`). Custom labels/formatted aliases and `formatters ?? format` precedence remain (`156–210,673–675`); raw strings never drive price calculations. Date formatting uses UTC and the validated daily date, with safe locale fallback.

I inspected `controlled.json`, `uncontrolled.json`, `fitted-data.json`, the WP-2 wrapper and focused probe source. A real native hover requested **136** while committed summary and actual final crosshair stayed **260**. Ten same-date events produced one request; offscreen committed selections hid the crosshair without moving the range. Home/End and external controlled prop changes reveal the selected point. Equal arrays and locale/copy changes preserved **60..95**, with **10 setData / 1 fitContent** unchanged. The wrapper forwards native events and reads the actual private crosshair for this pinned version; it does not fabricate a programmatic crosshair event. Its subscription wrapper is not cleanup evidence; the separate WP-3 lifecycle wrapper preserves unsubscribe identity.

`datasetKeys` and `applyChartData` (`280–294,561–583`) distinguish data changes from equal-valued arrays and display changes. Fitting occurs on initialization/new data and explicit reset, with logical range **-1..263** for 261 observations. Pointer selection preserves the viewport because the hovered observation is already visible; keyboard/external committed selection may reveal an offscreen point without refitting. Source-only scale redraws do not request a selection (`875–895`). The first/latest pixel probe confirms both boundary candles paint on desktop and mobile. There is no chart fetch in the renderer. Existing route tests and production probe verify wheel zoom, drag, reset and touch interactions without an additional chart request.

### SSR, cancellation, failures and cleanup

All library value imports are inside the mount effect (`793–835`); module-level imports are types only. SSR and the first client render share the same loading state, shell, selected values, legend, instructions and disabled reset. The actual-component Node SSR loader imports real chart sources, refuses unrelated transpilation and checks that the browser library was not loaded. Its no-JS browser artifact shows complete latest OHLCV and attribution. This proves the component contract; it does not claim that the existing app route renders data before hydration.

Async initialization checks cancellation after import; cleanup sets cancellation before disposal (`834–835,1055–1062`). Disposal disconnects ResizeObserver/MutationObserver, removes the exact media-query and crosshair handlers, cancels the pending resize frame, removes the chart and clears the host (`982–1002`). Data errors dispose the runtime; error rendering removes selected prices/details and disables reset (`1078–1086,1157–1192`). Empty/invalid adapter output clears every candle/volume series and removes SMA series through the ordinary data-update path. Existing PriceTab integrity/unavailable/error branches remove the component (`profile-tab-registry.ts:102–113`) and preserve same-context loading behavior.

The import-failure harness blocks only the dynamic library wrapper after a successful initial document, allowing the real component to mount: final error state has **0 Canvas / 0 tooltip**, hidden old values and retained static help/legend/attribution. I inspected its source, log and screenshot. Route error-removal tests cover unavailable/integrity/missing/forbidden and a later failed range request.

The StrictMode lifecycle script and JSON show five removals with **0 active charts / 0 Canvas / 0 connected-global listeners / 0 ResizeObserver targets**, stable remount counts **1 / 11 / 62 / 12**, zero disposed native crosshair delegates and no callbacks during the post-unmount wait. Source cleanup also disconnects the MutationObserver; the probe's target count specifically measures ResizeObservers. Detached-node handlers are not classified as operational/global leaks. The wrapper deliberately retains disposed records, so this is not heap-GC proof.

### Accessibility, responsive layout, touch and repaired geometry

`price-chart.tsx:1127–1247` retains a named focusable HTML region, keyboard shortcuts/instructions, polite atomic selected summary, explicit OHLCV details, five-entry legend/direction text and a separate reset button. Canvas is hidden from assistive technology. Localized new copy is optional with defaults; Korean/English wiring is present. Focus outlines and non-color direction markers remain in HTML; Canvas theme options respond to forced colors (`358–468,1035–1046`). The forced-colors screenshot shows actual changed Canvas colors, consistent with the palette test, not merely changed HTML.

`dashboard.module.css:1111–1132,1224–1228` applies natural sizing/overflow only to the Price tab and its signal-profile containers. Other widget rules are unchanged. `price-chart.module.css:32–86` gives the Canvas a full-height surface and resets minimum width, padding, border and table spacing beneath the chart host only. Global product table rules remain untouched. Source and the strict geometry tests independently explain why the former compressed surface and false left-axis gutter are repaired.

I visually inspected all five final surface screenshots: **375×800, 640×360, 768×1024, 1280×720, 1440×900**. Both plots and right axes fit; no recurrence of the outer crop/false gutter appears. I also inspected repaired desktop/mobile route screenshots and the final mobile long-press screenshot, which shows complete HTML values, directions, legend and attribution. Final E2E geometry checks require all inner canvases/table/axes to fit, empty left cells to be ≤1px, aligned panes, real Canvas hit targets, reachable HTML elements after scrolling, focus and no horizontal page overflow.

The mobile test uses force-bearing CDP touches, timed moves and a real Canvas hit: chart vertical swipe changes a scroll signature, a separate outside-widget swipe changes page/main scrolling with nested offsets unchanged, 600ms long press inspects a real bar, and an ordinary tap exits native tracking before pan/pinch. Pan/pinch change the date selected at the same plot fraction and match raw response OHLCV. This is meaningful Chromium touch evidence; it is not physical-device/Safari coverage.

### Dependencies, attribution and architecture

The complete package/lock diff adds exactly **lightweight-charts 5.2.1** and its required **fancy-canvas 2.1.0** dependency, with no wrapper/plugin or unrelated version change. Fresh byte comparison confirms local `LICENSE.txt` equals the installed Lightweight Charts LICENSE. The two-line local NOTICE and TradingView link are displayed verbatim in the renderer, including its U+0441 copyright character and **2025** year; the different **2026** distribution header is not used to invent a NOTICE year. The installed README requires visible NOTICE attribution/link, and final E2E verifies successful local LICENSE/NOTICE serving. The npm distribution contains no NOTICE file: independent upstream re-fetch was not performed, so upstream NOTICE provenance relies on the accepted pinned WP-2 comparison; local content/display were independently inspected.

The diff changes internal web renderer/dependency/test/assets only. Workspace/Cargo boundaries, Rust/Python spawn, web/API contracts/controllers, Compose, nginx and DB remain unchanged. I inspected the architecture files' component/runtime scope and changed-path list. No drawn structural edge changes; both `.puml` files and PNGs are correctly untouched. No diagram rendering or external renderer was invoked.

### Test observability and measurements

SVG element-count assertions were replaced with adapter/response count/value checks, actual Canvas paint, pane/axis geometry and native input effects. Existing geometry helpers/tests remain. Range payload counts **24 / 67 / 130 / 261**, boundary values, snapshot pinning, delayed-response rejection, prior-data retention during loading and error clearing remain covered. No skipped chart acceptance case was added. The only skip gate is the approved `intradayQuotesMode === "off"` beforeAll; its owner-only assertion and the unrelated intraday test bodies remain. The intraday chart assertion changes to Canvas paint/exact values as required by the migration.

The route's SMA-null test alone proves fixture nulls plus general paint, not a disconnected painted line; the separate inspected pixel harness supplies that missing observability. Temporary harnesses are actual-component probes, not mocked implementations. They remain `/tmp` evidence and are not permanent repository regression tests.

I read final unit/browser/type/lint/build logs and the final component-harness logs. They support WP-3's reported **57 files / 436 passed / 0 failed / 0 skipped**, full typecheck/lint success, **34 E2E passed / 0 failed / 12 owner-only intraday skips**, and production web build success. The four lint warnings and one config information message are outside the changed implementation lines. This review accepts those recovered runs as evidence; it does not represent them as new WP-4 executions.

I inspected the production measurement script and JSON and freshly checked all **21 physical JS chunks** against saved raw size, SHA256 and Node gzip-level-9 size, including aggregate totals: **PASS**. Library **192,287 raw / 61,262 gzip bytes**; observed route **1,166,596 / 331,381**. Requested URLs distinguish route chunks from other build chunks. Timing samples include navigation/data/hydration and automation scheduling. No prechange delta, isolated initialization benchmark or performance improvement is claimed.

## Nonblocking findings and limitations

| Severity / location | Trigger and evidence | Impact / disposition |
| --- | --- | --- |
| Low — `price-chart.tsx:438–443,550–558` native time-axis labels | At fitted extremes, all five final surface screenshots show a partly clipped native date tick inside its own Canvas; interior ticks and the selected crosshair date remain readable. | Cosmetic reduction in boundary tick readability. Full selected date remains in HTML, first/latest bars are selectable, plots/right axes fit and values are intact. Does not meet the plan's severe clipping threshold; no pixel-perfect SVG requirement. Optional follow-up: evaluate shorter native tick labels or supported edge-label alignment while preserving full HTML date/custom formatters and fitted boundary bars. No fix required for this verdict. |
| Low — temporary verification harnesses | Import-failure, controlled-crosshair, StrictMode and painted SMA-gap probes live under `/tmp`, with paths documented in QA. | Good review evidence, but their availability is local and these focused guarantees are not all continuously enforced by committed E2E. Optional later retention/automation within separately authorized test ownership; not a reason to remove or weaken current checks. |

## Unverified gates and scope limits

- Required bounded-review gate missing: **none**. Fresh browser/suite reruns were not required and no concrete new failure hypothesis justified them.
- The 12 separate owner-only intraday cases were not run in off mode; runtime activation was not authorized or changed.
- Existing “200%” coverage is **640×360 viewport equivalence**, not browser UI zoom proof. Physical mobile devices and non-Chromium engines were not established by these artifacts.
- App-route no-JS remains its pre-existing Loading workspace shell; actual PriceChart SSR text was verified separately. Hydration consistency is assessed from deterministic render/effect source and recovered browser error checks, not a new dedicated WP-4 hydration comparison.
- Heap collection and forced GC are unproved; lifecycle instrumentation measures operational resources, and retains disposed records.
- No comparable prechange build exists. No bundle delta or speedup can be inferred. Release packaging/deployment and live/provider behavior are outside this review.
- External upstream NOTICE was not independently re-fetched; local LICENSE bytes, local NOTICE text/display and serving evidence were inspected as described above.

## Checks actually executed by WP-4

1. `pwd`, `git rev-parse HEAD`, `git status --short`, manifest read; Python SHA256/byte-length verification of all 19 manifest entries and frozen/live diff equality at start and end — **PASS**.
2. `git diff --check` at start and end — **exit 0**. `git diff --stat`, scoped `git diff`, baseline `git show`, `rg`, numbered source reads and artifact/script/log reads — source/evidence assessment above; no product/test changes.
3. Installed Next lazy-loading guide and installed Lightweight Charts package/types/README/LICENSE inspection — browser-only dynamic import and local attribution assessment. Comparing an installed NOTICE failed because the npm package omits it; no missing repository asset.
4. Python local LICENSE/content assertions — **PASS**. An initial Python gzip recomputation differed from Node's gzip output; this was a probe-method mismatch, not chunk/hash drift. Repeating with the original Node `gzipSync(..., { level: 9 })` method — **exit 0**, all 21 physical chunks, hashes/sizes and route/all totals match.
5. `view_image` on the five final viewport surfaces, repaired 1280×720/375×800 route images, final mobile long press, forced colors, actual-component SSR/import failure and both SMA gaps — visually assessed above. JSON lifecycle/selection/data/measurement artifacts were parsed and checked; no independent browser execution.
6. Required memory-hook finish used the exact supplied envelope with `no_durable_change`. Sandbox receipt access failed, then the exact narrowly approved invocation reported **SKIPPED**, finished and inactive. No durable memory or Basic Memory write.

Read-only discovery also encountered inaccessible unrelated `/tmp` directories and absent npm NOTICE/fancy-canvas license filenames; these did not block the named handoff artifacts. No live network request or server start was performed, and unrelated port 33001 was untouched.

## Handoff

- Changed repository file: **this review only**, entire new document. Product/test/dependency/license/QA changes by WP-4: **none**.
- Specification deviations by WP-4: **none**. Existing approved layout, SSR-harness and off-mode clarifications were followed.
- Unresolved product blockers/fix requests: **none**. Coordinator final acceptance remains the handoff step; optional limitations above do not require fixes for bounded acceptance.
- Required files/evidence not found: **none**. Unverified claims are explicitly listed above; no missing requirement was filled by guess.
