# WP-7 Independent SOURCE Review — Stock Beta Intraday Quotes

Date: 2026-09-10 (Asia/Seoul)  
Review kind: independent SOURCE review; no implementation, test execution, build, runtime, or deployment work  
Workspace: `wks_c8105f3859e0ad64`  
Branch: `feature/stock-beta-intraday-quotes-20260908`

## Pin and scope

- Baseline: `d1baf9da9b13fcb61649b1c26de56aed87a83418`
- Reviewed HEAD: `8ac8f445a2678f6601bef6b55dcaebba007c0766`
- Production-source equivalence pin: the only change from `7d76e49` through HEAD is the continuity-plan document, so production source is the same as `7d76e49`.
- The baseline is an ancestor of reviewed HEAD.
- Initial index: clean.
- Initial worktree: exactly two coordinator-owned, unstaged diagram edits:
  - `docs/diagrams/component_architecture.puml`, SHA-256 `f9ab39b7a4c608db63a775720934ed539e00a981a652cb4e39b8a65badb63b0c`
  - `docs/diagrams/runtime_deployment.puml`, SHA-256 `3730dd467cc3aeda95744d8a02aa61f3e94a5fa6424aa3ff83f63eb214ddfc92`
- Those diagrams were inspected as current source and were not modified or staged.
- The review covered the integrated `d1baf9d..8ac8f44` source diff and necessary callers. It did not reread the historical execution log or redo package mapping/planning.

## Verdict

**SOURCE_ACCEPT**

Historical verdict for the original pin. Additional independent review of `f587396` by
`37b54534-4389-4484-bda7-394983af4705` returned `REVIEW_REJECT` on 2026-09-10 for the
retained-price plus failure/pause status issue. Coordinator confirmed and corrected it on
2026-09-11. The final addendum records restored source and QA acceptance after re-review.

No substantiated source finding remains after bounded adjudication of the initially reported HTTP demand-mutation concern. No Critical, High, Medium, or Low source defect was identified in the reviewed scope.

This SOURCE_ACCEPT is limited to the inspected source. It is not an overall WP-6, WP-7, QA, or release acceptance; the independent render/runtime/Web QA gates listed below remain unresolved.

## Finding adjudication

### WITHDRAWN — previously reported Medium current-identity locking concern

Disposition: **withdrawn, not downgraded**. No implementation or additional concurrency test is required by this review finding. The earlier report silently strengthened the accepted F1 contract from a post-demand-lock validation requirement into commit-time identity stability.

Accepted requirement and source evidence:

- The frozen WP4-B-F1 brief requires identity validation after all blocking owner-mutex/demand-row acquisition, in the same transaction and before every replay/terminal result or mutation: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes.md:2038-2048`. It explicitly requires app-readable inputs and forbids `FOR SHARE/UPDATE` of membership under `app`: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes.md:2048-2050`.
- F1's deterministic barrier proof invalidates membership while HTTP waits on the owner mutex or demand row, then requires 404 and unchanged demand state after the request proceeds to validation: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes.md:2051-2058`. F1b expressly permits no stronger concurrency claim: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes.md:2151-2154`.
- The full contract freezes the demand-cap advisory mutex for every create, renew, and release, but says that mutex does not serialize policy administration or replace READY/current-admission checks: `docs/superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md:610-626`. It also states that 0053 does not write intraday demand rows: lines 622-623.
- The broader API contract requires derivation from READY/current admission and same-transaction recheck for an expired ACTIVE renewal, but does not require that identity remain current until demand commit/response: `docs/superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md:465-486`.
- The API adapter calls the current-identity repository seams directly: `crates/api-server/src/repos/owner_intraday_quotes.rs:28-51`.
- POST/renew begins a normal actor transaction, takes the demand-cap advisory lock, locks an existing demand row, and then calls `lock_ready_admission`: `crates/job-queue/src/owner_equity_v2/intraday.rs:863-905`. It may subsequently commit an exact replay, renew, or insert at `crates/job-queue/src/owner_equity_v2/intraday.rs:926-1053`.
- DELETE similarly locks the demand row, calls `lock_ready_admission`, then commits a release/replay: `crates/job-queue/src/owner_equity_v2/intraday.rs:1082-1175`.
- `lock_ready_admission` is the accepted app-readable current check: a plain `SELECT` that requires READY membership and rejects a newer admission, without the forbidden membership row lock: `crates/job-queue/src/owner_equity_v2/intraday.rs:2817-2857`.
- Demand mutations serialize on `hashtextextended("owner-intraday-demand-cap|{owner}", 0)`: `crates/job-queue/src/owner_equity_v2/intraday.rs:2711-2740`.
- The existing F1 source test places a real barrier before the admission check, commits disable, then observes 404 and unchanged state: `crates/api-server/tests/http_owner_intraday_quotes.rs:653-717`. Sequential disabled/newer-generation cases cover settled invalidation before invocation: `crates/api-server/tests/http_owner_intraday_quotes.rs:451-650`.

