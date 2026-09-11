# Stock Beta intraday quotes — WP-8 synthetic integration QA

Date: 2026-09-11 (final correction; earlier executions retained)

Branch: `feature/stock-beta-intraday-quotes-20260908`

Baseline: `6cfa1ac`

Authority: the full intraday contract spec, original WP-8 lines 408–442 and its final
development approval, and the accepted `SOURCE_ACCEPT` review.

## Verdict

`QA_ACCEPT` — 2026-09-11: the additional retained-quote status defect is reproduced, fixed
and independently accepted, including fresh-build browser evidence and final cleanup.
The temporary `QA_REOPENED` state is closed. Earlier failed and passing execution records
below remain historical evidence; no deployment or operational activation is implied.

### Additional review remediation

Reviewer `37b54534-4389-4484-bda7-394983af4705` returned `REVIEW_REJECT` for one Medium:
HTTP200 with a valid recent quote plus provider failure or `NO_ACTIVE_DEMAND` reason can
render as normal validated cache. Existing typed-failure browser fixtures omit the quote
and miss this production-shaped combination. CLOSED text is also duplicated. Coordinator
confirmed the API/coordinator/view/fixture source mismatch; no new provider or DB run is
needed to reproduce it. Bounded Web repair, regression execution and independent re-review
are now complete. Earlier QA/cleanup results are preserved below.

First regression execution against unchanged production code: **10 failed / 33 passed**
across two focused files. Seven failures cover reason loss at the 30-second timer; three
rendered-state failures cover HTTP200 retained provider failure, retained no-demand pause,
and duplicated CLOSED text. Evidence: `/tmp/stock-beta-web-remediation.C78Afq/red-focused.log`.
This is expected red evidence, not a new test-suite pass or an executed browser regression.

The bounded correction now passes focused44/44 and whole Vitest336/336 (46 files), both
with explicit exit0 in `coordinator-final-focused.log` and `coordinator-final-vitest.log` in
that directory. The original reviewer37 returned **source ACCEPT, no findings** after
reading all eight Web deltas and those logs. It did not rerun tests. Subsequent browser
execution, screenshot inspection and cleanup below restore development acceptance.

### Corrected execution — 2026-09-11

Browser evidence: `/tmp/stock-beta-final-web-qa-current.LUM1ei/`. Source is the eight-file
Web correction on `f587396`, not a Rust/API/DB change. All counts below are complete summaries,
not observed progress lines. The two new build artifacts remain under each `next-output/`.

| Check | Result | Log relative to browser evidence directory unless noted |
| --- | --- | --- |
| Focused unit regressions |44 passed, exit0|`/tmp/stock-beta-web-remediation.C78Afq/coordinator-final-focused.log`|
| Whole Vitest |336 passed,46 files, exit0|`/tmp/stock-beta-web-remediation.C78Afq/coordinator-final-vitest.log`|
| Fresh standalone builds |Both exit0|`build-1/build-1.log`, `build-2/build-2.log`|
| Focused Chromium on build1 |11/11, exit0|`build-1/build-1-focused.log`|
| Focused Chromium on build2 |11/11, exit0|`build-2/build-2-focused.log`|
| Whole Web Chromium on build2 |85/85, exit0|`build-2/build-2-whole-final.log`|
| Final Web typecheck / scoped Biome |Both exit0;8 files checked without fixes|`final-web-typecheck.log`, `final-scoped-biome.log`|
| Final provenance and cleanup |8 source hashes unchanged; distinct build IDs; QA resources stopped|`final-provenance-and-cleanup.log`, `coordinator-cleanup-confirmation.log`|

Successful page/JavaScript/synthetic-state preflights returned200; widget preflights showed
price101200.00, ready state and a receipt time, with zero guarded external/forbidden requests
or WebSocket attempts. Logs are `build-1/build-1-{page-js,widget}-preflight.log` and the
corresponding `build-2/build-2-{page-js,widget}-preflight-{focused,whole-final}.log` files.

The new focused test checks actual HTTP200 JSON with RECENT/non-null price and receipt time,
the retained failure/no-demand reason and exact visible status, then the next clean response
and normal status recovery. In both builds, coordinator viewed the two intermediate PNGs in
`focused-results-owner/stock-beta-intraday-provid-55062-nses-then-recovers-normally-chromium/`:
`stock-beta-retained-provider-timeout.png` and `stock-beta-retained-no-active-demand.png`.
The tested desktop captures are readable and contained; they do not establish every possible
status at every viewport. The eight-reason unit table separately covers the30-second timer.
The synthetic fixture stamps each returned quote: this is not evidence that a real broker or
database refresh preserved one timestamp across multiple failed polls.

