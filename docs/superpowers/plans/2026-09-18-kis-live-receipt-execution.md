Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Approved KIS receipt and thirty-instrument recovery execution

## Goal and boundaries

Execute the owner's approved plan: restore the existing V1 thirty-instrument list through genuine V2 registration/admission, preserve EOD, and prove two new KIS receipts through cache/API/Owner browser followed by successful EOD publication using the same calendar evidence. Synthetic QA is prerequisite evidence, never live acceptance.

Coordinator workspace: `/data/worktrees/3puw275b/hot-chipmunk` (Paseo `wks_eb660d10b6c4c447`). Integration baseline `57f34879eb93ac0f8723d64b701dfb3ee19302e9` merges main `9723234` and recovery `f164d28`; main and production remain unchanged by this merge.

Applicable instructions: `/home/l1nnx/.codex/AGENTS.md`, repository `AGENTS.md`, Web `AGENTS.md`/`CLAUDE.md`, and `/home/l1nnx/.agents/skills/paseo-delegate{,-plan}/SKILL.md`. Read any additional path instructions before edits. Provider/model rules select luna max for specified implementation, sol high for independent conclusions; escalate one dimension at a time after observed failure. No worker delegates further. All worker reports explicitly state changed files/lines, deviations/reasons, checks/results, unresolved follow-up, and unverified items (`none` when empty).

Authorization: implementation, normal thirty-instrument onboarding, corrected immutable release, read-only KIS collection and bounded real receipt QA are approved. Accounts/orders/account identifiers/WebSockets, new provider surfaces, invented Owner sessions/READY rows, quota resets, forced data deletion and fabricated session evidence remain prohibited. Coordinator controls production transition gates. Worker scopes below narrow execution sequencing without requiring the owner to reapprove the approved task.

Verified pre-execution facts: installed current `74b39fa`; quotes off; research worker restarting exit 2; calendar latest 2026-08-31; V2 memberships/events/generations/admissions/snapshots all zero even with superuser RLS bypass; Owner and policy each one. API V2 entitlement pins are present but exact DB match is zero (reference match one, hash match zero). V1 approved registry/list has thirty instruments, 261 sessions and 7,830 bars; its UNREGISTERED metadata does not prohibit the separate sealed V1 read endpoint. V2 UI cutover did not populate memberships. These are dated observations, revalidate before mutation.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | Cross-crate immutable provenance and exactly-once calendar acquisition; concrete design below | high | historical manifest byte drift, duplicate broker capture, sink lineage conflict, two failed fixes |
| WP-2 | intermediate | Normal authenticated API onboarding with deterministic fixed-list input and fake API tests | high | authentication workaround needed, existing API cannot express recovery, scope overlap |
| WP-3 | hard | Production curation failure and entitlement provenance diagnosis; conclusions determine safe repair | medium | evidence cannot identify source artifact or failure; do not infer or mutate around it |
| WP-4 | hard | Independent safety/compatibility review across the integrated candidate | high | undocumented wire/schema change, missing actual-sink regression, optimistic live claim |
| WP-5 | intermediate | Existing Rust/Web/ops checks plus diagram maintenance and disposable DB integration | high | environment cannot isolate DB, resource pressure, repeat test failures |
| WP-6 | hard | Exact-image deployment, state-preserving operational repair and real Owner/provider acceptance | medium | absent Owner session, missing current-day official evidence, OOM/health gate or window failure |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | Calendar reuse without breaking EOD | market-data/collector calendar ingestion, provenance, normalization, publication and focused tests | merged baseline | codex/gpt-5.6-luna, max | implementation and regression evidence | compatibility/fake-reader tests and disposable actual-sink tests |
| WP-2 | 1 | intermediate | Safe thirty-instrument onboarding and receipt QA tooling | new `scripts/qa/owner-equity-v2-live-acceptance*` files and focused tests only | merged baseline | codex/gpt-5.6-luna, max | genuine-session browser/API workflow; plan mode default | provider-free fake Owner/API tests |
| WP-3 | 1 | hard | Determine exact runtime/entitlement repair | read-only production diagnostics; report only under dedicated `/tmp` | dated facts | codex/gpt-5.6-sol, high | sanitized cause and concrete repair instructions | original evidence hash matching; typed failure provenance |
| WP-4 | 2 | hard | Accept integrated source independently | read-only whole candidate | WP-1, WP-2, WP-3 | codex/gpt-5.6-sol, high | ACCEPT or actionable findings | actual diff/source/tests trace |
| WP-5 | 3 | intermediate | Complete candidate QA and documentation | tests/artifacts; `docs/diagrams/*`, relevant runbooks, ADR-0008 clarification | WP-4 ACCEPT | codex/gpt-5.6-luna, max | check matrix, current diagrams and operational runbook | local rendering, DB integration, fresh browser QA |
| WP-6 | 4 | hard | Deploy and verify approved production result | pinned official release/backup/repair/onboarding operations only | WP-5 and coordinator release gate | codex/gpt-5.6-sol, high | thirty statuses, live receipt and EOD evidence | actual running image IDs, real receipts and same-day EOD publication |

