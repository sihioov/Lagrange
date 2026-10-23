# R2 WP-2 — offline contract and recovery review

Status: bounded WP-2 source analysis complete; coordinator owns G1 acceptance. Source HEAD:
`cea7baa6298402db79d655140ad4c199f57f8697`, 2026-10-13. No runtime claim.

## Early U2 conclusion

New owner-beta requests approve the compiled v3 registry, not the old v2 example:
`crates/market-data/src/historical_price_only_approval.rs:249–265`. The fixed
range is **2016-08-29 through 2026-08-28**, 2,452 sessions / 26,972 ETF11 bars
(`crates/market-data/src/range_to_canonical_v3_price.rs:23–45`;
`configs/evidence/kis-historical-price-only-v3-approved-artifacts.json:1`).
The supported-as-of endpoint intersects exact ETF11 identities in those approved
bars and selects its maximum; enqueue rejects dates absent from the artifact
(`crates/api-server/src/http/owner_beta.rs:103–134,266–310,530–586`).

The inspected production producer is the authenticated request path to the
dedicated repository. Its durable replay key is client-key based, and enqueue
atomically writes a dedicated queue job plus owner-beta run
(`crates/api-server/src/http/owner_beta.rs:137–147`;
`crates/api-server/src/repos/owner_beta.rs:297–308,315–355,401–445`).
The owner-beta worker claims that existing exact job type, rather than producing
daily jobs (`crates/job-queue/src/owner_beta/runner.rs:157–174`). The generic
recommendation daily scheduler selects opted-in **PAPER** account bindings and
an independently pinned dataset (`crates/job-queue/src/recommendation/schedule.rs:44–54,90–118,135–155`);
it cannot stand in for owner-beta scheduling.

Therefore daily ETF EOD publication alone cannot provide market-current
owner-beta recommendations. A new input approval/update contract and unattended
trigger contract are required for the inspected route; neither is adopted here.
G1 must settle U2 before the affected WP-4 implementation and R2-04/08/09 acceptance.
Existing historical requests/replays remain a supported bounded route.

## Provenance and limits

This review supports R2-01/02 and the G1/G4 contracts for R2-03/04/05/07/08/09;
it completes none of those roadmap acceptance items. The execution request
authorizes this offline analysis, not lifecycle operations. Repository and global
AGENTS instructions, ROADMAP and the R2 common/WP-2 brief were read. Roadmap,
STATUS, plan and execution-record updates remain coordinator-owned.

Local `git diff --name-only 862fa954c16d28a617271fe6f613d34226e2d558 HEAD`
returned the nine documentation paths described in the handoff. Thus the inspected
product sources are also C3 sources. **C3 remains exactly
`862fa954c16d28a617271fe6f613d34226e2d558`; this report's eventual documentation
commit must not become its image revision.** Twelve-image/manifest acceptance is
handoff evidence, not reverified host evidence from WP-2. Historical exited2,
disabled timers and restored V2 do not establish a present cause or state.
WP-1's in-progress outputs were not consulted. No production/root/DB/provider
operation, build, product test, source change, commit or delegation was performed.

## Source-bound flow map