Harness failures are retained rather than presented as application failures or hidden:
the first detached-server preflight failed to connect (exit7); an inline harness attempt
exited127; a free-port assertion while the synthetic fixture was active exited1; and the
first whole-Web command used a root-relative missing config and failed before tests started.
Corrected same-session server/test execution and the proper Web config yielded the passes
above without source weakening or another build. Lower-level sandbox/network-isolation cause
was not independently established. Earlier failed logs and runtime-cleanup records remain.

No Rust, DB, soak, OpenAPI or Compose suite was rerun for this Web-only correction. Their
earlier accepted evidence and known unrelated lint/static limitations below remain unchanged.
Both build/runtime test subprocesses explicitly set owner_only; operational default-off,
installed-release wiring, external providers, production activation, main merge and push
remain untouched.

Reviewer37 returned **QA_EVIDENCE_ACCEPT, no findings, no remaining source/QA followup**
after reading the complete browser logs, final type/format/provenance/cleanup records and
all four specified intermediate PNGs. It did not execute tests or inspect every trace/video.
It relied on the reported inline exit127 failure because a separate retained command log
was not found; that failed attempt is not used as successful evidence.

Coordinator additionally confirmed cleanup at00:52:21–22UTC (09:52KST), with explicit exit0:
recorded PIDs906051/917698/924094/930674/917664/924082/930662 absent, ports33041/38191 free,
no active `.next` or link, both build artifacts retained and `git diff --check` passing.
Post-suite available memory was7.9GiB. This is a resource snapshot, not a memory-leak or
kernel-wide OOM proof. The historical DB cleanup remains unchanged; no DB was recreated.

The initial browser/API/database QA ran sequentially and exposed the failures below.
This report retains those initial results; the remediation addendum records later runs.
The corrected evidence package and fresh passing builds are recorded in the final execution
section below. Independent reviewer `975eaeff-a765-4ea7-aff4-814b3d943cfb` returned
`QA_EVIDENCE_ACCEPT`, with no substantiated Medium+ evidence defect remaining. Coordinator
confirmed final source hashes and resource cleanup at 18:47 KST. Production activation is
not part of this acceptance. Initial failures and then-pending decisions below are history,
not the current verdict; they are retained rather than rewritten as green runs.

## Final corrected execution — 2026-09-10

Final evidence directory: `/tmp/stock-beta-final-web-qa-followup-ztkt27/`.

| Final check | Complete result | Evidence |
| --- | --- | --- |
| TypeScript / lint | Exit0 / exit0; 236 files, four existing warnings/one info, no errors | `root-typecheck.log`, `root-lint.log` |
| Full Vitest | Exit0; 46 files, **325 tests passed** | `root-test-325.log` |
| OpenAPI | Exit0; **88 operations**, generated types synchronized and typechecked | `root-openapi-check.log` |
| Two fresh standalone builds | Both exit0 | `build-1/next-build-1.log`, `build-2/next-build-2.log` |
| Focused Chromium, final CSS | **10/10 passed on each build**, exit0 | `build-1/focused-build-1.log`, `build-2/focused-build-2.log` |
| Whole Web Chromium | **84/84 passed**, exit0 | `build-2/whole-web-build-2.log` |
| Detail visual regression | Old build fails actual geometry assertion; final1280/900/390 pass and captures are readable | `regression/08-regression-geometry-pre-fix.log`; focused result PNGs |
| Focused Rust/API + actual endurance | Earlier **432 passed** plus new **1 passed/0 ignored**, real1800.73s | Initial evidence directory and endurance directory below |

Coordinator viewed the final build1 detail PNGs at all three widths and build2 desktop/mobile
PNGs; price, signed change and receipt time are now readable and contained. These use synthetic
market data and an owned test-subprocess `owner_only` flag, not operational activation. The
real-time soak observed test-process RSS only; its bounds/cleanup and baseline lint limitations
are recorded below. Test execution is not a claim of actual broker or production readiness.

### Final acceptance mapping

| Required behavior | Final evidence and boundary |
| --- | --- |
| Current price/sign/base/time beside unchanged EOD, dashboard and detail | First focused E2E; both final builds pass. Detail geometry/screenshots at 1280/900/390 are readable; price/time advance is a separate passing scenario. |
| Selection, late response, same-instrument disable/re-registration | Passing switch and 005930 re-registration E2Es; fresh membership UUID and matching-generation positive control prevent wrong-identity false positives. |
| Hidden/offline/reconnect/logout/unmount and multi-tab isolation | Passing lifecycle/multi-tab E2Es; renewed visible demand precedes SPA unmount. Browser counters are synthetic; real caps/fairness use the executed DB/scheduler suites below. |
| Closed/halted/stale/unknown/failure/invalid/zero/future/prior-session states | Passing typed-state and last-good-value E2Es, with distinct prior-KST-session and future-timestamp cases. Actual timeout/retry/clock/fencing semantics also have Rust/DB coverage. |
| Catalog insertion/removal/reorder/visibility | Four actual dashboard/detail render-helper tests within the 325-test run, plus responsive browser placement. No new customization UI or actual browser zoom is claimed. |
| Cache-only 1/10/100 reads and EOD isolation | Passing browser counts plus real app-role API GET tests with populated, UPDATE-sensitive fingerprints. Fixture counters alone are not proof of provider-call absence. |
| Thirty-minute bounds | 360 simulated Web ticks with bounded timers/requests, plus real1800.73-second DB producer soak with20 consumers, bounded rows/attempts and sampled process RSS. PostgreSQL RSS and mathematical leak absence are not claimed. |
| Provider isolation | Final focused browser HTTP/WebSocket guards and loopback synthetic API; lower layers use fake provider transports. No actual market-data/account/order request or production activation was executed. |