All source workers use the coordinator workspace and disjoint ownership; they do not stage/commit/rebase. Coordinator alone commits integrated work. Further Rust constructor sites may receive only mechanical `copied_from: None` initialization by WP-1, never unrelated behavior edits. No concurrent Rust compilers: use one shared `/tmp/lagrange-kis-live-cargo.lock`, `CARGO_BUILD_JOBS=2`, and resource checks. No release image build until source QA completes.

## Worker briefs

### WP-1 — daily calendar reuse

Target: coordinator workspace, baseline above. Hard/high confidence; implementation choices are fixed here. Owned files are `crates/market-data/src/{contract,storage,ingest,normalize,publication,lib}.rs`, affected calendar provider adaptation only, `data-pipelines/collectors/src/{calendar_bootstrap,calendar_claim,pipeline,sink,worker,lib}.rs`, focused corresponding tests and actual-sink intraday regression. Mechanical constructors elsewhere are permitted solely to preserve compile compatibility. Do not change network allowlists, retry/token rules, Web, operations scripts, privileges, production state, or shared-build implementation.

1. Make the existing dedicated `kis-calendar` batch the one daily calendar source for both EOD-first and bootstrap-first acquisition. Preserve the durable day claim UUID and shared day lock. A claim without committed/reconciled exact source remains indeterminate; never recapture or reset it. Recover a committed source offline. Retain a narrow legacy complete-EOD source recovery path without fabricating a new claim.
2. Add optional `copied_from: Option<SourceFileReference>` to Raw envelope/file metadata. Reference fields: provider, market, batch UUID, file name, content hash and original retrieval instant. Omit absent fields when serialized so old immutable bytes stay unchanged. Verify committed source, exact date/mode/entitlement, file/hash/bytes/request metadata and nonfuture evidence; reject reference chains. Do not invent a new fetch timestamp.
3. Reused full EOD Raw still has all four kinds. Obtain only Bars, Reference and Corporate Actions from the provider, and incorporate the verified original Calendar response with its reference. No second holiday request. Calendar-only data must not masquerade as a complete EOD batch.
4. Reuse the deterministic standalone normalized `calendar.json` byte-for-byte, including its original lineage. Carry a narrowly validated reference in the complete normalized EOD bundle. Keep other files' ordinary EOD lineage and accept historical four-file bundles unchanged.
5. Carry internal calendar evidence UUID/hash/retrieval time separately through `PublicationBundle` and per-file evidence. Use those values for calendar history/projection; do not substitute the EOD batch/time. Strengthen duplicate matching to compare UUID/time as well as facts/hash. Existing DB columns suffice; do not add a migration without reporting a concrete necessity.
6. Test old manifest byte compatibility, both acquisition orders, concurrent attempts, exact source verification, tampering, interrupted claim/source/composite states, historical recovery, four-file shape, unchanged normalized calendar bytes, and actual PostgreSQL sink followed by intraday resolution. Morning publication then evening full publication must retain the same calendar UUID/hash/time; replay must be idempotent.

