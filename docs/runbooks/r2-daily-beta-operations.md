# R2 daily research beta — operations preparation

WP-2 offline draft, 2026-10-13; source
`cea7baa6298402db79d655140ad4c199f57f8697`. Coordinator takes ownership after
WP-2 and completes G1/G4 before any operating step. This is an effect map and
decision checklist, **not an authorization or executable apply sequence**.
Existing daily KIS/V2 read-only approvals persist. Root access, an installed
unit, a previous R1 drain or twelve built images do not grant lifecycle authority.
See [contract and recovery review](../reviews/2026-10-13-r2-contract-and-recovery.md)
for source findings, minimal U2 decision and bounded repair/test scopes.

## Values that must be recorded before execution

| Binding | Value / resolution owner |
| --- | --- |
| Serving candidate | **UNRESOLVED — G2.** C3 remains exactly `862fa954c16d28a617271fe6f613d34226e2d558` for R1. F1/F2 change R2 product inputs, so R2 requires a separate candidate; partial CI `3ee453f` is not final C_R2. |
| Fresh installed state | **OBSERVED — WP-1; refresh at G4.** 2026-10-13 13:48–14:00 UTC: current `fce9b90`, owner_daily, strict installed manifest; 3 running manifest services match and 5 mismatch. [Baseline](../reviews/2026-10-13-r2-runtime-baseline.md) records exact identities/hashes and limits. |
| Protected env | **CURRENT VERIFIED — WP-1; target UNRESOLVED — G4.** Installed unit uses trusted release-local `.env`, matching the global alias after resolution. New target commit/env binding still needs preparation; no values or credentials copied here. |
| Target/previous release | **UNRESOLVED — G4.** Immutable target path plus strict installed manifest; previous release target, schema/job/artifact/config compatibility evidence and recovery responsibility. |
| Operational calendar | **OBSERVED — WP-1; duties/deadline UNRESOLVED — G1.** Protected 2020-01-31..2027-12-31 artifact validated; 2026-08-18..10-13 selection is 37 sessions. Artifact/manifest hashes are in the baseline and private selection receipt; these are dates-only scheduling evidence. |
| Credentialed day evidence | **GAP OBSERVED — WP-1; preparer/deadline UNRESOLVED — G1.** At 2026-10-13 14:51 UTC, latest verified eligible calendar was 10-07 and exact 10-13 evidence was absent. Filesystem claim was not inspected. Existing source must be reused; indeterminate claims never recaptured. |
| Operational window | **UNVERIFIED — WP-1; named manual preparer/deadline UNRESOLVED — G1.** Actual current-KST official evidence, same-day retrieval, immutable hash, activation and REGULAR/SPECIAL/CLOSED bounds. |
| Recommendation input/trigger | **BLOCKED — U2/G1.** Existing active v3 ends 2026-08-28; existing personal-use rights persist. [Daily input/admission and opt-in trigger proposal](../reviews/2026-10-13-r2-daily-input-decision.md) awaits adoption. |
| Catch-up | **OFFLINE REVIEW ACCEPTED — WP-1; budget/action UNRESOLVED — G1/G4.** 28 daily-record gaps through the completed 2026-10-13 cutoff, plus 8/31 as an existing-EOD publication-recovery candidate. [Exact sets, conditional request counts and broader recovery effects](../reviews/2026-10-13-r2-catch-up-readiness.md) are reviewed; fresh admission and whole-run limits remain. No provider request made. |
| DB upgrade | **PARTLY VERIFIED — WP-1; refresh at G4.** 59 migrations match source checksums, pending/mismatch 0; 0050 already applied. Seven index existence/valid/ready/table/unique properties pass; complete definitions, roles/RLS and any new WP-4 migrations still need review. |
| Recovery point | **UNVERIFIED — G4.** Backup identity/integrity, DB+files consistency, dedicated artifact coverage, actual-role/application restore, independent secret/config recovery. |
| Lifecycle actions | **UNRESOLVED — G4.** Exact named units/containers, commands/effects, allowed interruption, shared absolute deadline, current work/lease settlement, resume owner, stop/success criteria. |
| Activation/observation | **UNRESOLVED — G4/G5.** Actual KST apply/start date/time, selected timer and other-timer disposition, first expected session and three-trading-day observation start. |