### Cleanup, provenance and remaining limits

- QA writer `e9d73ea1-4168-495c-b9ee-dfe30b946bad` completed and reported exact-PID cleanup.
  Coordinator independently checked PIDs `3680193`, `3680197`, `3680971`, `3680985`,
  `3694147`, `3694155`: none existed at18:47:26 KST. Ports33041/38191 had no listeners;
  the bounded Cargo/rustc/Next-build/Playwright-test process check returned no matches, and
  `apps/web/.next` was absent. No additional process was killed during this confirmation.
- Both final standalone trees, screenshots, traces and logs remain in the final evidence
  directory. Build IDs: `TeHwVKTYM0mZHxWsjiP1P` and `25ZUzbv7_NkfSwBoFMxrI`.
  Coordinator cleanup record: `coordinator-cleanup-confirmation.md` in that directory.
- The exact synthetic PostgreSQL project was removed after all DB tests finished; its tmpfs
  data is unrecoverable while logs remain. No production resource was removed.
- Existing collectors test-only `vec_init_then_push` strict all-target Clippy failure remains;
  its production library and the new endurance target pass strict Clippy. The unrelated
  operations static-runner mode0775 failure remains. This is not a globally lint-clean or
  production-release acceptance; Web lint retains four warnings and one info.
- Local Compose config-only validation (57 assertions) and paired local diagram rendering
  completed under development approval. Installed-release overlay selection, deployment,
  live-provider smoke and operational activation remain separately gated.
- Raw QA artifacts under `/tmp` are retained local evidence, not a durable external archive.
  This report records counts, source hashes, limits and evidence locations.

## Initial run history

The following four findings describe the **initial run**, not the current source. Items 1–2
have since been fixed and independently reviewed; item 4 has passing composition tests.
The addendum preserves intermediate pending states chronologically. The final mapping above
supersedes those states.

1. The production Stock Beta detail route fails in a fresh standalone Next build because
   `StockBetaDetail` passes a translation dictionary containing function values through a
   Server/Client boundary. This blocks the detail quote and detail catalog assertions and
   also reproduces in two existing `stock-beta.spec.ts` detail tests.
2. The existing `stock-beta-fixture.mjs` advances a newly added membership to `READY`
   without changing its generation from `0`, while the synthetic signal generation is `1`.
   The actual client identity matcher therefore correctly refuses the quote. The existing
   fixture was not changed.
3. The contract's 30-compressed-minute endurance/memory bound has no exact existing test in
   the repository. Scheduler, retention/GC, and 5,000-attempt bounds were executed, but no
  memory or 30-minute endurance claim was made in that initial run.
4. Initial coverage did not exercise insertion/removal/reorder/hide using the existing
   injectable catalog render helpers. The contract requires catalog composition, **not**
   new user-facing customization controls. Their absence is not a product defect.

No further decomposition is required for this QA package. The first two items are bounded
returns to production/fixture owners; the latter two are explicit coverage gaps.

## Owned implementation

In the initial QA writer's stage only the four WP-8-owned files changed; no production code, dependency, configuration,
existing test, plan, review, or diagram was changed.

- New `apps/web/tests/e2e/stock-beta-intraday.spec.ts` (603 lines after the final test-only
  cleanup correction).
- New `apps/web/tests/e2e/support/stock-beta-intraday-fixture.mjs` (353 lines).
- `apps/web/tests/e2e/support/synthetic-api.mjs`: reset/state/dispatch additions only, at
  lines 12–16, 134–142, and 201–213.
- This new QA evidence plan.

The synthetic fixture validates the actual route names and DTO shape, produces deterministic
signed prices/timestamps/statuses, supports bounded response delays and typed failures, tracks
demand identities/leases/releases/renewals and cache GETs, and exposes only counters/state to
the test. It never contacts a provider. The browser guard aborts every non-loopback HTTP(S)
request and asserts zero observed non-loopback, KIS/OpenDART, account/order, broker, or
browser-provider HTTP traffic. The initial guard did **not** observe or route WebSockets;
the initial zero-WebSocket claim was unsupported and is withdrawn pending the explicit guard.

