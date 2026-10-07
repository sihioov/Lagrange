# Stock Beta chart layout follow-up QA — 2026-10-08

## Scope

Updated only the existing layout expectations in `apps/web/tests/stock-beta-dashboard.test.tsx` and `apps/web/tests/e2e/stock-beta.spec.ts`, plus this report. The in-progress Lightweight Charts migration was already present and remains untouched. The dashboard product files were read-only during this verification.

Test edits are at `stock-beta-dashboard.test.tsx:202–269,451–533` and `stock-beta.spec.ts:1971–2018,2082–2089`. They align catalog/read order and placements to the frozen specification, wait for the ready snapshot and painted chart before desktop geometry sampling, assert the profile spans the grid before the remaining widgets, and add the 1024×900 tablet viewport. The row/canvas/focus/overflow thresholds were not relaxed.

## Layout and browser evidence

The temporary Playwright probe passed for ready and no-snapshot states at mobile, tablet, and desktop sizes. It found the profile first in visual order, spanning the dashboard grid, with every other visible widget below it. Ready profile body `scrollHeight` equaled `clientHeight` at all four viewports; there was no scrollable overflow ancestor between the chart and `main`. The page and body had no horizontal overflow.

| Ready viewport | Profile top (page px) | Profile width | Chart surface screenshot width | Profile body scroll/client |
| --- | ---: | ---: | ---: | ---: |
| 375×800 | 228.6 | 363 | 337 | 1057 / 1057 |
| 1024×900 | 177.6 | 954 | 928 | 914 / 914 |
| 1280×720 | 177.6 | 1048 | 1022 | 837 / 837 |
| 1440×900 | 177.6 | 1208 | 1182 | 909 / 909 |

At 1280px, the profile spans grid x=224–1272 and starts at y=177.6. The chart surface screenshot is 1022px wide; the accepted WP-3 historical surface PNG was 496px wide at that viewport (2.06× wider). The earlier approximately 492px measurement described the inner Canvas host, not the screenshot. No prior top-coordinate measurement was available for comparison. The no-snapshot profile also spans the grid and remains first: its top is y=183.6 at 375px and y=132.6 at 1024, 1280, and 1440px. The measured empty-state widgets follow below it without overlap.

Each ready view had 11 Canvas elements; pixel sampling detected chart paint. Selecting `000002.KRX` through the ranked-signal control changed the selected instrument from `000001.KRX` and issued a chart request for `000002.KRX`. The probe recorded no external browser requests, unexpected WebSockets, or page errors.

Page and profile-section screenshots were saved under `/tmp/stock-beta-chart-layout-qa/`:

- `page-ready-375x800.png`, `page-ready-1024x900.png`, `page-ready-1280x720.png`, `page-ready-1440x900.png`
- `profile-ready-375x800.png`, `profile-ready-1024x900.png`, `profile-ready-1280x720.png`, `profile-ready-1440x900.png`
- `probe-results.json` contains the viewport measurements. The E2E chart-surface images are under `playwright/stock-beta-provider-free-S-a5838-0-zoom-equivalent-viewports-chromium/`; its 1280×720 image is 1022×291.

## Checks

- `npm --prefix apps/web test` — the sandbox attempt could not launch Chromium. The approved retry completed with 434/436 passing at the time; the dashboard reorder expectation I owned and one current-quote placement expectation failed. The owned dashboard file then passed its focused run: 15/15.
- `tests/stock-beta-intraday-widget.test.tsx` was outside my write scope. Its shared-worktree diff later changed the desktop current-quote expectation from row 6 to row 7, matching the frozen spec; its focused run passed 8/8. I did not author that edit. The full unit suite was not rerun after the focused verifications.
- Full provider-free `stock-beta.spec.ts` run — 33/34 passed, including the responsive case with 1024×900. The desktop geometry case initially measured the internal region before app readiness and saw a 4px offset. A focused rerun after switching to grid-widget bounds still saw that offset until the test waited for the snapshot and painted chart; the final focused geometry run then passed 1/1 with the strict `<2px` row alignment intact. The full 34-case suite was not rerun after that precondition fix.
- `npm --prefix apps/web run typecheck` — passed. Scoped Biome check — exit 0 with two `!important` warnings in `dashboard.module.css:2051,2073`. `git diff --check` — passed.
- Production build (`API_INTERNAL_URL=http://127.0.0.1:38182 NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS=1 NEXT_TELEMETRY_DISABLED=1 OWNER_INTRADAY_QUOTES_MODE=off npm --prefix apps/web run build`) — passed.

## Cleanup and remaining verification

The Next dev process (PID 502989, cwd `apps/web`) was stopped with Ctrl-C before the build. The final synthetic API process (PID 516615, cwd repository root) was stopped after the build. Recheck found no task-owned API, Next, or Chromium process and no listener on 33017 or 38182. Port 33001 remained owned by unrelated PID 1780113 and was not contacted or stopped.

No product defect was observed in the measured layout. The complete unit and E2E aggregates were not rerun after their focused fixes; their focused cases and the final browser probe passed. No other specification deviation or unverified layout condition was identified.

## Coordinator acceptance

- Accepted the two product-file changes, final test diff, parsed probe measurements and actual screenshots. The former migration's other 17 pinned files remain byte-for-byte unchanged; only the permitted dashboard CSS and E2E file differ within that prior manifest. Dashboard widget placement changes do not alter architectural dependency/runtime edges, so diagrams remain unchanged.
- The coordinator authored the current-quote row-7 expectation. After worker completion, the catalog-reorder test's custom row was set to 7 (default row 2) so it still proves that row overrides reach rendered CSS variables. `npm --prefix apps/web test -- tests/stock-beta-dashboard.test.tsx tests/stock-beta-intraday-widget.test.tsx` passed **23/23 in 2 files**; scoped Biome passed and `git diff --check` passed. This is an affected-file verification, not a fresh full-suite aggregate.
- The worker's final desktop geometry result is recorded as passed in `playwright-final-layout/.last-run.json`. The temporary probe reports no layout failures, unexpected requests/sockets or page errors. Its source and ready/no-snapshot results were independently read; the desktop/tablet Canvas images, mobile profile and 1440×900 page screenshot were visually inspected.
- Some desktop profile-element screenshots include a blank area below the app scrollport. They are not evidence that the entire profile is visible simultaneously. Use the page captures and Canvas crops together with the strict scroll/focus/geometry checks; the complete card is reachable through ordinary main/page scrolling.
- The worker is idle with no pending permissions. Independent listener inspection confirmed 33017, 38182 and 33018 closed; unrelated 33001 remains PID 1780113. Required layout evidence missing: none. Unresolved product blockers: none. Changes remain local; no commit, merge or deployment.