No values are inferred from the historical `research-worker` exited2 observation.
The exact current mode determines whether daemon absence is expected
(`docs/contracts/service-status-v1.md:119–146`). R3 live WS acceptance is not an
EOD acceptance prerequisite (`docs/ROADMAP.md:144–147,282–283`).

## Calendar and freshness responsibilities

Maintain three separate records, each with named preparer/verifier and safe
absence result. G1 must adopt those assignments; the draft does not appoint them.

1. **Dates-only scheduler:** provider-free protected refresh through
   `kis-daily-calendar-refresh.sh` after the pinned environment and reviewed
   horizon/override evidence are ready. Existing fixed refresh range is
   2020-01-31..2027-12-31; a DB frontier older than that blocks selection and needs
   a reviewed range decision, not a guessed weekday list. Check reads files;
   apply writes protected calendar/manifest/overrides and requires reviewed
   replacement when changed. Checked-in XKRX history is not today's installed
   artifact or canonical KIS calendar
   (`scripts/ops/kis-daily-calendar-refresh.sh:15–30,116–155`;
   `scripts/ops/kis-daily-production.sh:479–482`;
   `docs/runbooks/xkrx-calendar-bootstrap.md:3–17,25–36`).
2. **Credentialed KIS calendar:** existing EOD capture or the installed
   calendar-only command. Preserve committed source bytes/hash, original retrieval
   and exact day claim. Calendar-only explicit UUID acquisition differs from
   reuse-only publication. Claim without source, source without matching claim,
   conflicting identity or multiple sources stops; no fresh UUID workaround.
   Calendar-only requires current operational window/shared mode, research daemon
   stopped/absent and independently checked daily timer inactivity. Those
   lifecycle actions need exact G4 authority. Ordinary EOD need not wait for an
   intraday window or WS receipt
   (`data-pipelines/collectors/src/calendar_bootstrap.rs:253–308,399–475`;
   `scripts/ops/compose-release.sh:554–588`;
   `docs/runbooks/stock-beta-intraday-quotes.md:203–249`;
   `data-pipelines/collectors/src/pipeline.rs:694–708`).
3. **Operational window:** manual official KRX check on the actual KST day.
   Installer has no fetch/browse path, admits only current requested date and
   same-day nonfuture evidence, retains content-addressed files and activates
   last. Supply before actual reviewed intraday opening if intraday availability
   is intended. Named preparer/deadline remain unresolved. Missing/stale/invalid
   window fails intraday proof; it must not be substituted with XKRX dates or
   yesterday's restamped entry
   (`scripts/ops/install-intraday-session-window.py:170–204,273–302,684–700,751–768`;
   `docs/runbooks/stock-beta-intraday-quotes.md:167–199`).

Expected EOD session: before **16:30 KST** use prior civil day; at equality use
today, then latest verified TRADING session with continuous TRADING/CLOSED
coverage through cutoff, bounded to 370 days. Before a newly eligible session's
**18:30** deadline, prior complete data may be EOD_PENDING; at equality lag is
EOD_STALE. Older missing sessions already have overdue deadlines. Holidays retain
the prior session/deadline. This is display grace, not a new production SLA
(`crates/job-queue/src/owner_equity_v2/schedule.rs:8–18`;
`docs/contracts/service-status-v1.md:31–33,278–290`).
Current intraday calendar has separate exact-today/36h lineage checks; historical
EOD evidence does not acquire invented fresh retrieval timestamps. Independent
window proof is checked even on CLOSED by the dashboard
(`docs/contracts/service-status-v1.md:265–284`).

## Official helper effects and dependency order

G4 must populate an exact command ledger using reviewed values. No apply command
with placeholder commit/env/date/permission is supplied here.