## Environment and process controls

- `npm ci --ignore-scripts --no-audit --no-fund --prefer-offline` had already completed with
  324 packages; manifests and lockfile remained unchanged.
- Node `24.13.1`, npm `11.8.0`, Next `16.3.0`, and the already-cached pinned Playwright
  Chromium were used.
- The approved pinned local PostgreSQL QA endpoint was used on loopback port `55438`; the
  credential is intentionally omitted from this record. DB acceptance tests created and
  dropped only their own scratch databases. No Docker lifecycle command was run.
- Only loopback ports `33041` (standalone Web) and `38191` (synthetic API) were used after
  free-port checks. The standalone server was run from the nested output path
  `.next/standalone/apps/web/server.js`, with its static/public assets available.
- Two fresh standalone builds were made sequentially. Before each corrected focused replay,
  `/stock-beta` returned HTTP 200 and the discovered JavaScript asset
  `/_next/static/chunks/0jvhpaew_uadu.js` returned HTTP 200.
- Every server used for the replays was stopped by its recorded PID. Final verification
  showed both PIDs gone and both loopback ports free.
- Available memory was checked before build 1 (8.4 GiB), between builds (8.2 GiB), after
  browser runs (8.0 GiB), and finally (10 GiB). No current-QA-day kernel OOM entry was
  observed. The host does have historical 2026-09-09 OOM records outside this QA run; no
  current build or test process was killed by OOM.
- The initial restricted-namespace DB/listener failures were retried through the approved
  local-network path. No external provider, KIS, OpenDART, account, order, operational
  production database, or live profile was used.

## JavaScript checks and builds

All commands ran from the repository root unless stated otherwise.

| Command | Exit | Complete result |
| --- | ---: | --- |
| `npm run typecheck` | 0 | API placeholder check and Web `tsc --noEmit` passed. |
| `npm run lint` | 0 | 233 Web files checked; 4 pre-existing warnings, 0 errors. Warnings are the deprecated recommended config, two `document.cookie` notices, and two `!important` CSS notices. |
| `npm test` | 0 | Vitest: 44 files and 316 tests passed. |
| `npm run openapi:check --workspace @lagrange/api-server` | 0 | Spec in sync; 88 operations linted; generated TypeScript type-check clean. |
| `API_INTERNAL_URL=http://127.0.0.1:38191 NODE_OPTIONS=--max-old-space-size=4096 NEXT_TELEMETRY_DISABLED=1 npm run build --workspace @lagrange/web` (build 1) | 0 | Fresh production standalone build completed. |
| Same build command (build 2) | 0 | Second fresh production standalone build completed after resource/OOM check. |

Build logs and root command logs are retained under:
`/tmp/stock-beta-intraday-qa-E9y9Bl/` (`build-1.log`, `build-2.log`, `npm-typecheck.log`,
`npm-lint.log`, `npm-test.log`, and `npm-openapi-check.log`). The final post-correction
reruns are `npm-typecheck-final.log`, `npm-lint-final.log`, `npm-test-final.log`, and
`npm-openapi-check-final.log`.

## Browser execution

The focused command was run from `apps/web` with `--project=chromium --workers=1` and the
two loopback origins. The first required replay of each fresh build used the test package
before a test-only multitab cleanup assertion was corrected:

- build 1 initial run: exit 1, 8 tests, 4 passed and 4 failed;
- build 2 initial run: exit 1, 8 tests, 4 passed and 4 failed.

The initial multitab failure was in the new QA test: it hid the first of two deliberately
visible pages and then waited for zero leases while the second page still owned one. That
assertion was corrected only in the new spec to wait for one, then zero after the second
page was hidden. No production or existing fixture file was changed.

The final corrected replays used the same fresh build artifacts:

- build 1 corrected replay: exit 1, 8 tests, 5 passed and 3 failed;
- build 2 corrected replay: exit 1, 8 tests, 5 passed and 3 failed.

The exact final focused command was:

```text
PLAYWRIGHT_BASE_URL=http://127.0.0.1:33041 \
SYNTHETIC_API_ORIGIN=http://127.0.0.1:38191 \
npx --no-install playwright test tests/e2e/stock-beta-intraday.spec.ts \
  --project=chromium --workers=1 --output=<focused-evidence-dir>
```

The final whole Web run used build 2 and the corrected test source:

```text
PLAYWRIGHT_BASE_URL=http://127.0.0.1:33041 \
SYNTHETIC_API_ORIGIN=http://127.0.0.1:38191 \
npx --no-install playwright test --project=chromium --workers=1 \
  --output=/tmp/stock-beta-intraday-qa-E9y9Bl/whole-e2e-corrected-results
```