Why the earlier scenario is not a defect:

1. T1 acquires every required demand-side blocking lock and observes the exact identity as READY/current immediately before its result or mutation. That check is the accepted linearization point.
2. T2 disable or generation invalidation that starts or commits after that observation overlaps T1, so the history may linearize T1 before T2 even if T2 commits before T1 returns. Real-time linearizability does not force overlapping operations into response/commit order.
3. A subsequent non-overlapping API operation after T2 completes rejects the invalid identity. Future GET and producer selection/publication independently revalidate identity; 0053 disable intentionally does not rewrite demand rows. No third observable operation or accepted invariant was identified that makes the allowed T1-before-T2 ordering contradictory.
4. The earlier report offered only the stronger unaccepted rule “identity must remain current through demand commit/response.” The helper name and absence of a membership lock do not establish that rule, especially where F1 explicitly forbids that lock.

No non-overlap counterexample was found: if invalidation completes before T1 begins, or while T1 is still blocked before its admission check, T1 observes invalid state and returns 404. The source-only evidence therefore does not support a finding.

## Cross-layer source review results

### Shared credential boundary and KIS client

- All inspected production constructors use the shared `ProductionReadCoordination` snapshot/auth path when coordination is enabled, including owner backfill, action-range Raw collection, stock-price-beta Raw collection, and the worker. The intraday path does not create a second credential/token implementation.
- The coordination implementation stores the actual bearer token, verifies its durable record with a credential-derived HMAC, uses restrictive file handling, serializes token issuance and credentialed GET reservations, enforces per-channel spacing/budget/cooldown, and classifies 401/429 behavior.
- The intraday attempt has its own bounded transport timeout while still acquiring the shared coordination reservation. No source path was found that permits a credentialed reader to opt out when shared coordination is configured.
- API and Web source contain no quote-provider client, KIS credential, token, coordination-store, or provider-host path.

### Parser, cache, and provider contract

- The typed intraday parser requires the seven named fields, rejects duplicate/missing fields and invalid decimals/identity, and maps sign/halt state without accepting an undocumented response shape.
- The collector seam uses the exact read-only quote endpoint/TR pair and parses an already-guarded attempt in memory. It explicitly never writes Raw or provider response bodies: `data-pipelines/collectors/src/intraday_quotes.rs:1-6`; `parse_intraday_attempt` only parses or preserves typed failure reservation metadata at lines 297-317.
- The producer hands a successful parsed quote to `publish_success_in_window`, which writes the intraday DB cache rather than Raw: `crates/job-queue/src/owner_equity_v2/intraday_producer.rs:402-437` and `crates/job-queue/src/owner_equity_v2/intraday.rs:1914-1978`.
- No account identifier, order, balance, execution, correction/cancellation, or trading surface was found in the reviewed intraday integration.

### Demand, cache, RLS, producer, session, and budget

- Migration 0054 isolates demand, cache, and producer state; actor reads/mutations and worker fenced writes are role-scoped. The intraday quote path writes only its DB cache and never writes an EOD Raw/Curated table or changes EOD publication. Existing EOD collectors retain their separate immutable Raw validation/commit boundary.
- Cache visibility is identity/session/age based. A missing active demand is not used as a cache-visibility rejection; `NO_ACTIVE_DEMAND` governs producer demand state rather than deleting an otherwise valid cached quote from GET.
- Producer execution remains daemon-only, default-off, demand driven, fenced, and bounded. Membership/generation and producer ownership are independently rechecked at publication.
- The 36-hour calendar evidence age check is separate from exact same-KST-date and nonfuture session-window proof. Neither check substitutes for the other.
- Attempts consume the daily owner budget and share endpoint/channel coordination; retry classes remain bounded. The quote/cache flow never mutates EOD state.

### API and Web