| Step | Effect and stop boundary |
| --- | --- |
| Source/image admission | G2 fixes product candidate, G3 accepts same-candidate verification and twelve real images/manifest. C3 reuse needs source-identity proof; changes require a separately fixed candidate. No old build exception/drain is a fresh operational admission. |
| Recovery gate before first mutation | Establish backup and complete consistent recovery scope, previous-release compatibility, fresh work/leases/resources and exact service interruption/resume authority. If any migration is blocked, stop before roles/migrate. |
| Release installer `--dry-run` / `--check` | Dry-run reads no protected source. Root check validates current installed env/strict V2 manifest; neither changes containers or DB (`scripts/ops/deploy-production-release.sh:118–141,194–223,266–269`). |
| Release installer `--apply` | Requires exact clean HEAD, new no-clobber directory, trusted env commit and V2 manifest. Archives/copies/verifies and atomically switches current/compatibility links. Does not stop/start/build/migrate (`scripts/ops/deploy-production-release.sh:280–343`). |
| New disabled daily units and policy binding | Daily installer `--install-disabled` writes only unused exact names, reloads, verifies loaded/inactive/disabled. Can install after 16:30; existing target/unit collision stops. Service-status policy preparation needs matching current/env/manifest and those daily files; its apply writes policy/history/units and activates observer, a separate operational action (`scripts/ops/install-kis-daily.sh:291–394,521–523`; `scripts/ops/install-service-status.py:560–619`). |
| Compose plan/preflight | Config/overlay and Compose expansion validation, preflight host provisioning checks. Special bootstrap/refresh additionally validate installed identities. Do not count ordinary plan/preflight as complete release-apply admission (`scripts/ops/compose-release.sh:180–230,591–616,660–667`). |
| Mode-aware full release Compose apply | Verify installed current/image IDs, owner-beta artifact approval, V2 and active-WS process gates; postgres up → role bootstrap → SQLx UP + catalog → Raw init → schema check → API/Web → selected workers → proxy. These are mutations even if images are cached. Daily receipt omits research daemon target; absent receipt uses legacy selection. Does not drain an already running daemon or auto-rollback after failure (`scripts/ops/compose-release.sh:237–308,359–396,696–758`). |
| Calendar preparation/publication | Dates-only protected refresh and actual current day proof/window are distinct from release deployment. Calendar-only one-shot may write Raw/DB or reuse/publish; mixed historical catch-up must be separately classified. No scheduled window renewal is created (`scripts/ops/compose-release.sh:554–588`; `data-pipelines/collectors/src/worker.rs:1462–1477`; `crates/market-data/src/ingest.rs:218–246`). |
| Daily installation `--apply` | Strictly before 16:30:00 KST, even for root. Writes units/reloads/enables timer, never starts either unit. Replacement can stop existing active timer and refuses active service. No lateness override (`scripts/ops/install-kis-daily.sh:526–596`). |
| Timer start / first provider-capable run | Separate lifecycle decision, fresh exact missing sessions/budget and exclusivity first. Persistent activation may immediately catch up. Do not bypass the installer's 16:30 gate using a guessed direct activation sequence. Source F1 needs repair/reproduction before pre-close backlog execution (`scripts/ops/install-kis-daily.sh:425–460`; review F1). |
| Result/observation gate | Record terminal receipt, actual ETF Raw/Curated dates/lineage and recommendation input/as_of, V2 exact admissions/snapshot, API/display. Timer enabled/healthy container/accepted 0 or 74/75 is not publication success (`docs/contracts/service-status-v1.md:191–199`; `docs/ROADMAP.md:269–292`). |

Release helper's optional active-WS gate and config effects must be respected
without expanding this R2 worker into R3 activation. If the installed modes
necessitate an operational branch outside approved G4 scope, return that exact
branch/effect for a coordinator decision; do not silently alter protected modes
or execute a refresh (`scripts/ops/compose-release.sh:206–225,387–396`).

## Missing-session and retry admission