Whole Web result: exit 1, 82 tests, 77 passed and 5 failed. The five are the three final
intraday failures plus the two existing detail failures at `stock-beta.spec.ts:424` and
`:582`; all other existing Web tests passed. The final logs are
`focused-build-1-corrected.log`, `focused-build-2-corrected.log`, and
`whole-e2e-corrected.log`.

Representative retained evidence includes:

- dashboard screenshots in both corrected result trees at
  `stock-beta-intraday-provid-12ecc-art-on-dashboard-and-detail-chromium/stock-beta-intraday-dashboard.png`;
- late-switch screenshots at
  `stock-beta-intraday-provid-fb32d-esponse-after-B-is-selected-chromium/stock-beta-intraday-switch-b.png`;
- failure screenshots, videos, and traces for detail, READY identity, and catalog failures in
  both corrected result trees;
- existing Stock Beta 1280px screenshot in the corrected whole result tree.

Independent review found the initial dashboard/switch viewport captures ended above the quote
widget. They are retained as page captures, not accepted evidence of visible price/time. The
remediation must capture the widget itself after semantic price/time assertions.

## Rust/API execution

Every successful command used `CARGO_BUILD_JOBS=2 --locked --offline`. Every DB command also
used the pinned QA `DATABASE_URL` with `-- --test-threads=1`. All listed authoritative runs
had zero ignored tests.

| Command/binary | Exit | Result |
| --- | ---: | --- |
| `cargo test -p kis-client` | 0 | 216 passed across library and binaries: 133 library, 12 `intraday_attempt`, 16 `live_order_state`, 36 `read_coordination`, 14 `reconciliation`, 5 `transport_agreement`; 0 failed/ignored. |
| `cargo test -p market-data --test intraday_quotes` | 0 | 16 passed, 0 failed/ignored. |
| `cargo test -p collectors --test intraday_quotes` | 0 | 4 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_demand_identity` | 0 after approved local-network retry | 4 passed, 0 failed/ignored. The first restricted-namespace attempt was 4 setup failures from DB reachability, not test assertions. |
| `cargo test -p job-queue --test intraday_calendar_read_state` | 0 | 5 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_quotes` | 0 | 24 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_read_state` | 0 | 1 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_producer` | 0 | 6 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_producer_pipeline` | 0 | 10 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_producer_lifecycle` | 0 | 9 passed, 0 failed/ignored. |
| `cargo test -p job-queue --test intraday_producer_scheduling` | 0 | 5 passed, 0 failed/ignored. |
| `cargo test -p api-server --lib` | 0 after approved local-listener retry | 103 passed, 0 failed/ignored. The first restricted-namespace attempt had one listener-bind failure. |
| `cargo test -p api-server --test http_owner_intraday_quotes` | 0 | 5 passed, 0 failed/ignored. |
| `cargo test -p api-server --test http_owner_intraday_quote_cache` | 0 | 4 passed, 0 failed/ignored. |
| `cargo test -p api-server --test http_owner_equity_v2_chart` | 0 | 4 passed, 0 failed/ignored. |
| `cargo test -p api-server --test openapi_contract` | 0 | 16 passed, 0 failed/ignored. |

The authoritative passed focused Rust/API total is 432 tests. Complete logs are retained in
the evidence directory with the `rust-` prefix.