Use locked/offline Cargo where available, jobs=2 and the shared compiler lock. Disposable tests only; no production DB/provider calls. Report architecture contradictions instead of weakening validation. Required common report applies; include exact missing checks rather than claiming an unrun DB test.

### WP-2 — real Owner onboarding and receipt QA

Target: coordinator workspace. Intermediate/high confidence. Own only new `scripts/qa/owner-equity-v2-live-acceptance*` implementation/tests. No product UI, Rust, database migration, deployment or credentials changes. Inputs: existing checked-in V1 universe, V2 API/contracts, browser coordinator, installed read-only boundaries.

Implement an operator tool with explicit plan/apply modes, default plan, using Playwright attached to an explicitly supplied existing authenticated Owner browser context. Never create a test/forged session, read another user's profile, persist cookies/storage state, or use raw DB writes. If attachment/authentication is unavailable report a typed prerequisite. Reuse existing API CSRF/idempotency and same-origin browser requests. Read exactly the approved thirty IDs; reject duplicates/count drift and any foreign-origin/redirect path. Scope enrolment to one pilot first, then remaining IDs sequentially; inspect existing memberships before adding, preserve existing READY, never automatically retry permanent failures or reenable DISABLED records. Stable idempotency identity must survive tool restart without exposing session material.

Observe normal lifecycle to READY with bounded waits and report typed stops. A raw-data reuse/import route is not invented by this tool: it uses existing normal API/worker behavior. Receipt QA selects one genuine READY membership, records baseline version/time, observes at most 90 seconds, and requires two subsequent valid same-identity RECENT receipts and DOM/API agreement. Equal successive prices are valid. Stop demand in finally; allow existing 30-second expiry and verify quiescence. Do not dump provider/API response bodies, tokens or browser traces containing session material. Retain only whitelisted codes, counts, versions and timestamps; private Owner screenshots may show normalized UI quotes.

Fake the browser/API boundary in focused tests to cover idempotency, owner mismatch, auth loss, wrong origin, partial enrolment, permanent failure, version/time advance, identical prices, stale/mismatched receipt and cleanup. Do not launch real enrolment/provider traffic in WP-2. Common report applies.

### WP-3 — production diagnosis

Target: coordinator workspace; report under `/tmp/lagrange-kis-live-20260918/`. Hard/medium confidence. Read-only diagnostics only for now. No source edits, state repair, credential rotation, timers, provider requests or service restarts. Read applicable protected-state/official ops tooling before inspecting it. User has approved eventual repair but exact change must be grounded first.

Determine the original approved entitlement document and compute comparison booleans without printing protected references/hashes/body. Distinguish API/worker/config/DB mismatch and identify which value contradicts the actual approved document. Diagnose research-worker exit2 via bounded typed error metadata only, never free broker messages or full payload/log dumps. Revalidate daily timer/current image/schema/backups and available resources; report exact state-preserving official repair steps. Determine actual Owner-browser attachment availability without extracting credentials. Read-only Docker/SQL and narrow native escalations are within scope. Common report applies; unknown historical cause is not permission to overwrite it.

### WP-4 — independent acceptance

Target: same workspace after integrated commits from WP-1/2. Hard/high confidence, source/report read-only. Verify approved scope, byte/lineage compatibility, no duplicate holiday fetch or quota reset, exact source identity, all-source shared locking, authenticated-only thirty-instrument operations, and absence of fake acceptance. Inspect tests against real sink/provider contract rather than fixtures mirroring wrong assumptions. Report severity with file/line, acceptance decision and missing operational proof. Do not repair source directly. Common report applies.

### WP-5 — integrated QA and evidence maintenance