| Stage | Existing input/output and authority | Evidence |
| --- | --- | --- |
| Installed mode | Exact release receipt selects daily/daemon. Absence preserves legacy daemon startup; mismatch fails. Excluding a daemon from a Compose target list does not stop an existing daemon. | `scripts/ops/compose-release.sh:279–308,721–752`; `docs/contracts/service-status-v1.md:119–146` |
| Dates-only selection | Protected XKRX artifact supplies civil sessions; DB credentialed KRX/KR EOD rows define the missing frontier. This is operational scheduling, not official calendar evidence or verified Curated completeness. | `scripts/ops/kis-daily-production.sh:351–451,479–509`; `docs/runbooks/xkrx-calendar-bootstrap.md:3–17,67–80` |
| Calendar/EOD | A committed dedicated calendar source is reused before fetching other EOD kinds. Normal current-day ingestion can own the durable day lock/claim; historical-session CLI uses the separately named snapshot compatibility seam. | `crates/market-data/src/ingest.rs:218–246,278–343`; `data-pipelines/collectors/src/pipeline.rs:673–708`; `data-pipelines/collectors/src/worker.rs:1462–1477,2627–2658` |
| Publication | Credentialed source → immutable wire Raw → normalization → canonical four-document publication → cumulative price recovery. Final progress event is withheld until cumulative recovery succeeds. | `data-pipelines/collectors/src/pipeline.rs:646–653`; `data-pipelines/collectors/src/worker.rs:1480–1515,2661–2664`; `migrations/0022_research_publication.up.sql:13–53` |
| ETF freshness | Raw date, price dataset publication, exact ETF11 coverage, entitlement and expected calendar session are separate checks. Raw frontier is not Curated success. | `crates/api-server/src/repos/service_status.rs:78–139`; `docs/contracts/service-status-v1.md:90–108,278–290` |
| Owner-beta | Approved sealed artifact → request enqueue with strategy snapshot/five pins → exact-type worker → price-only factors/target → atomic dedicated results/run/queue settlement. | `crates/api-server/src/repos/owner_beta.rs:297–445`; `crates/job-queue/src/owner_beta/compute.rs:65–115`; `crates/job-queue/src/owner_beta/publish.rs:171–195,321–442` |
| Owner V2 | Provenance-bearing latest eligible calendar session → READY memberships whose admitted generation lags → deterministic incremental jobs → new immutable generation/signal snapshot. At most 2 GET per incremental job, distinct from Add/Retry's 7 GET. | `crates/job-queue/src/owner_equity_v2/schedule.rs:73–154`; `migrations/0053_owner_managed_equity_universe_v2.up.sql:1084–1237`; `docs/runbooks/owner-managed-equity-universe-v2.md:67–81,116–126` |
| Owner API/chart | Exact snapshot/generation/artifact binding. An older shared reference cannot make newer valid Owner data appear future-dated; freshness remains UNVERIFIABLE until an adequate reference exists. | `crates/api-server/src/http/owner_equity_v2.rs:695–728,798–811,897–903`; `crates/api-server/src/repos/candidates.rs:154–178` |
| Intraday display | Same-day credentialed calendar proof plus independent operational window; no quote/provider action is authorized by the status reader. | `docs/contracts/service-status-v1.md:265–277`; `data-pipelines/collectors/src/intraday_quotes.rs:299–309,347–374` |

## U1 — three artifacts, eligibility and preparation