Daily runner `--plan` validates env/calendar with no DB/provider/state operation;
`--check` reads an EOD snapshot via `db_psql`, which starts a temporary
`db-migrate` client container (`--no-deps`, secret read only inside it), not the
migration entrypoint. The SELECT has no explicit READ ONLY transaction or statement
timeout in the wrapper. A WP-1 query must therefore be separately reviewed for
role/image binding, minimum scope, timeout and container effect
(`scripts/ops/kis-daily-production.sh:351–386,454–482,552–557`;
`scripts/ops/lib/db.sh:30–65`). Its no-Docker wording in check output refers to
worker execution and must not hide the DB-client effect.

The selection spans **first published credentialed KRX/KR EOD date through
today**, including interior holes, not just last-success+1. It rejects no frontier,
duplicate per-date EOD, future dates, calendar mismatch and over-10,000 selection;
it does not enforce a chosen owner GET-attempt budget
(`scripts/ops/kis-daily-production.sh:362–451,479–550`).
Record every exact date and a reviewed attempt/token ceiling before activation.
Pure no-retry ETF market-GET planning ceiling is 92 per fetched date plus one
calendar snapshot call (11 bars, 11 reference, seven action classes × at most ten
pages). Actual Raw reuse can reduce it; retries, standalone calendar bootstrap,
snapshot misses, token issuance and V2 demand must be budgeted separately. This
formula is **not enforced quota or approval**
(`crates/market-data/src/providers/kis.rs:80–86,229–302,507–538`).

Keep global daily lock and backfill state lock; reject a research daemon, preserve
state/Raw and review permanent failure. The per-state token-window timestamp is
one-minute attempt protection, not global proof across changed state/release.
`shared_required` wiring supplies existing cross-process coordination; preserve
it without dumping persisted secret state
(`scripts/ops/kis-daily-production.sh:327–342,572–603`;
`scripts/ops/backfill-production.sh:422–440,472–503,575–609`;
`scripts/ops/lib/kis-read-compose.sh:170–201`;
`docs/runbooks/stock-beta-intraday-quotes.md:292–303`).
74 is retryable and 75 deferred snapshot miss; both are accepted systemd results,
not proof of useful completion. Do not mark a state/session PUBLISHED or READY
with SQL, delete a queue, reset counters or recapture an indeterminate calendar
(`scripts/ops/backfill-production.sh:627–639`;
`docs/runbooks/owner-managed-equity-universe-v2.md:175–179`).

## Input and Owner coverage gates

Current owner-beta new requests are sealed v3, 2016-08-29..2026-08-28; replay can
select compiled v2/v3 only by persisted five pins. Exact pins and decision proposal
are in the review. No existing daily owner-beta producer was found. Generic daily
recommendation uses opted-in PAPER accounts and independent dataset pins, so it
is not a fallback (`crates/market-data/src/historical_price_only_approval.rs:249–296`;
`crates/market-data/src/range_to_canonical_v3_price.rs:28–38`;
`crates/api-server/src/repos/owner_beta.rs:315–355`;
`crates/job-queue/src/recommendation/schedule.rs:104–155`). U2 must settle the new
admitted-input and unattended trigger before current daily recommendation
acceptance; preserve historical research functionality meanwhile.

V2 existing READY increments remain approved, sequential, maximum 2 GET/job,
with admitted prior generation/entitlement/commit and SQL idempotency/active-job
guard. Source F2 must be reproduced/repaired or explicitly rebutted before counting
late-day evidence arrival as reliable daily scheduling. Compare all five admission
identity columns, minimum covered last_session and complete current snapshot rows;
generation created_at alone is not successful publication
(`docs/runbooks/owner-managed-equity-universe-v2.md:118–126`;
`migrations/0053_owner_managed_equity_universe_v2.up.sql:1149–1237`;
`crates/api-server/src/repos/service_status.rs:157–216`).
Maintain the existing chart fix: stale shared ETF reference with newer valid
Owner snapshot → UNVERIFIABLE, not future-data failure; independently future
Owner date beyond cutoff remains rejected
(`crates/api-server/src/http/owner_equity_v2.rs:798–811,897–903`).