Target: coordinator workspace after review acceptance. Intermediate/high confidence. Run focused then required integration checks using disposable DB, no production/provider access. Validate merged build layout and ops selectors, Web types/tests/fresh production browser QA, Rust/calendar/intraday/manifest compatibility. Serialize compilers and stop on resource failure. Update structural `.puml` evidence and locally render PNGs; update runbook/ADR to document owner-approved thirty-ID onboarding and EOD reuse (no blind V1 artifact/READY copy). Preserve old historical evidence as dated history. Common report includes all skipped/unavailable tests and exact resources cleaned.

### WP-6 — official rollout and actual acceptance

Target: exact accepted commit and its immutable installed release, protected production paths only through official tools. Hard/medium confidence. No alternative hand-built activation, bypassed gates, synthetic Owner or expired-grant reuse. Inputs: WP-3 concrete repair, successful source QA, complete image manifest, current official day evidence, real Owner browser.

Follow mandatory sequential low-priority background image builds, batches of 2–3 as reporting units, jobs2/Compose1, resource/PSI/OOM/health checks and no leftover compilers. Validate backups/restoration and exact runtime image IDs; only then perform approved official install/migration/rollout and diagnosed entitlement/EOD repair. Correctly stop an existing runner when disabling or rolling back; changing current symlink alone is not runtime rollback.

Install actual same-day official KRX evidence with the protected installer, bootstrap/reuse the single KIS calendar, and activate the read-only quote path through the release wrapper. One genuine Owner pilot must reach READY before the remaining thirty-list onboarding proceeds. Two actual KIS quote receipts are a separate in-session acceptance criterion and do not block otherwise valid onboarding outside trading hours. Honor the approved 90-second receipt observation, shared token/attempt budget, typed failures and demand teardown. After market close verify full EOD publication with original calendar evidence and no second holiday call. No after-hours simulated acceptance. Return exact commit/images, membership count/status, receipt timestamps/version advancement, DOM match, cleanup and EOD result. If market/auth/host prerequisite prevents acceptance, identify it and retain monitoring without claiming completion.

## Coordinator gates

1. Prelaunch: skill available; profiles/models discovered; merged clean baseline; self-contained briefs and nonoverlapping ownership. Log owned agent IDs and notifications. Register a bounded fallback heartbeat if callback delivery is not established.
2. Wave1: inspect actual diffs/tests and diagnosis; resolve contradictions before dependent work. No source worker deploys. Integrate and commit accepted work; route repeated failures through documented model escalation.
3. Wave2/3: independent ACCEPT, meaningful disposable DB and browser evidence, current structural diagrams, exact candidate commit. Do not broaden reruns without a new failure/change.
4. Release: enforce official backup/image/config/resource/health gates. Approved plan covers scoped production operations; authentication or missing real-day evidence remains a concrete external prerequisite, not a reason to weaken the gates.
5. Final: every owned worker terminal and report recovered, no pending permissions, thirty-list outcome explicit, actual KIS receipts + API/DOM agreement + stop behavior + EOD same-source publication proven. Otherwise report the exact remaining work and maintain/hand off bounded monitoring. Never report synthetic tests as provider receipt.

## Diagnosed blocker and bounded dependency addition (WP-3 result)

WP-3 verified that current API/runner pins match the approved document, while DB rows retain its original hash; research-worker fails offline on cumulative price entries with two immutable entitlement references. Existing register/activate tooling cannot amend the active row. This blocks the already approved EOD repair, so WP-4 waits for the following bounded resolution rather than bypassing it.

- **WP-3A — hard, medium confidence, architecture decision, codex/gpt-6-astra xhigh.** Read-only source/Git/report analysis to choose the smallest correct recovery and amendment design. Compare explicit entitlement-separated dataset recovery against an audited transition; do not assume differing strings mean equivalent rights. Determine whether existing approved documents suffice or historical provenance requires Owner evidence. Report exact files/contracts/tests and a bounded implementation brief. No production or source mutations. Dependencies: WP-3 report; may read unfinished WP-1 only as tentative. Escalation: insufficient authoritative historical evidence is an external prerequisite, not permission to invent a transition.
- Implementation of the accepted design becomes **WP-3B**, classified and briefed only after the decision. It must run after WP-1 if worker.rs ownership overlaps, and before WP-4.