| Input | Source and existing supply path | Timing / absence behavior | Responsibility still to settle |
| --- | --- | --- | --- |
| Dates-only XKRX | `kis-daily-calendar-refresh.sh` uses preprovisioned pinned `nt/.venv`, exchange_calendars 4.13.2 and reviewed overrides; fixed operational horizon 2020-01-31..2027-12-31. Apply writes protected calendar/manifest/overrides, with explicit replacement. It does not call KIS or download packages. | Must cover the DB's **first** published date through today's selected range, not only the latest missing date. Validation fails before DB/worker/provider selection if artifact is missing, tampered or out of range. Checked-in historical artifact ends 2026-08-28 and is a different artifact. | Existing role is operator. Named maintainer, verification cadence and refresh deadline are not fixed in inspected sources. G1 must assign them; do not invent a new timer. |
| Credentialed KIS calendar | Existing EOD ingestion, or installed calendar-only bootstrap with retained UUID / reuse-existing-source. Both must preserve actual retrieval/hash/source. Dedicated source reuse is already implemented even in the historical compatibility seam because `ingest_kis_bundle` checks committed calendar first. | KIS snapshot uses first actually needed date and fails on uncovered later dates without a second snapshot call. Current-day intraday proof must be exact today, three-way DB lineage and nonfuture retrieval within 36h. Historical EOD expectation needs continuous verified TRADING/CLOSED coverage through cutoff (370-day bound); historical evidence is not tested with today's 36h rule. | Operator preparation of missing historical/day evidence remains required. Named responsible person and completion time for standalone supply are unresolved. Fresh runtime claim/source state and timer/daemon preconditions must come from WP-1. |
| Today's operational_v1 window | Operator checks official KRX evidence that KST day and supplies source URL/retrieval/hash plus REGULAR/SPECIAL/CLOSED entry. `install-intraday-session-window.py` validates and publishes immutable content then activation. No browser automation or provider fetch is supplied. | Both requested date and evidence must be today's KST date, nonfuture. REGULAR is 09:00–15:30; SPECIAL uses exact reviewed bounds; CLOSED has null bounds. Missing/invalid/stale → UNKNOWN/zero quote production, with separate failed status reason. Dashboard checks window/calendar even when CLOSED. | Daily evidence renewal is explicitly manual. The named operator and deadline (before that day's actual intraday open if intraday availability is promised) are unresolved; this is not an EOD prerequisite or R3 live-WS acceptance gate. |

Evidence: `scripts/ops/kis-daily-calendar-refresh.sh:15–30,116–155`;
`scripts/ops/kis-daily-production.sh:150–154,480–482`;
`docs/runbooks/xkrx-calendar-bootstrap.md:25–36,67–80`;
`crates/market-data/src/providers/kis.rs:507–547,559–566`;
`crates/market-data/src/ingest.rs:218–235,278–343`;
`docs/contracts/service-status-v1.md:265–290,678–685`;
`scripts/ops/install-intraday-session-window.py:170–204,273–302,751–768`;
`docs/runbooks/stock-beta-intraday-quotes.md:117–126,167–207`.

The manual calendar-only path already exists: installed Compose helper verifies
current release/images, requires research daemon stopped/absent, and runs one
calendar-once command. The runbook additionally requires daily timer inactive.
The helper checks daemon state but does **not** implement that timer precondition
in `run_intraday_calendar_bootstrap`; G4 must verify it independently, never
assume preflight proves it (`scripts/ops/compose-release.sh:554–588`;
`docs/runbooks/stock-beta-intraday-quotes.md:203–249`). Its runtime requires current
operational window, owner-only mode and shared coordination; reuse does not read
KIS credential values or construct a capture-capable provider
(`data-pipelines/collectors/src/calendar_bootstrap.rs:215–308`).

Normal current-day EOD can acquire/reuse a source without requiring an intraday
window (`data-pipelines/collectors/src/pipeline.rs:694–708`), whereas the explicit
calendar-only bootstrap requires that window. Daily batch execution actually
selects `HistoricalRangeSnapshot`, not `CurrentDayShared`
(`data-pipelines/collectors/src/worker.rs:1464–1477,2649–2658`). Do not equate
single-date dedicated-source reuse with a proof that a mixed historical catch-up
and today's pre-session acquisition satisfy one total daily acquisition budget.
G1 must classify the exact dates/claim/source arrangement before enabling catch-up.
The existing historical compatibility contract must not be widened silently.

**Cutoff is not the display deadline.** Before 16:30 KST, eligible date is prior
civil day; at equality today becomes eligible. Then choose latest verified
TRADING session, with continuous coverage for display assurance. Prior complete
data can show EOD_PENDING until **18:30 KST**, with equality EOD_STALE. An older
missing session is already overdue; weekends/closures retain prior session and
deadline. The two-hour interval is accepted display grace, not a new producer SLA
or change to timers (`crates/job-queue/src/owner_equity_v2/schedule.rs:8–18`;
`docs/contracts/service-status-v1.md:31–33,278–290`).

G1 can resolve U1 with a small responsibility record: for each of the three
inputs name preparer, verifier, exact KST deadline/coverage, existing supply path,
safe missing-input result and escalation owner. Suggested constraints are
dates-only verification before any daily execution; same-day window before its
reviewed open; calendar proof before any consumer needing it; EOD outcome checked
at 18:30. These constraints are a **proposal**, not adopted new responsibilities.
Manual intraday renewal is a dependency to disclose when defining “unattended”
EOD observation; R2 must not silently promise automated intraday evidence renewal.

## U2 — exact active pins and minimal decision note

These are repository commitments, not live artifact verification:

| Five-pin field | Active v3 value |
| --- | --- |
| candidate_content_sha256 | `sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d` |
| artifact_manifest_sha256 | `sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6` |
| stage5_manifest_sha256 | `sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a` |
| action_manifest_sha256 | `sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550` |
| approval_registry_sha256 | `sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707` |

All source pin values are in
`configs/evidence/kis-historical-price-only-v3-approved-artifacts.json:1`;
registry hash was independently computed locally with `sha256sum`, matching
`docs/decisions/0006-etf11-ten-year-historical-coverage.md:165–173`.
The approval binds exact price/action sources, hashes/counts/retrieval instants,
zero noncash action rows and 157 excluded cash-dividend rows, not just a range end.
It requires MATERIALIZED/UNREGISTERED/NOT_PUBLISHED, OWNER_ONLY,
PRICE_RETURN_ONLY, vendor_snapshot=true, strict_pit=false
(`crates/market-data/src/historical_price_only_approval.rs:423–479,508–560`).
The endpoints use approved artifact bars, not licensing.as_of or generic dataset
READY. Replay selects only compiled v2 or v3 registry hashes and revalidates all
five pins (`crates/market-data/src/historical_price_only_approval.rs:268–296,335–363`;
`crates/job-queue/src/owner_beta/compute.rs:75–87`). The Aug-28 remediation spec's
Aug-19 example is stale as a v3 range claim; its discovery/display principle still
matches current implementation (`docs/superpowers/specs/2026-08-28-owner-recommendation-remediation.md:9,23–36`).

Scope of the “no daily trigger” conclusion: searches of production enqueue uses,
owner-beta modules/binary, recommendation scheduler and ops/config/Compose job-type
wiring found the repository/API producer and the claim/recovery consumer, no
owner-beta daily enqueue producer. The current client key is namespaced
`owner-beta:price-only:v1:<client-key>`; different keys can create separate requests
for the same day. Thus durable HTTP replay is **not** a once-per-session scheduling
contract (`crates/api-server/src/repos/owner_beta.rs:315–355,401–445`). Owner-beta
results/claim settle atomically and recheck recommendation entitlement on
publication; generic queue sweeping excludes the type and dedicated recovery
reconciles it (`crates/job-queue/src/owner_beta/publish.rs:171–195,321–442`;
`crates/job-queue/src/owner_beta/recovery.rs:1–6,67–83`). A healthy daemon is not a
recommendation trigger. Generic scheduling explicitly selects PAPER bindings and
cannot be repurposed in this bounded read-only beta
(`crates/job-queue/src/recommendation/schedule.rs:104–155`).

| Decision branch | Consequences and acceptance |
| --- | --- |
| Supported existing route | Continue request-driven sealed v3 research/replay. Advertise only actual common artifact dates and their latest supported date. This meets historical functionality, **cannot** satisfy market-current daily R2-04/08/09. No code change can erase that limitation. |
| Proposed minimal daily price-only route, NOT APPROVED | Keep immutable v3 history; build a separately versioned cumulative input from explicitly identified approved base plus validated daily EOD increments. Define each source batch/file/hash and original retrieval/calendar/action lineage, exact session coverage, gaps/corrections/revision policy and five-pin-equivalent approval binding. Never splice increments while claiming the fixed v3 single-capture lineage. A changing range needs a new contract/approval reader, not merely editing range_end or the existing registry. |
| Rights and authority decision | ADR-0005 already resolves retention and private single-owner use from 2016-08-29; do not ask the Owner to reconfirm those rights without an actual scope change. Preserve its exact reference/hash. The unresolved decision is independent admission authority for a new derived daily-input contract, not the existing KIS entitlement. Existing daily collection approval persists; it does not itself grant automatic input promotion or waive historical promotion gates. No new API/provider, Paper, generic READY, total-return or strict-PIT claim follows. |
| Proposed trigger, NOT APPROVED | Decide which existing Owner strategy configs are opted in, cadence, exact completed-session/input-admission prerequisite, owner actor/capacity/entitlement authority, deterministic identity and failure policy. Suggested identity binds owner, config snapshot/hash, session, input approval/version and trigger-contract version. Persist dedup across restarts and concurrent enqueue; reuse existing dedicated settlement/recovery discipline. Different approved input revisions need an explicit rerun/correction policy. No account binding/PAPER opt-in can substitute. |
| Display/publish decision | Discovery must come only from the new admitted input. Preserve old run pins/config/results; show input supported date, recommendation as_of and separate expected current session, with pending/stale/unavailable where appropriate. Never relabel Aug-28 results as today's. Define when all enabled strategies have published and how missing/blocked strategies affect daily acceptance. |

Authority for these boundaries: ADR-0006 forbids collection-only admission and
requires independent promotion (`docs/decisions/0006-etf11-ten-year-historical-coverage.md:18–38,165–173`);
current source fixes all five pins/envelope as above. The proposed route deliberately
leaves rolling lookback size, new approval authority/version, strategy opt-in,
execution time, correction policy and deadlines **UNRESOLVED**. They must be
selected before an execution-ready WP-4 brief. This is the only branch needing
additional design decomposition/reclassification; ordinary recovery and the
confirmed bounded defects below do not need an architecture redesign.

## U3 — official operation effects and recovery limits

| Operation | Exact current effect / gate | Evidence |
| --- | --- | --- |
| Release installer dry-run/check | Dry-run does not read protected inputs. Check is root-only validation of exact current installed manifest/env, no Docker or DB operation. | `scripts/ops/deploy-production-release.sh:118–141,194–223,266–269` |
| Release apply | Exact clean HEAD + protected matching env + strict V2 manifest; archive tracked files into a new root-owned immutable release; publish compatibility links and atomically switch current. Refuses overwrite. Does not build/start/stop/migrate. | `scripts/ops/deploy-production-release.sh:280–343`; `:25–49` |
| Release rollback | Validate existing target's installed env/manifest, compatibility links, atomic current switch. Does not change images, running containers, DB, data, roles, secret or timers. | `scripts/ops/deploy-production-release.sh:194–243,272–277`; `docs/reviews/2026-10-12-r1-release-readiness.md:145–151` |
| Compose plan/preflight | Validate scope config/overlays and Compose expansion; preflight additionally host preparation checks. Special calendar/refresh preflight verifies installed identity/readers. No release data/migration effects, but not a shell-only plan. Ordinary release plan/preflight does not prove all release-apply gates. | `scripts/ops/compose-release.sh:180–230,591–616,660–667` |
| Compose release apply | Verify exact installed current + manifest/image IDs; approval/V2/active-WS gates; postgres up; role bootstrap; UP migrations; raw-init; schema-check; API/Web; selected workers; proxy. Daily receipt excludes research target, absent receipt preserves legacy research target. No implicit drain or automatic failure rollback. | `scripts/ops/compose-release.sh:237–308,359–396,696–758`; `docs/runbooks/production-release-and-backup.md:327–336` |
| Migrate | SQLx UP with finite lock/statement limits, then separate catalog sync. No atomic 59-step upgrade, no automatic DOWN. Concurrent index steps can leave independently inspectable partial state. | `deploy/db/migrate.sh:217–231`; `migrations/0024_research_publication_source_index.up.sql:1–9`; `docs/reviews/2026-10-12-r1-release-readiness.md:133–139` |
| Daily install | Dry-run/preflight checks local release inputs; check compares exact unit bytes/root metadata. Apply strictly before 16:30, writes/reloads/enables, never starts. Replacement may stop an active **timer**, refuses active daily service. install-disabled is no-clobber new-unit installation/reload, postchecks loaded/inactive/disabled, any hour. | `scripts/ops/install-kis-daily.sh:165–207,342–394,475–523,526–596` |
| Timer enable vs start | Installer enable establishes boot scheduling only; actual start/activation is separate lifecycle permission and may invoke Persistent catch-up. Timer fires daily 16:30 KST, 1min accuracy, zero randomized delay; service is 12h timeout, accepted 74/75. | `scripts/ops/install-kis-daily.sh:425–460,593–596`; `docs/contracts/service-status-v1.md:191–199` |
| Daily runner plan/check/execute | Plan validates literal env/calendar only; check validates backfill config and launches DB-client one-shot for SELECT, reports missing dates. Execute additionally protected directories/global daily lock, date-specific V4 state initialization/reconciliation, and delegated provider-capable backfill. | `scripts/ops/kis-daily-production.sh:243–342,344–386,454–482,552–615`; `scripts/ops/lib/db.sh:30–65`; `scripts/ops/lib/kis-daily-state.py:182–196` |
| Backfill execution | State-specific lock; identity/calendar verification; reject research daemon; token issue attempt timestamp safeguard; progress appended/fsynced. Auto-resume allows defined resumable states but FAILED needs operator review. Does not stop daemon for caller. | `scripts/ops/backfill-production.sh:370–440,449–503,575–622`; `scripts/ops/lib/backfill-progress.py:225–234` |
| Backup check/run/verify | Check is protected-config/metadata validation. Run writes encrypted DB/Raw/Curated archives, restores disposable DB and publishes set, then **can prune old sets**. Verify-latest writes verification marker and extracts/restores temporary data; it is not read-only. | `scripts/ops/run-production-backup.sh:153–167,190–244,267–275,290–341` |

**Mode preparation order matters.** Service-status installation binds exact current
manifest/env and exact daily units before writing a receipt; apply activates the
observer, so it is a distinct operational step. A changed current plus stale receipt
blocks full release startup; no receipt starts legacy research. Daily installation
therefore cannot be replaced by legacy static units, and leaving a prior daemon
running cannot be repaired by mere target omission
(`scripts/ops/install-service-status.py:560–619`;
`scripts/ops/compose-release.sh:279–308,721–752`). G4 must settle disabled daily-unit
installation → matching policy preparation → mode-aware Compose order, with named
lifecycle authority; WP-2 does not apply any of it.

**Pending migration gates:** determine actual ledger/checksums, successful/failed
entries and concurrent-index validity first. If 0050 is already applied, new
owner-beta rows are expected and its empty-table precondition is not rerun. If it
is pending and pre-0050 rows exist, it locks the table, inspects complete contents
without FORCE RLS filtering, and rejects with 55000 rather than manufacturing
enqueue-time strategy lineage (`migrations/0050_owner_beta_strategy_snapshots.up.sql:4–29`).
Do not delete/archive rows by hand or patch checksums to pass. Data-dependent
DOWN fences and destructive removals are independently reviewed in the R1 handoff;
they are not an automatic rollback menu
(`docs/reviews/2026-10-12-r1-release-readiness.md:133–151`;
`migrations/0059_research_execution_status.down.sql:7–12`).

**Backup is not a single DB+filesystem snapshot.** `pg_dump` precedes Raw tar,
which precedes Curated tar; no shared snapshot fence or WAL continuity is recorded.
Archives exclude dedicated Owner V2 artifact storage outside Raw/Curated, runtime
coordination/claims outside those trees, release/env/config/secret/key recovery.
Whether an actual host has additional covered paths is unknown. Disposable restore
uses `--no-owner --no-privileges`, inert NOLOGIN roles and only checks presence of
`_sqlx_migrations`, `data_batches`, `dataset_versions`; it does not prove production
ownership/grants/RLS login semantics, artifact-to-row consistency or full application
recovery (`scripts/ops/run-production-backup.sh:198–240,290–303`;
`docs/runbooks/production-release-and-backup.md:630–653`;
`docs/runbooks/owner-managed-equity-universe-v2.md:21–31`).
Before data-dependent upgrade, G4 needs either approved quiescent fencing with
named services/deadline/resume responsibility or another demonstrated consistent
recovery point, complete artifact coverage, and actual-role/application restore
evidence. Old backup exit 0 and R1 image-build-only drain attestation supply none
of those missing facts. No production restoration or DOWN is proposed as an
automatic response to a failed apply.

**Catch-up budget is an admission gap.** Exact selection is every unpublished
XKRX session from first published DB date through today, including interior holes;
there is no owner-selected maximum missing date range or GET-attempt cap on this
wrapper (`scripts/ops/kis-daily-production.sh:21,479–550`). One process is bounded
to 10,000 sessions, not a one-session authorization
(`data-pipelines/collectors/src/worker.rs:1434–1445`).
At a no-retry theoretical ceiling, a newly fetched ETF EOD date needs 11 bars +
11 references + up to 7 action-class walks × 10 pages, plus one calendar capture
per snapshot acquisition: at most **92N + 1 market GETs** for N wholly fetched
dates in that snapshot-only arrangement. This is a planning upper bound, not
an enforced activation quota or retry/token budget. Calendar reuse/Raw recovery
can reduce it; transport retries and bootstrap acquisition must be accounted for
separately. Evidence: `crates/market-data/src/providers/kis.rs:80–86,229–302,507–538`.
V2 budget is separately 2 GET per incremental job; Add/Retry has its separate 7/700
ceiling (`docs/runbooks/owner-managed-equity-universe-v2.md:67–75,118–126`). G1/G4
must record fresh exact dates and total attempt budget, recovery/retry effects and
token coordination. Do not activate then discover the catch-up size.

## Confirmed source defects and bounded verification packages

“Confirmed” here means the source control/data flow exhibits the defect under the
stated input. It does not mean a product test was executed, a production incident
was reproduced, or the historical research exited2 cause was established.

### F1 — before-close filtering is lost across the backfill boundary

Input: before 16:30, today is a trading date, and at least one **older** session is
missing. Daily computes pending, drops today, reports the reduced list in check,
but execution passes `--start "$range_start" --end "$today"`. The seeded daily
state includes published DB dates only. Backfill reloads the full range and
reconstructs every date not marked PUBLISHED; thus today re-enters the actual
`--backfill-session-dates` argv. With no older hole, daily exits early and does
not exhibit this branch. Evidence:
`scripts/ops/kis-daily-production.sh:511–546,552–557,576–615`;
`scripts/ops/lib/kis-daily-state.py:182–196`;
`scripts/ops/backfill-production.sh:534–565,619–622`.

Impact: Persistent boot catch-up before close can attempt immutable partial-day
collection despite the wrapper guard. Request equality between checked selection
and worker argv; never mark today PUBLISHED merely to filter it.
Proposed exact WP-3 edit allowlist:
`scripts/ops/kis-daily-production.sh`,
`scripts/ops/kis-daily-production-self-test.sh` only.
If repairing the delegation requires an explicit session-list option or changing
state identity, return that additional `backfill-production.sh`/helper scope to
G1 before implementation; the current WP-3 maximum does not include backfill.

Focused candidate regression: fake host/clock/calendar/DB plus captured delegated
worker argv, **including the real backfill selection logic**, with older hole +
today at 16:29:59 and 16:30:00; today-only before close (no worker); no missing
sessions; interior holes; repeated execution/state preservation; permanent/74/75
failure. Confirm no real Docker/provider, no premature today Raw, matching
check/execute date lists. Existing self-test currently verifies units/state/static
contracts rather than this complete delegation branch
(`scripts/ops/kis-daily-production-self-test.sh:5–16,462–474`).

### F2 — Owner V2 marks the civil scheduling key complete on an older close

Input: after 16:30, today's calendar has not arrived but a prior provenance-bearing
TRADING row exists. Cycle returns Ok with that older `report.as_of`, possibly
scheduled=0. Caller completes today's eligible **civil key**, so later same-day
calendar publication cannot trigger another scheduling attempt in that daemon.
There is an analogous suppression when an active older incremental job causes
no insertion and later settles in the same civil key. SQL safely refuses duplicates;
caller success cadence loses the need to revisit. Evidence:
`crates/job-queue/src/owner_equity_v2/schedule.rs:78–105,107–154`;
`crates/job-queue/src/bin/owner-equity-v2-runner.rs:445–457,909–953`;
`migrations/0053_owner_managed_equity_universe_v2.up.sql:1168–1183`.

Impact: existing approved daily incremental work can lag until another eligible
key or restart; not evidence that this happened on the host. G1 expected behavior:
bounded re-evaluation when evidence/coverage advances within the same eligible
day, while existing SQL identity/active-job fence remains authoritative. Distinguish
proved CLOSED days from missing today; do not require weekday inference or keep
rescheduling an already complete generation.
Proposed exact WP-5 edit allowlist:
`crates/job-queue/src/owner_equity_v2/schedule.rs`,
`crates/job-queue/src/bin/owner-equity-v2-runner.rs`,
`crates/api-server/tests/owner_equity_v2_db.rs` only.
No migration, schema, provider, market-stream or API/Web policy changes.

Candidate tests: old calendar → successful zero-work cycle → today's late
calendar → same-key incremental enqueue; active older job settles → later
same-key progress; proven CLOSED/no-work day; repeated polls/restart/concurrent
producer produce one eligible job; no lineage/entitlement fails closed. Use binary
cadence unit seam plus disposable actual-role DB scheduler test. Existing cadence
test only changes eligible keys and calls complete itself
(`crates/job-queue/src/bin/owner-equity-v2-runner.rs:1264–1289`).

### No-change verification proposals

| Scope | Exact bounded files to read/test; no edits by default | Expected evidence |
| --- | --- | --- |
| Chart freshness already fixed | `crates/api-server/src/http/owner_equity_v2.rs`; `crates/api-server/tests/http_owner_equity_v2_chart.rs` | Preserve older-reference/newer-Owner → UNVERIFIABLE, equal → CURRENT, newer adequate reference → STALE; future cutoff rejected. Source/tests already cover these (`:798–811,1587–1614`; test `:614–631,697–774`). No fresh runtime claim. |
| Historical beta approval/replay | `crates/market-data/src/historical_price_only_approval.rs`; `crates/api-server/src/http/owner_beta.rs`; `crates/api-server/tests/http_owner_beta_price_only.rs`; `crates/job-queue/src/owner_beta/compute.rs`; `crates/job-queue/tests/owner_beta_publish.rs` | Reject unknown registry/pin/date; preserve v2/v3 replay; exact ETF11 discovery, atomic settlement and stale-claim rejection. No dynamic-registry edit while U2 unresolved. |
| Calendar/source recovery | `data-pipelines/collectors/tests/kis_recovery.rs`; `data-pipelines/collectors/src/calendar_bootstrap.rs`; `scripts/ops/test_install_intraday_session_window.py`; `scripts/ops/test_xkrx_calendar_bootstrap.py` | Existing cases include EOD-first/bootstrap-first/idempotent exact-source reuse, claim-only failure, one snapshot over covered historical dates (`kis_recovery.rs:426,469,541,723`). Test mixed catch-up/current-day arrangement separately before counting it as same-source daily evidence. |
| Release/install fences | `scripts/ops/install-kis-daily.sh`; `scripts/ops/kis-daily-production-self-test.sh`; `scripts/ops/kis-read-compose-self-test.sh`; `scripts/ops/test_install_service_status.py` | No late apply, no implicit start, disabled no-clobber/collision handling, mode receipt mismatch, daemon/timer conflict; fresh source-bound unit identity. No weakening gates. |

Suggested later commands (not run in WP-2): provider-free daily self-test and
Python calendar/window installer tests; disposable DB
`cargo test --locked -p api-server --test owner_equity_v2_db` and
`--test http_owner_equity_v2_chart`; fixture/disposable
`cargo test --locked -p collectors --test kis_recovery`,
`cargo test --locked -p job-queue --test owner_beta_publish`.
Coordinator must bind candidate, isolated resources and required no-skip environment
first. These filenames refer to actual repository locations; `owner_equity_v2_db`
belongs to **api-server**, not job-queue. Do not run production-coupled tests.

## Missing inputs for WP-1 / coordinator integration

| Input | Safe required observation / effect on G1 |
| --- | --- |
| Installed identity/mode | UTC/KST observed time, exact current release/manifest/image binding, receipt profile/project/commit; physical daily service/timer bytes/digests/drop-ins and full timer inventory. Historical values are insufficient. |
| Execution exclusivity | Current daemon/one-shot state, active daily/backfill locks, current queue leases/attempts and token-coordination mode validity without printing token/state bytes. Unit start/end/normal exit/74/75, last/next trigger with actual receipt, not unloaded default Result=success. |
| Calendar supply | Operational dates-only range/hash/counts/trust; first DB EOD date and exact missing dates through eligible cutoff; today's credentialed day claim/source and three-way calendar joins; 370-day historical coverage including CLOSED; today's window metadata/hash/same-day retrieval and actual bounds. |
| ETF lineage | Raw EOD dates separately from exact price publication/dataset/raw binding, ETF11 session count, rights predicate result and publication time. Raw max or dataset READY alone is insufficient. |
| Owner-beta | Installed compiled artifact approval/discovery range/hash agreement with v3; actor-scoped latest successful as_of/finished_at/input five-pin match and strategy snapshot; queue type counts without owner/job IDs or payload dump. No observed fresh daily trigger should be inferred from SUCCEEDED count. |
| Owner V2 | Owner-scoped active/READY counts, latest generations plus all five admission joins, minimum covered last_session, exact current-universe snapshot/rows and published_at; scheduler's actually selected as_of/key if safely exposed. |
| Upgrade | Actual ledger success/checksum delta and concurrent-index validity; if 0050 pending, **globally visible** pre-0050 run existence with legitimate read role (actor-local count cannot prove global emptiness); migration/catalog/role/RLS applicability. |
| Recovery readiness | Exact encrypted set identity/coverage/integrity, checkpoint consistency rationale, dedicated artifact availability, restoration limits/actual-role checks, previous release compatibility with current schema/jobs/data/config and legitimate secret recovery. Fresh disk/memory/concurrent work and maintenance scope/deadline/resume owner. |

The optional private aggregate draft is a coordinator-review aid, not an executed
query or approved WP-1 command. It returns summaries only, uses schema definitions
and existing source queries, and leaves connection/role/actor/date values unresolved.
Do not execute it until the coordinator fixes scope/RLS/read-only limits and confirms
the installed schema. See the runbook for precise gating and the private directory
for provenance. All missing runtime inputs above remain unverified in WP-2.

## Checks and bounded handoff

Performed: local `rg`/`rg --files`, line-numbered source/document reads, local
`git rev-parse HEAD`, C3→HEAD pathname diff, registry `sha256sum`; documentation
whitespace, evidence-reference/path and private permission checks. No product
tests/builds/SQL/operations executed. Documentation-only checks do not validate
the proposed defect repairs. The supplied memory-context envelope was used;
no Basic Memory or invented scope/session was used.

Unresolved: U1 named duties/deadlines; U2 new input/admission/trigger/display
contract; U3 fresh identity/migration/restore/catch-up/maintenance inputs. F1/F2
await G1 reproduction contracts and worker scope acceptance. No present root
cause is asserted. Coordinator receives the runbook draft's ownership after WP-2;
source findings are proposals to integrate, not accepted operating instructions.
Background work: none. Permission waits: none. Further decomposition: only the
unresolved U2 design branch; this bounded analysis is finished.

### Coordinator clarification — existing entitlement

The rights wording above was narrowed after directly checking
`docs/decisions/0005-kis-personal-use-entitlement.md:16–26`: existing private
single-owner use and retention must not be reconfirmed without revocation or a
scope change. No such change is proposed. Only the new daily input admission and
unattended recommendation contract remains a decision. This clarification does
not approve an actual new artifact or change the original source findings.