- Routes and DTOs are explicit. Authentication/Owner role, CSRF for mutations, privacy collapse, RLS actor context, and cache-only GET behavior are present. Owner and Member are the actual roles; no admin HTTP role was invented.
- GET performs no demand/cache/provider mutation and no provider/token call. Repeated and denied GET paths are covered by source tests with database fingerprints/provider-call guards.
- The Web client uses only the same-origin API contract. Its coordinator serializes mutations, fences requests with abort/epoch state, retains valid quote display across nonterminal refresh states, and releases/clears across visibility, offline, unmount, membership change, and logout/authorization transitions.

### Default-off operations and diagrams

- `deploy/compose/compose.intraday.yml` remains an explicit, non-auto-loaded preparation overlay. Installed-release selectors are unchanged and activation remains separately gated.
- The overlay gives every enabled credentialed reader the shared writable coordination mount while API/runner consume only their required read-only session-window state. No API/Web credential route is introduced.
- The current coordinator-owned component and runtime diagrams reflect the separate KIS/EOD and intraday cache/API flows, default-off overlay, and evidence-bound structural edges. They remain unmodified and unstaged by this review.

## Production and test source coverage

Reviewed production source included:

- shared KIS configuration, token/coordination store, market-data client, and all production constructors/callers;
- typed intraday quote parser, provider-free in-memory capture seam, and hand-off to the intraday DB cache publisher (not Raw);
- migration 0054, RLS/grants, demand/cache/producer repositories, worker scheduling, session/calendar proof, budget/failure/publication/GC paths;
- API routes, DTO/error/privacy ordering, actor-scoped repository adapter, and GET read-only behavior;
- Web schemas, API client, state/coordinator lifecycle, widget/catalog/page integration;
- Compose preparation overlay, static operations checks, runbook, OpenAPI, and both current diagram sources.

Reviewed relevant test source included:

- coordination spacing, crash recovery, token reuse/rotation/permissions, budget, timeout, 401, and 429 cases;
- typed parser/collector fixtures and invalid-shape/value cases;
- migration/RLS, demand sequence/capacity/identity, lock contention, post-lock clocks, producer lease/fencing/publication, disable/generation invalidation, retry/fairness, and EOD-contention cases;
- API privacy/status/DTO/current-identity/cache/no-demand/calendar/clock and repeated-GET read-only cases;
- Web contract, client, coordinator, lifecycle, page/widget, and architecture-boundary cases.

Intentionally omitted or not established by this review:

- **No test, Cargo command, compiler, build, npm/install, Next/browser, Docker/Compose/DB, provider, external network, production, deployment, or activation command was run.** Source inspection is not substituted for executed tests.
- The previously accepted C3 132-test result and B1/B2 focused fixtures were treated as prior evidence; they were not rerun.
- Unrelated regression suites, ancillary test helpers not needed to trace a reviewed boundary, the historical 4,600-line execution log, and package remapping/planning were not reread end to end.
- No live credential, account, order, host clock/environment, `/opt`, or production state was inspected.

## Known gates and UNVERIFIED items (not new source findings)

- **UNVERIFIED — renderer/PNG:** local diagram rendering and regenerated PNG acceptance remain pending owner permission. The `.puml` sources were reviewed only.
- **UNVERIFIED — Compose engine:** actual engine merge/render validation of the explicit overlay has not been performed.
- **UNVERIFIED — Web toolchain/docs:** npm dependencies and installed Next documentation are absent; browser/Next verification has not been performed.
- **Known unrelated static-runner gate:** the top-level operations static command exits 1 on the unrelated post-backfill-health `mode 0775` condition after the Stock static call succeeds. This review neither repaired nor reclassified it as a Stock implementation finding.
- **UNVERIFIED — WP-8:** browser, accessibility, lifecycle timing, negative-path, and integrated runtime QA remain outstanding.

Therefore SOURCE_ACCEPT does not imply overall WP-6/WP-7/release ACCEPT. Render, Compose, Web, and WP-8 runtime gates remain separate.

## Read-only commands and results

- `pwd`; `git rev-parse HEAD`; `git branch --show-current`; `git merge-base --is-ancestor ...`; `git status --short`; `git diff --name-only`; `git diff --cached --name-only` — confirmed cwd, branch, ancestor relation, exact HEAD, empty index, and only the two diagram edits before this report.
- `sha256sum docs/diagrams/*.puml` — confirmed both supplied C2a hashes exactly.
- `git log`, `git show`, `git diff --stat`, `git diff --name-status`, and targeted `git diff` — established the integrated source delta and that `7d76e49..8ac8f44` changes only the continuity plan.
- `find`/`rg --files` for instruction and scoped file discovery; `wc -l`, `sed`, `nl`, and `rg` for targeted source, test, specification, runbook, plan, OpenAPI, Compose, and diagram inspection — read-only; no sibling repository content was reviewed. The bounded reconciliation additionally read only plan lines 2038-2070 and 2151-2154, contract lines 606-630, and the cited intraday parse/publish sources.
- No test-like or state-changing command was executed.