The original Owner approval requires pilot registration reaching READY before the remaining list. Two live receipts are a separate acceptance criterion; lack of a trading window alone must not add a new block to otherwise valid onboarding.

## Accepted WP-3A direction and implementation ownership

Coordinator inspected canonical Curated artifact paths, version allocation, exact source lookup, existing audit table and entitlement schema. Adopt exact-reference partitions under the existing canonical dataset ID/global generations, plus a narrowly audited amendment of the current-reference approval row. No cross-reference cumulative history or new transition table is authorized or needed for V2 onboarding. Consumer coverage and pinned old datasets remain enforced. Detailed constraints and tests are in the private `wp3a-report.md`; operational values stay outside Git.

- **WP-3B — hard, high confidence, specified recovery implementation, codex/gpt-5.6-luna max.** Own collector worker recovery and focused research_worker/candidate_catalog tests; narrow CurateStore exact-source-set reuse support only if needed. Wait for WP-1 handoff because worker.rs overlaps. Partition before latest-per-date, preserve per-partition no-mix checks, fix replay/version churn and anchor binding edge cases, maintain rights-window and transient-error semantics. Disposable actual-sink tests mandatory; no provider/production/source rewriting. Escalate after repeated failures or unresolved ledger identity.
- **WP-3C — intermediate, high confidence, specified ops implementation, codex/gpt-5.6-luna max.** Own provision-entitlement.sh, a new narrowly named entitlement-amend helper if needed, operator-attestation-self-test.sh, and new entitlement-amend-db-test.sh only. May run alongside WP-1/WP-2: no overlapping files. Implement pinned original/amended document validation and root-only transactionally audited CAS using existing data_entitlements/audit_logs, with read-only check and exact audit-backed idempotent replay. No production amendment or new privileges/tables. Validate actual SQL on disposable DB; missing test infrastructure is reported rather than skipped as success.

WP-4 now depends on WP-1, WP-2, WP-3B and WP-3C; WP-5/6 gates otherwise remain unchanged.

## WP-2 acceptance follow-up escalation

WP-2 first and follow-up reports were recovered but not accepted. Coordinator verified that follow-up checks freshness before unchanged-baseline waiting, quiescence is only two samples and permits zero polling interval, and the claimed rendered-DOM test builds querySelector maps instead of rendered markup. **WP-2R**, intermediate/high confidence, takes the same two QA files plus one new focused Web test only if actual React rendering needs the existing Vitest harness. Escalate model one step from luna to terra while keeping max effort; this is a repeated acceptance failure, not speculative escalation. No product changes or live browser/provider use. It must implement actual elapsed stability, stale baseline waiting, and a real rendered DOM test; report actual versus fake verification precisely. Original WP-2 remains idle, no overlapping writers.

WP-1 integration candidate is committed as 98bed9e after actual disposable sink/intraday tests. WP-3B may add a narrowly scoped read-only exact-ledger/binding inspection in candidate_sink.rs if needed, with no migration or privileges. It also owns strengthening the existing job-queue morning/evening sink test to use distinct valid retrieval times; this is a test gap found at coordinator integration, not production approval. WP-4 remains the independent whole-candidate acceptance gate.

## WP-4 rejection and bounded corrections

Independent review of candidate `7a20358` rejected advancement: the amendment SQL executes as `migration_owner`, but forced audit RLS has no matching policy; the minimal-schema test omitted this. Executing-release evidence is caller-asserted, replay identity varies by release, and actual EOD-first to standalone-bootstrap identifier handoff is incomplete. WP-5 and rollout remain gated.