## Migration, backup and restoration boundaries

Before Compose apply, compare actual ledger and file checksums plus concurrent
index validity. SQLx UP and later catalog sync are separate operations. Pending
0050 requires global pre-0050 run table emptiness and rejects 55000 otherwise;
already applied 0050 is not blocked just because post-upgrade runs exist
(`deploy/db/migrate.sh:217–231`;
`migrations/0050_owner_beta_strategy_snapshots.up.sql:4–29`). Check real role/RLS
visibility; actor-local empty results cannot attest global migration readiness.
No ad-hoc data deletion, fabricated strategy snapshot, edited checksum or
automatic DOWN resolves this gate.

Official production backup captures DB dump, Raw and Curated sequentially;
it has no automatic common snapshot fence/WAL link. It omits host env/config,
secret/key material and dedicated V2 artifact tree outside Raw/Curated. Verify
additional recovery coverage rather than assuming it. Disposable restore builds
inert NOLOGIN roles, restores without owners/privileges, checks three table names;
it does not prove application role/login/RLS, complete artifact linkage or
application recovery. `--run` can prune old sets; `--verify-latest` writes a marker
and restores temporary data, so neither is a read-only WP-1 command
(`scripts/ops/run-production-backup.sh:190–244,267–341`;
`docs/runbooks/production-release-and-backup.md:630–653`;
`docs/runbooks/owner-managed-equity-universe-v2.md:21–31`).
G4 must fix the consistent recovery point, fence or reconciliation proof, dedicated
artifact/state scope, actual-role restore checks and secret-recovery owner before
upgrade. Backup receipt alone does not complete this step.

| Failure point | Bounded response for G4 to authorize explicitly |
| --- | --- |
| Installer validation/staging failure | Stop; preserve evidence and old immutable releases; verify whether current changed. No Compose/DB restoration inferred. |
| Current changed, Compose not applied | Verify previous target's manifest/env and schema/data compatibility before official current rollback. This switches only link/compatibility paths. |
| Migration or catalog failure | Preserve actual successful ledger entries/index state; determine which phase committed. Do not DOWN, rewrite checksum or rerun before applicable data conditions are established. |
| Partial service startup/image mismatch | Record actual image/health/work/leases and affected service set. No automatic stop or old Compose invocation. Coordinator selects compatible target/actions, interruption deadline and resume responsibility. |
| Daily run failed/deferred/indeterminate | Preserve state, immutable Raw and unfinished jobs; distinguish retryable/deferred/permanent and published dates. Recheck exact budget/calendar/source before approved recovery. |
| Operational restoration needed | Separate destructive/data-restoration decision with exact set, consistency, role/secret/artifact and compatibility checks. Link rollback is not DB/files restoration. |

Effects are supported by `scripts/ops/deploy-production-release.sh:194–277,313–343`,
`deploy/db/migrate.sh:217–231`,
`docs/runbooks/production-release-and-backup.md:327–336`,
`scripts/ops/backfill-production.sh:422–440,627–639`, and
`docs/reviews/2026-10-12-r1-release-readiness.md:128–151`.

## Completion evidence and remaining handoff

G1 integrates WP-1 fresh facts with WP-2 source findings, accepts/rejects F1/F2,
assigns U1 duties and settles/reclassifies U2. G4 supplies exact executable actions
and missing U3 values; G5 fixes actual expected sessions and observation start.
Do not claim R2 complete from isolated regression, migration exit, timer activation
or healthy container. Three consecutive actual trading days require scheduled
ETF data/recommendation and Owner updates with matched input/as_of/API/display,
no manual completion substituted, plus normal Owner use
(`docs/ROADMAP.md:254–292`).

WP-2 ran only offline reads/hashes and document/reference/permission checks. Product
tests, host evidence, restoration, lifecycle and first data receipt are unverified.
No remaining WP-2 background process or permission wait. This draft's unresolved
cells are deliberate; coordinator owns completing them and roadmap/execution
updates after accepting the bounded review.