## Decomposition status

The independent SOURCE review and this bounded adjudication do **not** need further decomposition. No code, test, lock, schema, or runtime change was made or requested by the final finding disposition.

## Coordinator-recorded WP-8 follow-up — 2026-09-10

This addendum records subsequent independent adjudication; it does not change the historical
source-only scope or imply that the original reviewer executed tests.

- Read-only sol/high reviewer `975eaeff-a765-4ea7-aff4-814b3d943cfb` accepted the narrow
  serializable quote Server/Client boundary, real catalog/coordinator additions and repaired
  browser/fixture evidence. The later detail-width Medium was reproduced on an old build,
  fixed only by desktop/tablet full-span rules and covered by responsive containment checks.
  Its bounded two-file source review accepted that correction.
- The same reviewer returned `QA_EVIDENCE_ACCEPT`: root325/46 tests, OpenAPI88, two build
  exits0, focused10/10 twice, whole84/84, real1800.73-second endurance and all six final detail
  screenshots were inspected. No substantiated Medium+ evidence defect remained.
- Accepted CSS hash `129db3565b1eceddc78a6e1082b04925268b03696f33c8a0ae413b9652eb4a9e`,
  E2E hash `413f53887a290003312ee49f4b145ad0c1e416837cd120a99106516104745c76`, endurance hash
  `09f841c4a2b9452da1f346c22cc4ee51c5663f750d9d10a1fc459034c127789b`; coordinator rechecked all three.
- At review time final PID cleanup was not independently evidenced. Coordinator subsequently
  confirmed six recorded QA PIDs absent, ports33041/38191 free and no compiler/test processes
  at18:47 KST. The [final QA report](2026-09-08-stock-beta-intraday-quotes-qa.md) records exact
  checks, final mapping, process-RSS limits and retained evidence locations.
- Owner-approved local rendering, actual config-only Compose validation and Web toolchain QA
  close those earlier development gates. Existing collectors test-target Clippy and unrelated
  operations mode0775 static failures remain explicit. No actual provider traffic, release
  install, production activation, main merge or push is approved by this acceptance.

## Additional review correction — 2026-09-11

The fresh review on `f587396` identified a Medium: a production-shaped HTTP200 response can
retain a valid RECENT quote while carrying provider failure or `NO_ACTIVE_DEMAND`; the widget
incorrectly displayed validated cache. CLOSED wording was also duplicated. This was reproduced
before production edits:10 failed/33 passed in the two focused files.

The same independent sol/high reviewer `37b54534-4389-4484-bda7-394983af4705` subsequently
returned **ACCEPT, no findings** after reading the full eight-file Web correction and tracing
the API response contract. Seven failure reasons now show retained refresh failure, no demand
shows paused without hiding a valid quote, and market/semantic status text is deduplicated.
The 30-second timer preserves the reason; a later clean response clears it. Cache-age phase
and the existing null-quote retention/demand policies remain unchanged.

The reviewer inspected final focused44/44 and whole Vitest336/336 logs (exit0) and the old-code
red evidence in `/tmp/stock-beta-web-remediation.C78Afq/`, and ran read-only diff checks.
It did not execute tests or rescan the prior111-file branch, Rust/DB/ops or coordinator docs.
Source acceptance was followed by **QA_EVIDENCE_ACCEPT**, no findings or remaining source/QA
followup, from the same reviewer. It read both fresh build exits0, focused11/11 on each,
whole Web85/85, final typecheck/scoped Biome exits0, unchanged eight-file hashes and cleanup
records in `/tmp/stock-beta-final-web-qa-current.LUM1ei/`. It viewed the four retained-failure/
paused intermediate PNGs at original resolution, not every screenshot, trace or video.

The initial server-launch, inline-wrapper, port-assertion and wrong-cwd config failures are
preserved as harness errors; none is counted as a successful test or source regression.
For inline exit127, the reviewer relied on the provided failure record because a separate
retained command log was not found. Final cleanup aggregation initially lacked its own exit
marker; coordinator independently confirmed recorded PIDs absent, both ports free, `.next`
absent and both artifacts retained with an explicit exit0 at00:52:22UTC. The final QA report
records those exact checks and limitations. No further source fix was requested or made.