- **WP-3CR — hard, high confidence, bounded permission/attestation/replay correction, codex/gpt-5.6-terra max.** Escalate the failed WP-3C implementation one model step, preserving effort. Own the four amendment scripts, a narrowly named installed-release verification helper if needed, and one new `0055` up/down migration only. Preserve FORCE RLS and append-only audit semantics; permit only the amendment event's SELECT/INSERT to the actual operator role, with no broad audit access, UPDATE/DELETE, role memberships or bypass. Validate existing immutable-release identity before DB access and keep amendment correlation stable across releases while retaining the original verified executor. Replace the minimal schema with complete repository migrations and real role bootstrap. Require apply/replay across two releases, contention, rollback, non-target preservation, and negative RLS/mismatch tests. No production action or change to historical rows, approvals, provider, Web, Rust, general DB helper or Compose wrapper.
- **WP-1R — intermediate, high confidence, specified existing-source handoff, codex/gpt-5.6-luna max.** Own standalone calendar bootstrap, its CLI, the calendar command block in the release wrapper, and focused calendar tests only. Add explicit reuse-existing mode resolving the authoritative claim/source identity under the same day lock; this mode never acquires a new broker response and fails if no valid committed/reconciled source exists. Keep explicit UUID mode strict, including mismatch rejection and indeterminate claim-only behavior. Both paths share identity/provenance validation and publication. Test the real standalone core/entry boundary after EOD-first and bootstrap-first, replay, mismatch, claim-only and contention with zero duplicate capture. No operational activation or changes to accepted EOD/recovery behavior.

These corrections may run concurrently with disjoint files; compiler/DB checks remain serialized under the shared lock. Each requires coordinator integration and repeat WP-4 acceptance. The narrow additive RLS migration is justified by F1's actual permission failure and supersedes only the earlier implementation package's no-migration constraint. Full details and severity are in `/tmp/lagrange-kis-live-20260918/wp4-report.md`; future documentation/diagram maintenance remains WP-5.

### Full-schema correction to F1

WP-3CR's actual full-migration test and coordinator inspection of `0039_auth_audit_outbox.up.sql:36-42` disproved WP-4 F1's claimed INSERT denial: existing `migration_owner` SELECT/INSERT policies already support the amendment. No additional permission is needed. The proposed new 0055 restrictive-policy migration is therefore withdrawn before integration; existing auth-audit policies and role behavior must remain unchanged. WP-3CR retains F2/F3 release binding and stable replay corrections plus the complete-migration test proving actual-role execution, append-only behavior, rollback and unchanged role/policy state. Tests must not impose a new denial on audit operations the existing migration role was already intentionally permitted to perform. Repeat WP-4 must correct F1 explicitly and review the remaining actual changes.

## WP-5 browser and operational evidence correction

WP-4 accepted source candidate `025c27d` for integrated QA only. WP-5 completed
fresh full-schema DB, recovery, build-layout and local Web checks, but full QA
acceptance remains withheld: the owner-enabled browser matrix has two stale
mode/empty-state assertions, and the new operational gate test duplicates the
recipe instead of executing the documented code. Coordinator inspected the
actual dashboard placement and test sources before deciding the correction.

**WP-5R — intermediate, high confidence, codex/gpt-5.6-terra max.** Escalate one
model step after repeated QA evidence gaps, preserving effort. Own the two Stock
Beta browser specs, a narrowly necessary local QA runner, the pre-amendment
runbook subsection and its provider-free self-test. Assert empty-state safety
(no identity/demand/quote before READY), retain the complete registration,
disable and new-generation lifecycle, and run explicit owner-only/off mode
browser checks against fresh standalone builds. Execute the actual documented
Docker gate in fixtures; revalidate release, daemon and image identity before
both amendment actions using the same effective Docker selection. No product,
provider, production, dependency, migration or permission changes. Root reviews
results and exact diffs before integration; deployment remains gated.