## Strict format and clippy

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo fmt --all -- --check` | 0 | Passed. |
| `cargo clippy -p kis-client --all-targets --locked --offline -- -D warnings` | 0 | Passed. |
| `cargo clippy -p market-data --all-targets --locked --offline -- -D warnings` | 0 | Passed. |
| `cargo clippy -p collectors --all-targets --locked --offline -- -D warnings` | 101 | One pre-existing, out-of-scope test-target lint at `data-pipelines/collectors/src/stock_price_beta_materialize.rs:611`: `vec_init_then_push`. |
| `cargo clippy -p collectors --lib --locked --offline -- -D warnings` | 0 | Production library target passed; the failure is confined to the existing test target. |
| `cargo clippy -p job-queue --all-targets --locked --offline -- -D warnings` | 0 | Passed. |
| `cargo clippy -p api-server --all-targets --locked --offline -- -D warnings` | 0 | Passed. |

The collectors lint was not patched because that file is outside WP-8 ownership.

## Initial WP-8 acceptance mapping (historical, superseded above)

| Acceptance case | Evidence | Status |
| --- | --- | --- |
| Selection and A→B switch; late A cannot replace B | `switches identity and discards a late A response after B is selected`; synthetic delay on A and quote/version assertions | PASS on both corrected builds. |
| Price, sign, percent, base, timestamp, and separate EOD chart | Dashboard portion of `renders the signed current quote beside an unchanged EOD chart on dashboard and detail`; 261 candles, 12120 chart observations, no extra chart request, signed `+1,200`/`+1.20%` and ISO timestamp | Dashboard PASS; detail portion BLOCKED by detail route failure. |
| Add, READY, disable, re-add | `adds a membership to READY, disables it, and adds another READY membership` | BLOCKED at the first added READY quote: existing fixture generation mismatch; disable/re-add steps are not falsely claimed. |
| Multi-client lifecycle, cache polling paths, release isolation, typed capacity UI | Initial multi-tab browser test, two cache paths and synthetic two-consumer cutoff | PASS only for these browser behaviors; NOT proof of real caps/dedup/fairness. Real 20-consumer/5-identity rules and scheduling are attributed to the DB/scheduler tests below. |
| Hidden, offline, reconnect, logout | `stops on hidden/offline state, reconnects with a new demand, and stops on logout notification` | PASS on both corrected builds. |
| Closed, halt, stale, unknown calendar/window, 429, typed timeout, 503, invalid quote, zero price | Initial typed scenario loop | PASS for typed rendering and initial invalid-value rejection, not a real transport timeout or preservation of a last good browser value. |
| Current KST day rejects prior-session quote; same-instrument price/time advance | Initial `date-rollover` actually supplied a future timestamp; initial quote remained unchanged | NOT VERIFIED by initial browser tests. Future-timestamp rejection is separate evidence, not rollover. |
| Old EOD chart remains separate from current quote | Dashboard chart assertions and `lifecycle_shared_eod_arbitration_then_quote_success_preserves_durable_eod_row`; API chart suite | PASS for dashboard/DB/API; detail browser display blocked. |
| Catalog placement, order, visibility, responsive rendering | Initial fixed catalog browser assertions | Partial evidence only. Inject the real quote entry through both existing render helpers; do not add customization controls. |
| Cache-only 1/10/100 GETs | Browser test `repeats cache-only app GETs 1, 10, and 100 times without demand mutations`; real API `owner_intraday_quote_cache_get_is_read_only_and_owner_scoped` calls `assert_get_batch` for 1/10/100 and fingerprints all operational/EOD rows | PASS; no demand/provider/token write was observed in these synthetic/real-role test seams. |
| Scheduler/readiness/retention/attempt bounds | Exact tests listed below | PASS at the tested bounds; no 30-minute/memory endurance claim. |
| Provider/network safety | Initial per-page HTTP route guard and request assertions | HTTP-only evidence; WebSocket observation is pending. Synthetic counters do not prove a real server has zero provider calls or no writes. Rust fixture transports and DB fingerprints cover those lower layers. |

## Exact scheduler, retention, and attempt evidence

The following existing tests were executed and are the source of truth for semantics that the
browser cannot prove:

- `five_identities_run_two_sorted_rounds_and_duplicate_consumer_adds_no_turn`
- `one_identity_dispatches_again_on_the_next_due_cycle`
- `persisted_nine_second_retry_after_blocks_a_new_cycle_until_expiry`
- `halted_identity_waits_sixty_seconds_then_reprobes_after_real_demand_renewals`
- `demand_sequences_replay_expiry_release_scope_and_gc_are_typed`
- `fenced_success_failure_read_gate_and_gc_preserve_last_good`
- `budget_exhaustion_is_durable_and_next_kst_day_can_start_again`
- `shared_intraday_retries_at_most_three_actual_get_attempts`
- `five_thousand_intraday_reservations_exhaust_one_kst_day_and_next_day_resets`
- `lifecycle_daemon_automatically_dispatches_on_real_five_second_cadence_and_shutdowns`
- `lifecycle_inflight_shutdown_discards_publication_and_retains_real_debt`
- `lifecycle_generation_change_and_disable_discard_late_real_results`
- `lifecycle_shared_eod_arbitration_then_quote_success_preserves_durable_eod_row`

These establish the five-second cadence, 15-second renewal/30-second lease behavior,
identity/consumer caps, retry and attempt accounting, 5,000-attempt daily fence, next-KST-day
reset, halt reprobe, latest-only/fenced cache behavior, logical/physical retention behavior,
late-result fencing, and EOD isolation. These initial tests did not measure 30 minutes of
rows plus process memory. The added endurance test is recorded separately below.

## Remediation history — 2026-09-10 (completed)

The bullets retain the sequence of failures, repairs and intermediate pending states.
The final execution table, mapping and cleanup record above are authoritative.

- Coordinator fixed the actual detail Server/Client boundary by forwarding only quote inputs
  and string-valued copy through `CurrentQuoteClient`. The full function-valued dictionary
  stays outside that boundary. The initial generation mismatch was reproduced by a failing
  fixture test, then fixed. Narrow Web typecheck and 29 widget/fixture/architecture tests passed.
- One new standalone build and real page/JS preflight passed. Retained evidence:
  `/tmp/stock-beta-intraday-followup-oF0qdm/`. Existing Stock Beta **29/29 passed**;
  intraday **7/8 passed**, with the remaining test enumerating detail DOM immediately after
  navigation before waiting for destination readiness. Both owned servers were stopped and
  the fresh output retained. This is not the required final two-build replay.
- Independent read-only review confirmed the initial 432 Rust/API and browser counts, and
  identified the test/claim gaps corrected above. Same-instrument re-add must use a fresh
  membership ID as the actual API does; its first admitted generation may remain 1. Adding
  another instrument is not that test. Original browser/fixture writer is repairing these gaps.
- Coordinator's real catalog insertion/removal/reorder/hidden-placement tests and injected-clock
  30-minute scheduled-work test passed: **two files, 32 tests**. The latter exercises 360
  five-second ticks, 361 GETs, 121 demand create/renew calls, one consumer, one in-flight GET,
  constant pending timer count, and unmount with zero timers/one release and no later requests.
  These are component composition and simulated-time bounds, **not process-memory evidence**.
- Independent reviewer 975eaeff accepted the three quote production files and, separately,
  the two catalog/coordinator test changes. Its one Low type-hardening suggestion was applied:
  the client boundary accepts only the five serializable hook inputs, not injected clocks/clients.
  Subsequent TypeScript and scoped formatting passed. The in-flight metric covers GETs only;
  SSR catalog tests do not claim mounted-effect execution.
- The repaired browser evidence received bounded independent source acceptance from 975eaeff:
  same-instrument re-add with fresh identity; reactivated demand before SPA unmount; distinct
  price/time progression and prior-session cases; last-good value and exact timestamp retention;
  widget-element screenshots; and explicit WebSocket guard/observation. A matching-identity
  pre-disable 200 control prevents the old-identity 404 unit probe from passing for a wrong
  instrument. Final TypeScript/scoped formatting and three focused Vitest files passed
  (**36 tests**, 13:57 KST). This is not yet a repaired-browser execution pass.
- A test-only actual-DB producer soak uses a real 1,800-second observation with RSS samples,
  because the producer does not inject PostgreSQL wall-clock time. Original test source received
  independent acceptance. Coordinator corrected scoped lint issues and added safe aggregate
  progress output; subsequent compile, strict target Clippy and Rust-2024 formatting passed.
  Frozen source SHA-256: `09f841c4a2b9452da1f346c22cc4ee51c5663f750d9d10a1fc459034c127789b`.
  The first execution failed at setup because sandbox loopback was denied (0 elapsed seconds,
  no scratch DB). Approved local-loopback execution then **passed: 1 test, 0 failed, 0 ignored,
  1,800.73 seconds**, command exit 0 at 14:23:23 KST. Actual counters: 1,767 producer cycles,
  354 attempts/354 successes, 179 renewal batches/3,580 renewal operations for 20 consumers.
  RSS had 181 samples: post-warmup baseline 11,868 KiB, maximum 11,900 KiB, final 11,768 KiB
  (observed growth 32 KiB against a 32 MiB bound). This measures the test process, not PostgreSQL
  RSS or a mathematical absence of leaks. Final demand/cache/producer rows were bounded at
  20/1/1; all demands released, then three idle cycles added no requests. EOD/publication count
  invariants passed; stronger update-sensitive fingerprints come from the separate API tests.
  Source hash stayed unchanged. Evidence: `/tmp/stock-beta-intraday-endurance-MIDSCL/`,
  `intraday-endurance-approved-loopback-1800s.log`. No clock or production change.
- At this checkpoint final Web checks and two fresh build browser replays were running with
  e9d73ea1. Repaired browser execution was pending; no `QA_ACCEPT` was issued then.
- Frozen-source final root checks passed, each with complete exit 0: TypeScript; lint
  (236 files, four existing warnings/one info, no errors); **46 Vitest files/325 tests**;
  OpenAPI **88 operations**, synchronized generated types/typecheck. Evidence:
  `/tmp/stock-beta-final-web-qa-3iPZ43/01-root-typecheck.log` through
  `04-api-openapi-check.log`. Fresh standalone/browser runs are the remaining execution gate.
- Final-build first replay initially ran with the required synthetic Web runtime flag omitted:
  `OWNER_INTRADAY_QUOTES_MODE=owner_only`. It completed **1 pass/9 fail** with the default-off
  widget absent. Coordinator confirmed the server-only exact getter and the recorded launch
  command, stopped only that worker to prevent the same incorrect setup repeating, confirmed
  idle and resumed it with the corrected test-subprocess instruction. Both fresh builds had
  already succeeded; reuse them rather than rebuilding for a runtime flag. This is a QA setup
  error (including coordinator brief omission), not permission to change the default-off product.
  Earlier temporary preflight quoting/PID-lifetime and Playwright cwd resolution errors are
  separately retained. No source or operational setting was changed by these procedural fixes.
- Corrected owner-only QA runtime: focused **10/10 passed on each of the two fresh builds**,
  then whole Web **84/84 passed** (complete exit 0). Coordinator checked logs 26, 30 and 31 in
  `/tmp/stock-beta-final-web-qa-3iPZ43/`, and directly viewed dashboard/detail/advancing widget
  captures. Dashboard and advancing price/time were readable, but the detail capture was only
  about 81 pixels wide and unreadable. Semantic success therefore does **not** close visual QA:
  detail's twelve-column CSS grid lacks a full-width span for the newly catalogued quote.
  Independent bounded diagnosis and responsive geometry regression are pending; no final
  acceptance is issued from the 84 passing semantic tests alone.
- Bounded independent diagnosis confirmed that Medium. Coordinator applied only two full-span
  metadata selectors in `detail.module.css`, before existing named rules, and added responsive
  geometry/containment assertions to the first intraday E2E scenario. The two-file source
  correction received independent ACCEPT; TypeScript and scoped Biome passed. New regression
  **failed as intended on the old build**, specifically at `expectReadableDetailQuote`, exit 1.
  Actual old geometry: 1280 viewport quote80.91/board1048px (7.72%); 900 viewport62.75/830px
  (7.56%); 390 viewport378/378px (100%). Desktop/tablet price and timestamp overflowed, mobile
  was already correct. Evidence `/tmp/stock-beta-final-web-qa-followup-ztkt27/regression/`,
  logs08 (red assertion) and10 (successful geometry diagnostic). Temporary missing-output-dir
  and ambiguous diagnostic-link errors were retained separately, not counted as defect proof.
  Both CSS-corrected fresh builds then passed, and the recorded focused replay passed
  **10/10 on each build** (exit 0). The first direct 10/10 run lacked a retained command-exit
  record and was rerun against the same frozen build solely to retain that record; no rebuild
  or source change. Coordinator viewed corrected1280/900/390 detail captures from build1 and
  desktop/mobile from build2: price, signed change and receipt are readable and contained.
  CSS SHA-256 `129db3565b1eceddc78a6e1082b04925268b03696f33c8a0ae413b9652eb4a9e`;
  E2E SHA-256 `413f53887a290003312ee49f4b145ad0c1e416837cd120a99106516104745c76`.
  The final whole-Web replay and root checks subsequently passed as recorded above.
- After all DB executions completed, coordinator confirmed the retained QA container's exact
  project, pinned image, tmpfs-only storage, loopback port and zero other client connections.
  Historical synthetic scratch DBs remained from earlier work. Removed only project
  `lagrange-intraday-qa-20260908` with its known QA Compose file; its one container and network
  were removed successfully. No permanent volume/production resource was touched. Tmpfs data
  is not recoverable; all execution logs remain. Web replays use only the synthetic Node API.

## Initial findings returned without patching (subsequently remediated)

### Detail route serialization failure

The fresh standalone server logs repeatedly report Next's
`Functions cannot be passed directly to Client Components` error. The concrete path is
`apps/web/components/stock-beta/stock-beta-detail.tsx:42–49`, where `copy: t` places the
server dictionary into the client view model. The dictionary includes function-valued entries
such as `requestFailure` and `staleChartMessage`. The new assertions fail at
`stock-beta-intraday.spec.ts:255` and `:577`; the existing suite fails at
`stock-beta.spec.ts:456` and `:596`. This is a production-code ownership return, not a QA
fixture workaround.

### Added-membership generation mismatch

The existing fixture creates an added resource at
`apps/web/tests/e2e/support/stock-beta-fixture.mjs:409–412` using `REQUESTED` generation 0.
Its `advancePendingMemberships` path at `:91–107` changes lifecycle/coverage but never updates
generation, while the fixture's `signal` function emits generation 1. The actual client
matcher rejects that identity, so the new test fails at `stock-beta-intraday.spec.ts:323`.
The fixture also retains the disabled-instrument set on disable at `:438–448`, which must be
considered when the re-add path is repaired. The existing fixture was deliberately left
unchanged per the task boundary.

## Evidence index

All retained execution logs, standalone build trees, HTML/JavaScript preflight captures,
screenshots, videos, and Playwright traces are under:

`/tmp/stock-beta-intraday-qa-E9y9Bl/`

Important result trees:

- `focused-build-1-corrected-results/`
- `focused-build-2-corrected-results/`
- `whole-e2e-corrected-results/`
- `build-1-next/` and `build-2-next/`

The initial writer created no commit. Coordinator integrates the reviewed fixes and QA files
together with this report on the feature branch; no main merge or push is part of QA.
