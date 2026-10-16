# WP-7 independent integrated R2 release review

**INCOMPLETE integrated coverage; four concrete findings: two P1 and two P2.**
This is an independent, report-only source review of the full R2 release scope,
not a continuation of the earlier snapshot-only reviews. It does not grant
implementation, compiler, runtime, operational or final R2 acceptance. G3 cannot
close on this report: the findings and the explicitly unreviewed scope remain.

## 1. Revision, authority, inputs and timing

- Candidate: `f215665012d1f4f1b81289dd94606bceb0cb8c46`.
- Comparison baseline: `b0a2579bef472ae6b604405d0ab6f50255786dd4`.
- Inventory: [review-inputs.json](../../target/r2-daily-beta/wp-7/f215665/review-inputs.json),
  1,480 lines, SHA256 `f7e598e6900df1393bc36f6f4969b8039fa05fd626bd72ccbd69aa2785c1c1f4`.
- Verified all 211 frozen local pins: 202 changed source paths, two diagram
  drafts and seven evidence artifacts. SHA256, byte counts and specified line
  counts matched. Independently matched all 202 candidate Git tree/blob bindings
  and SHA256 of those candidate blob bytes. There are 386 changed paths including
  documentation; the 202 review targets total 6,862,906 bytes / 174,410 lines.
- Startup: `2026-10-16T17:31:05.523Z` (`2026-10-17` KST); reporting deadline:
  `2026-10-16T18:31:05.523Z`. Final check time is recorded below.
- Read root `AGENTS.md`, current R2-01–09 roadmap entries, the integrated review
  packet, WP-7/G3–G6 in the daily-research plan, and the implementation plan's
  Wave86 resumption. The latest one-reviewer/report-only authority overrides the
  older suggested output location, routine roadmap-update instructions and
  historical `reviewerStarted:false` preparation field. No delegation occurred.
- The separate [resumption receipt](../../target/r2-daily-beta/wp-7/f215665/resumption-2026-10-17.json)
  records the new authority/start. No automatic memory helper or receipt was
  written, and no coordinator hook identity was reused.

Identity verification is not semantic review. Appendix A enumerates every
changed target and distinguishes full text inspection, changed-hunk inspection,
partial inspection and identity-only checks. Unread tests, generated contracts
and unchanged adjacent implementation are not implicitly covered by related
file names, previous component reports or hosted PASS rows.

## 2. Prioritized findings

### P1-01 — Valid daily ON configuration blocks its independent EOD supplier

**Anchor:** [validate-production-config.sh:142](../../scripts/ops/validate-production-config.sh#L142).
The new mode gate rejects every `owner_only` daily configuration whose validation
scope is not `release`. Both official supplier paths deliberately validate with
`--scope backfill`: [kis-daily-production.sh:344](../../scripts/ops/kis-daily-production.sh#L344)
and [backfill-production.sh:535](../../scripts/ops/backfill-production.sh#L535).

**Trigger:** Install a valid daily ON protected environment and run the existing
official KIS daily/backfill helper against that environment. ON is not an
ambient bypass: [activation_binding:151](../../scripts/ops/lib/owner-beta-daily-operations.py#L151)
requires `OWNER_BETA_DAILY_MODE=owner_only` in the protected release values.
The installed daily unit passes its installed environment to the official helper
([install-kis-daily.sh:420](../../scripts/ops/install-kis-daily.sh#L420)). The
closed mode parser reads that same dotenv field
([owner-beta-daily-release.sh:16](../../scripts/ops/lib/owner-beta-daily-release.sh#L16)).

**Consequence:** Validation fails before the approved collection path runs.
Enabling the recommendation pipeline therefore prevents normal EOD supply and
catch-up with its valid installed environment. This is a source-level integration
blocker; it is not a claim that the currently installed older release has hit it.

**Bounded closure:** Allow a correctly validated ON release environment to be
used by its independent read-only supplier scopes, while retaining verified
release-only daily activation, default OFF, ambient-override rejection, calendar,
pre-close and explicit request/catch-up limits. Do not work around the defect by
editing the installed protected environment or injecting a disabled shell value.
Add an integrated regression through the real configuration validator and both
official helper call paths using valid ON values. The inspected whole-flow driver
preseeds Raw evidence; it does not exercise this supplier/validator composition.

### P1-02 — Bootstrap status rejects ordinary enabled, non-forced users RLS

**Anchor:** [owner-beta-daily-control-status.sql:46](../../deploy/db/owner-beta-daily-control-status.sql#L46).
When no daily control row exists but the historical run table has `input_kind`,
the status query rejects `users.relrowsecurity=true`. Migration 0010 intentionally
enables RLS on the shared `users` table; that catalog flag does not mean its table
owner, `migration_owner`, is effectively filtered. The candidate's actual-role
backup fixture explicitly distinguishes `(true,false,"migration_owner")` from
`row_security_active=false`
([owner_beta_daily_backup.rs:161](../../crates/job-queue/tests/owner_beta_daily_backup.rs#L161)).

**Trigger:** A normal schema59 or schema62 database, ordinary users RLS, and no
daily control row. Merely having the historical recommendation table/input-kind
column takes this branch; an orphan daily run is not required.

**Consequence:** Status raises `DAILY_CONTROL_IDENTITY_VISIBILITY` instead of
returning the legitimate bootstrap state. Initial `prepare` calls this status
before creating control ([operations.py:495](../../scripts/ops/lib/owner-beta-daily-operations.py#L495));
the reader-floor release guard also calls it when that guard is required, before
admitting the old schema
([operations.py:668](../../scripts/ops/lib/owner-beta-daily-operations.py#L668)).
Initial prepare, and the applicable reader-floor guard path, can be blocked.

**Bounded closure:** Use effective filtering for the required direct
`migration_owner` caller, as the existing
[backup audit:58](../../deploy/db/owner-beta-daily-backup-audit.sql#L58) and reservation
repair already do. Preserve refusal of missing/inaccessible identities, effective
RLS, orphan history and actor-count overflow; do not disable RLS or broaden roles.
Exercise real schema59/62 no-control status and first prepare, with separate
FORCE-RLS and orphan-history negative controls. The inspected transport regression
[creates users without RLS](../../scripts/ops/owner-beta-daily-transport-self-test.py#L246),
so it misses this normal-schema condition. Its four-state fixture also substitutes
minimal tables instead of the actual migration layout.

### P2-01 — Orphan-admission reconciliation requires the wrong secret mode

**Anchor:** [owner-beta-daily-backup.py:645](../../scripts/ops/lib/owner-beta-daily-backup.py#L645).
The reconciliation reader requires the admitter's serving secret to be
`10003:10003/0400`. The canonical provisioner creates that exact runtime path as
`10003:10003/0440`
([provision-runtime-secrets.sh:151](../../deploy/secrets/provision-runtime-secrets.sh#L151));
the production validator independently expects `10003:10003/440`
([validate-production-config.sh:842](../../scripts/ops/validate-production-config.sh#L842)).

**Trigger:** Reconcile a correctly provisioned installation whose immutable file
inventory contains an extra admission requiring native reverification. The
controller calls `reverify_once` in that branch
([backup.py:802](../../scripts/ops/lib/owner-beta-daily-backup.py#L802)).

**Consequence:** Trusted-file validation fails before native reverification. The
controller remains safely OFF, but cannot reconcile this recoverable orphan state
with the official provisioned secret. Changing host permissions to bypass the
reader would contradict the provisioner and validator.

**Bounded closure:** Align this one read with the established serving-secret
0440 contract, retaining exact UID/GID, directory, link, ACL and redaction guards.
Add a real-file metadata regression through the unmocked reverification preflight.
The inspected controller fixture mocks the entire function
([backup-self-test.py:612](../../scripts/ops/owner-beta-daily-backup-self-test.py#L612));
its orchestration assertions cannot catch this mismatch.

### P2-02 — Daily restore-verifier uncertainty loses recoverable ownership state

**Anchors:** [backup.py:364](../../scripts/ops/lib/owner-beta-daily-backup.py#L364),
[backup.py:402](../../scripts/ops/lib/owner-beta-daily-backup.py#L402),
[backup.py:263](../../scripts/ops/lib/owner-beta-daily-backup.py#L263).
The daily restore verifier creates its Docker container before entering its
cleanup `try/finally`. A lost create response leaves no recorded container ID or
durable owned-name/nonce intent. After start, failure of the cleanup inspection,
TERM or wait can also leave cleanup unconfirmed. In either case the enclosing
`Storage.temporary` unconditionally deletes the private restored staging tree.

**Trigger:** Docker accepts create but the client loses its response, or an
inspection/wait fails after the isolated verifier has started. These are explicit
bounded subprocess/daemon error paths, not an assumed production incident.

**Consequence:** An owned created/running verifier may survive the failed command
while its mounted copy/dump is removed and no recoverable identity/staging receipt
is retained. Another verification has no unresolved-intent guard and can create
another clone. This does not claim production database overwrite or a false
successful restore; it is a cleanup/recovery defect in the daily verification path.

**Bounded closure:** Record exact owned name/nonce/image/staging intent before
create; resolve only that owned resource; remove its copy only after confirmed
cleanup. Retain intent/copy and fail closed on unresolved cleanup, preventing a
new verifier until bounded reconciliation. The candidate already implements this
pattern for the separate cold path
([production-preupgrade-recovery.py:143](../../scripts/ops/lib/production-preupgrade-recovery.py#L143),
[intent:224](../../scripts/ops/lib/production-preupgrade-recovery.py#L224)). Extend
the daily verifier's tests to lost create reply and cleanup-observation failure.
The four native cold-recovery cases cover a different implementation; successful
daily invalid-dump cleanup does not establish these failure paths.

All four findings are static source conclusions, not locally executed failures.
No additional concrete P1/P2 finding is asserted for an unread path or solely
because operational evidence is missing.

## 3. R2-01–09 dispositions and reviewed cross-component flows

| Item | Independent disposition and remaining condition |
| --- | --- |
| R2-01 installed path/mode diagnosis | OPEN. Compared protected-mode parsing, official daily/backfill calls and immutable-release controls; P1-01 blocks their valid ON composition. Old fce9b90/timer observations do not admit this candidate. Fresh protected manifest/state and complete host health remain unavailable to this reviewer. |
| R2-02 calendar/window responsibility | OPEN. Read the three-way distinction between date artifact, credentialed KIS calendar and same-day intraday window; inspected pure supply planning and bounded calendar/reconstruction. Named provider/window preparer, concrete readiness deadline and adopted G4 command/budget binding remain operational prerequisites. A 10,000-date catch-up ceiling is not a GET budget. |
| R2-03 daily execution recovery | OPEN; P1-01. Inspected exact missing-date filter propagation, official helper validation and release-local unit invocation. No supplier call, timer activation or catch-up occurred. Exact allowed dates/requests, no duplicate daemon and actual installed-mode recovery still need evidence. |
| R2-04 ETF daily pipeline | PARTIALLY REVIEWED, not accepted. Followed snapshot → lineage/calendar → provider reconstruction → frontier/continuation → sealed artifact → independent admission → schedule → sealed computation/publication → API/UI dates. Strong source boundaries are described below; wire/S1 and test residuals remain in Appendix A. First actual approved-base host whole-flow and publication are open. P1-01/P1-02 obstruct rollout. |
| R2-05 Owner V2 compatibility | PARTIALLY REVIEWED / operationally OPEN. Read the 60-second scheduler delta, mixed historical/daily queue dispatch and collector classification deltas; inspected preservation of existing strategy equations and historical pins. Did not independently audit all unchanged Owner V2 collector/generation/signal/chart paths or their full regression bodies. Actual same-input Owner V2 refresh remains open. |
| R2-06 actual Owner use | OPEN. Inspected new Owner/entitlement checks, session extraction context, CSRF/idempotency on configuration changes and UI eligibility. The approved-base driver uses a synthetic Owner session and cannot substitute for normal Auth0 login and actual Owner interactions. |
| R2-07 failure/expiry/recovery | PARTIALLY REVIEWED / OPEN. Read stale/failure/queue/retry projection, separate input/recommendation/publication dates, prior-result retention, browser session-expiry handling, failure settlement and recovery deltas. Full adversarial test-body review and installed cancellation/restart/recovery remain open. P2-01/P2-02 affect maintenance recovery. |
| R2-08 three trading days | OPEN, not started for this candidate. Actual first publication and three consecutive unattended trading days, including Owner V2/data/API/UI dates, are absent. Manual repair or fixture time advancement cannot count as an unattended day. A failed day requires the roadmap's restart condition. |
| R2-09 first release acceptance | OPEN. Inspected migration0060–0062 source, bootstrap, controls/drain, V1/V2 backup transition, cold schema59 recovery and approved-base driver. P1-02/P2-01/P2-02, unreviewed validation/test paths, host authority/admission, actual images/recovery/deployment, separate functional smoke, Owner and observation gates prevent closure. Current-link selection is not service migration or database rollback. |

### Source properties traced, within the stated coverage

- **Evidence and authority:** The snapshot retains immutable bytes and read-only
  descriptors under the shared commit lock; explicit trusted UID, no-follow
  descriptor traversal, pre-dedup bounds, physical-line commitments and final
  bindings are distinct from approval. Reconstruction checks full source identity,
  lineage/calendar/request chains and recomputes pure normalization. Frontier and
  continuation reject conflicts or changed seals rather than pick a convenient
  file/date. Historical prefix rows and V3 registry bindings are compared.
- **Independent stages:** The Compose overlay and binaries separate producer
  UID10002/no DB or provider authority, admitter UID10003/direct dedicated SQL
  login, and worker UID10001. Producer artifacts alone do not confer admission;
  computation requires the reopened admitted capability and publication repeats
  SQL authority in its atomic settlement transaction. No public FD extraction or
  provider reconstruction fallback was observed in the production paths read.
- **Time and dedup:** Activation/config epochs, exact daily permanent key,
  earliest eligible civil/session evidence and bounded poll/replay/schedule paths
  remain explicit. The pure supplier plan is not an activated provider loop or a
  new request allowance. Schedule/read projections distinguish 16:30 eligibility
  from 18:30 stale deadline; queue retry does not move the original data deadline.
- **Publication and UI:** Historical and daily input unions remain distinct;
  daily runs do not acquire invented historical result pins. Prior successful
  publication is retained when a newer input/run fails, waits or becomes stale.
  Browser response handling redirects typed session expiry, despite panel/form
  catch blocks. Manual historical publication and the five existing strategy
  equations remain separate from daily input commitments.
- **Release/recovery:** The installer keeps no-clobber immutable release behavior,
  reader-floor checks and release locks. Six-file backup transition preserves
  old bytes and journaled recovery; it does not restore a database. Cold recovery
  captures a stopped complete schema59 cluster with writer fencing, protected
  roots, original audit/catalog/checksums and a networkless copied-cluster verifier.
  The native negative fixtures assert their intended error stages rather than
  accepting arbitrary Docker failure. Their hosted success does not establish
  real host ownership, downtime permission or a production restore result.

These observations are not exhaustive contract proofs. In particular, the
complete wire canonicalization implementation, S1 legacy-preservation/test coverage,
large migration-contract suite and many regression bodies remain unreviewed here.

## 4. Read-only verification and hosted evidence limits

Actual checks performed:

1. Stdlib SHA256/byte/line comparison of all 211 frozen pins and the inventory
   itself, before review and again during report preparation: all matched.
   Final pin validation is recorded in section 7.
2. Read-only `git -c core.hooksPath=/dev/null` candidate/baseline tree, blob and
   diff inspection. All 202 inventory blob bindings and candidate blob SHA256
   matched; the changed-path counts were 202 target paths / 386 including docs; target delta +69,033/-700 lines.
3. Read source and scoped diffs as enumerated in Appendix A; followed concrete
   adjacent call paths and runbooks rather than treating implementer conclusions
   as independent evidence. No source/parser/helper or test was executed.
4. Historical V2 and V3 approval registry blobs are identical at baseline and
   candidate. V2 SHA256 `4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380`;
   V3 SHA256 `5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707`.
   Also compared unchanged V3 materializer, KIS provider and approval baseline blobs.
   Also byte-compared 14 concrete adjacent source/reference files with their
   candidate blobs: all matched (browser response/session helpers, product gate,
   API session extraction, KIS unit installer, identity/grant/RLS migrations,
   the two approval registries, V3 materializer/provider/approval implementation).
   This is preservation evidence, not a fresh whole-implementation review.
   D4 remains 950 lines / SHA256 `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f`;
   D5 remains 740 lines / SHA256 `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6`.
   Relevant D4/D5 source-selection, authority and supply sections were read;
   a complete fresh line-by-line contract reread was not performed in this WP-7 pass.
5. Read both complete frozen PUML drafts. Independently resolved 303 citation
   records / 473 individual source ranges across 130 candidate files: all line
   bounds valid. Followed the new R2 edges in the inspected source. Complete
   semantic re-attestation of all inherited diagram edges was not completed.
6. Read frozen CI report/disposition and inspected structured evidence: exact
   candidate, attempt1, eight successful jobs, seven execution checkout SHAs,
   eight recorded full-log digests and 100 unique individually PASS first-rollout
   definitions with actual result rows. Did not fetch or reread raw hosted logs.
7. Report whitespace and local-link/line-bound validation: final results below.

The structured hosted record is
[ci-evidence.json](../../target/r2-daily-beta/ci/f215665/ci-evidence.json), SHA256
`d1560cf61a53b0790c5623a0a91fbf4c106e4bf9048742c5abebbe619b26cd67`;
its [disposition](../../target/r2-daily-beta/ci/f215665/coordinator-disposition.json)
is SHA256 `bf234ecfd0b416ecf67577d77c1911e66109261ced97e180d7dcc1173bc278aa`.
CI38901117928 attempt1 completed SUCCESS at `2026-10-16T12:55:45Z`.
The coordinator's 1,137-pin / 506-blob check is preserved as hosted collection
evidence; this reviewer independently checked the 211 review pins / 202 source
blobs, not all 1,137 pins. The four native cold-recovery cases and finite observed
source/restored `999:0/0700` tuples remain isolated fixture evidence.

Two fragmented non-required kis-client results, nested helper independent rows
and exits, doctest-name extraction, ignored/skipped/excluded cases retain the
receipt's explicit limits. Old failed runs are not overwritten. No hosted run
was queried, dispatched, rerun, cancelled or duplicated by this reviewer.
Local compilation, lint, tests, database and runtime checks were **NOT EXECUTED**.
No rustfmt check was needed or run; existing hosted formatting is separately
recorded evidence. Hash matching and formatting do not prove compilation.

One inert summary expression initially called `len()` on the inventory's integer
changed-path count after successfully validating all pins. It failed with
`TypeError`; corrected direct-field inspection returned 386. Several scoped
path lookups required correcting file names. These were read-only inspection
errors, not missing required inputs or executed project failures.

**D01 — stale harness evidence template (documentation/evidence gap):**
[EVIDENCE_TEMPLATE.md:81](../../deploy/db/integration-validation/EVIDENCE_TEMPLATE.md#L81)
still prescribes full-release `60 / 60` twice, while the actual candidate
[validator:1417](../../deploy/db/integration-validation/validate.sh#L1417)
requires `62:62` and current hosted evidence records 62. The staged 0060
lane correctly remains 60. README retains its older 0060-specific scope.
Do not use the old full-release template numbers as this candidate's admission
criteria; reconcile the template separately without rewriting staged/historical
evidence. This is not a failed current migration run or an additional P1/P2
source finding.

## 5. Structural, operational and acceptance gates still open

Both frozen draft SHA256 values matched:

- Component: `301282ab625accc3d4582f704a3309ffd75337f8d77eae409dbad104b1b40fd9`,
  363 lines.
- Runtime: `30cf83909cb0ef770da3f87de6a47b669d925f3bc52d17beb974e936b88bb40f`,
  448 lines.

The frozen preparation receipt says not rendered/integrated. During review the
coordinator appended a [resumption follow-up](2026-10-17-r2-resumption.md) reporting
docs-only `7271fdf` integration and corrected uncropped local rendering. That is
newer bookkeeping, not frozen candidate source mutation. Independently hashed
both current repository PUML files: they are byte-identical to the frozen drafts.
This reviewer checked the PUML text/ranges, not those PNGs or their visual layout. The same
follow-up reports image plan/preflight success and zero existing separate
`research-smoke.yml` runs. Those newer preparation claims were read from the
document, not independently executed or fetched here; the separate adopted
functional-smoke gate remains open.

The coordinator's prepared `sudo -n id -u` probe returned1; its exact denial reason
is unknown. This reviewer made no host query or permission request. Subsequent
reported unprivileged observations do not supply protected manifests/state,
complete OOM visibility or normal image health admission. The old installed
release remains selected; the historically stopped research worker and required
fresh bounded Owner V2 drain/resume packet are not waived by source CI success.

Still required, without initiating any of it here:

- Close the four findings and residual independent review scope; bind any changed
  candidate and its required verification afresh.
- Resolve concrete G4 commands, exact date/request budgets, window/calendar
  preparer and deadline, maintenance/writer fence, interruption and resume owner.
- Complete separate exact-candidate functional smoke under coordinator control,
  protected host/image admission and actual immutable image build/manifest checks.
- Perform authorized schema59 cold capture/isolated restore proof, approved V3
  host whole-flow, six-file backup transition and exact mode/role/ACL/checksum
  observations before operational admission. Current-link change is not DB DOWN
  or rollback, and no automatic production DB restore is approved by this report.
- Establish actual first ETF daily publication, normal Auth0 Owner V2/ETF use,
  same-input API/UI dates and three consecutive unattended trading days for the
  accepted candidate. R2-08 and final `C_R2` remain OPEN.

## 6. Scope deviations, residual work and ownership

**Deviation:** Exhaustive review of all 202 files / 174,410 lines is incomplete
within the fixed reporting bound. The review prioritized integrated production
flows and first-rollout/recovery controls, while preserving explicit residual
paths/ranges below. It did not silently narrow the assigned R2 scope or count
hosted result names as manual source/test review. No second reviewer or follow-on
task was launched.

Necessary residual review includes: full wire/SQL canonical parity and S1 legacy-preservation/adversarial
coverage; unchecked artifact/frontier/replay/snapshot test bodies; complete
migration/bootstrap/rollback/adversarial contract fixtures; remaining policy/helper and PG-validator
execution wiring; generated OpenAPI consistency; remaining browser/E2E and
unchanged Owner V2 generation/chart/Auth0 call paths. Appendix A names every
target still containing unread source or unread changed hunks. Findings need
bounded corrections and targeted regression evidence before disposition.

Missing required files or pin mismatches: **none**. Conflicting current authority
or immutable pin requirements: **none**. Documentation expectation mismatch: **D01**,
the old full-release template count; it does not change the current 62-migration
source/CI binding. Historical preparation fields and dated WRITTEN_UNEXECUTED
runbook checkpoints are not newer execution evidence.
Unverified: residual source/test paths, independent raw-log recovery, complete
diagram semantics/PNGs, local compiler/lint/tests, non-Linux behavior, actual
cross-principal/ACL deployment, all image/host/operational/Owner/trading-day gates.
New architectural requirement established: **none**.
Further decomposition needed: **yes**, the enumerated residual review exceeds
this single bounded pass; no decomposition or additional work is authorized or
started by this report. Background work: **none**. Pending permissions or waits
for this reviewer: **none**. Auto-review denials: **none**. Memory writes: **none**.
Only this report is written; no source, other document, Git or operational change.

## Appendix A. Exact changed-target inspection ledger

`F` means the complete file text was read. It does not mean every test oracle was
proved or executed. `D` means all changed hunks were read with relevant context;
the unchanged remainder was not fully reviewed. `P` gives the inspected ranges
or portions; all other body/hunks remain unreviewed. `U` means pin/blob identity
only, with **all source/test body and semantic comparison unreviewed**. Generated
files and tests are included, not silently excluded. A D/P/U row is residual
scope for any claim of a full-file integrated audit.

| Frozen target (candidate line count) | Inspection | Explicit scope / residual |
| --- | --- | --- |
| [.github/workflows/ci.yml](../../.github/workflows/ci.yml) (274) | F | All 274 lines read; execution not performed |
| [apps/api-server/generated/openapi.ts](../../apps/api-server/generated/openapi.ts) (5873) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/api-server/openapi.json](../../apps/api-server/openapi.json) (16235) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/api-server/scripts/openapi-spec.mjs](../../apps/api-server/scripts/openapi-spec.mjs) (3579) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/app/(authenticated)/recommendations/page.tsx](../../apps/web/app/(authenticated)/recommendations/page.tsx) (438) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/app/(authenticated)/strategies/page.tsx](../../apps/web/app/(authenticated)/strategies/page.tsx) (95) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/components/recommendations/owner-beta-daily-panel.tsx](../../apps/web/components/recommendations/owner-beta-daily-panel.tsx) (171) | F | All 171 lines read; execution not performed |
| [apps/web/components/recommendations/owner-beta-history.tsx](../../apps/web/components/recommendations/owner-beta-history.tsx) (72) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/components/recommendations/owner-beta-report.tsx](../../apps/web/components/recommendations/owner-beta-report.tsx) (374) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/components/strategies/strategy-catalog.tsx](../../apps/web/components/strategies/strategy-catalog.tsx) (70) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/components/strategies/strategy-config-form.tsx](../../apps/web/components/strategies/strategy-config-form.tsx) (252) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/components/strategies/strategy-daily-control.tsx](../../apps/web/components/strategies/strategy-daily-control.tsx) (83) | F | All 83 lines read; execution not performed |
| [apps/web/lib/api/contracts.ts](../../apps/web/lib/api/contracts.ts) (130) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/api/owner-beta-daily.ts](../../apps/web/lib/api/owner-beta-daily.ts) (40) | F | All 40 lines read; execution not performed |
| [apps/web/lib/api/product-client.ts](../../apps/web/lib/api/product-client.ts) (370) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/i18n/dictionaries/recommendations.ts](../../apps/web/lib/i18n/dictionaries/recommendations.ts) (609) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/i18n/dictionaries/strategies.ts](../../apps/web/lib/i18n/dictionaries/strategies.ts) (146) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/products/contracts.ts](../../apps/web/lib/products/contracts.ts) (195) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/products/owner-beta-contracts.ts](../../apps/web/lib/products/owner-beta-contracts.ts) (403) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/lib/products/owner-beta-daily-contracts.ts](../../apps/web/lib/products/owner-beta-daily-contracts.ts) (108) | F | All 108 lines read; execution not performed |
| [apps/web/lib/products/owner-beta-history.ts](../../apps/web/lib/products/owner-beta-history.ts) (89) | F | All 89 lines read; execution not performed |
| [apps/web/lib/products/paper-contracts.ts](../../apps/web/lib/products/paper-contracts.ts) (338) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [apps/web/tests/e2e/owner-beta-recommendations.spec.ts](../../apps/web/tests/e2e/owner-beta-recommendations.spec.ts) (441) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/e2e/support/recommendation-fixture.mjs](../../apps/web/tests/e2e/support/recommendation-fixture.mjs) (548) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/e2e/support/synthetic-api.mjs](../../apps/web/tests/e2e/support/synthetic-api.mjs) (324) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/owner-beta-recommendation-surface.test.tsx](../../apps/web/tests/owner-beta-recommendation-surface.test.tsx) (490) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/r2-daily-api-contracts.test.ts](../../apps/web/tests/r2-daily-api-contracts.test.ts) (214) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/r2-daily-ui-browser.test.ts](../../apps/web/tests/r2-daily-ui-browser.test.ts) (131) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/r2-daily-ui-contracts.test.ts](../../apps/web/tests/r2-daily-ui-contracts.test.ts) (122) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/r2-daily-ui-history.test.ts](../../apps/web/tests/r2-daily-ui-history.test.ts) (137) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/r2-daily-ui-surface.test.tsx](../../apps/web/tests/r2-daily-ui-surface.test.tsx) (94) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [apps/web/tests/strategy-catalog-contract.test.tsx](../../apps/web/tests/strategy-catalog-contract.test.tsx) (63) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [configs/evidence/owner-beta-daily-price-policy-v1.json](../../configs/evidence/owner-beta-daily-price-policy-v1.json) (1) | F | All 1 lines read; execution not performed |
| [configs/evidence/owner-beta-daily-reader-v1.json](../../configs/evidence/owner-beta-daily-reader-v1.json) (1) | F | All 1 lines read; execution not performed |
| [crates/api-server/src/bin/paper-runner.rs](../../crates/api-server/src/bin/paper-runner.rs) (871) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/contract.rs](../../crates/api-server/src/contract.rs) (1617) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/http/dto.rs](../../crates/api-server/src/http/dto.rs) (874) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/http/mod.rs](../../crates/api-server/src/http/mod.rs) (770) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/http/owner_beta.rs](../../crates/api-server/src/http/owner_beta.rs) (921) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/http/owner_beta_daily.rs](../../crates/api-server/src/http/owner_beta_daily.rs) (63) | F | All 63 lines read; execution not performed |
| [crates/api-server/src/http/state.rs](../../crates/api-server/src/http/state.rs) (591) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/http/strategies.rs](../../crates/api-server/src/http/strategies.rs) (374) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/repos/owner_beta.rs](../../crates/api-server/src/repos/owner_beta.rs) (1752) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/api-server/src/repos/owner_beta/daily.rs](../../crates/api-server/src/repos/owner_beta/daily.rs) (736) | F | All 736 lines read; execution not performed |
| [crates/api-server/src/repos/strategy_configs.rs](../../crates/api-server/src/repos/strategy_configs.rs) (156) | F | All 156 lines read; execution not performed |
| [crates/api-server/src/runtime.rs](../../crates/api-server/src/runtime.rs) (2794) | P | daily-mode configuration delta and 545–586 context; full existing runtime and all test hunks not attested |
| [crates/api-server/tests/common/mod.rs](../../crates/api-server/tests/common/mod.rs) (945) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/http_owner_beta_access.rs](../../crates/api-server/tests/http_owner_beta_access.rs) (327) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/http_owner_beta_daily.rs](../../crates/api-server/tests/http_owner_beta_daily.rs) (438) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/owner_beta_daily_support/mod.rs](../../crates/api-server/tests/owner_beta_daily_support/mod.rs) (462) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/owner_equity_v2_db.rs](../../crates/api-server/tests/owner_equity_v2_db.rs) (1472) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/owner_market_stream_http_support/mod.rs](../../crates/api-server/tests/owner_market_stream_http_support/mod.rs) (2378) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/owner_market_stream_integrated_support/mod.rs](../../crates/api-server/tests/owner_market_stream_integrated_support/mod.rs) (2409) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/service_status_reads.rs](../../crates/api-server/tests/service_status_reads.rs) (1719) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/api-server/tests/service_status_reads_support/mod.rs](../../crates/api-server/tests/service_status_reads_support/mod.rs) (1532) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/factor-engine/src/bars.rs](../../crates/factor-engine/src/bars.rs) (527) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/factor-engine/src/daily_price_only.rs](../../crates/factor-engine/src/daily_price_only.rs) (272) | F | All 272 lines read; execution not performed |
| [crates/factor-engine/src/lib.rs](../../crates/factor-engine/src/lib.rs) (63) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/factor-engine/src/price_only.rs](../../crates/factor-engine/src/price_only.rs) (512) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/factor-engine/tests/daily_price_only.rs](../../crates/factor-engine/tests/daily_price_only.rs) (120) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/src/bin/owner-beta-runner.rs](../../crates/job-queue/src/bin/owner-beta-runner.rs) (578) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/bin/owner-beta-runner/daily.rs](../../crates/job-queue/src/bin/owner-beta-runner/daily.rs) (403) | F | All 403 lines read; execution not performed |
| [crates/job-queue/src/bin/owner-equity-v2-runner.rs](../../crates/job-queue/src/bin/owner-equity-v2-runner.rs) (2005) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/owner_beta/daily_compute.rs](../../crates/job-queue/src/owner_beta/daily_compute.rs) (91) | F | All 91 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/daily_input.rs](../../crates/job-queue/src/owner_beta/daily_input.rs) (406) | F | All 406 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/daily_target.rs](../../crates/job-queue/src/owner_beta/daily_target.rs) (177) | F | All 177 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/mod.rs](../../crates/job-queue/src/owner_beta/mod.rs) (44) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/owner_beta/publish.rs](../../crates/job-queue/src/owner_beta/publish.rs) (1614) | P | daily production addition 746–1310, error variants, changed-hunk unit assertions; not full historical body or every fixture helper |
| [crates/job-queue/src/owner_beta/recovery.rs](../../crates/job-queue/src/owner_beta/recovery.rs) (711) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/owner_beta/runner.rs](../../crates/job-queue/src/owner_beta/runner.rs) (671) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/owner_beta/runner/daily_admission.rs](../../crates/job-queue/src/owner_beta/runner/daily_admission.rs) (820) | P | 1–699 production; private tests 700–820 unread |
| [crates/job-queue/src/owner_beta/runner/daily_catalog.rs](../../crates/job-queue/src/owner_beta/runner/daily_catalog.rs) (363) | F | All 363 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/runner/daily_execution.rs](../../crates/job-queue/src/owner_beta/runner/daily_execution.rs) (554) | F | All 554 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/runner/daily_ingest.rs](../../crates/job-queue/src/owner_beta/runner/daily_ingest.rs) (1392) | F | All 1392 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/runner/daily_local_state.rs](../../crates/job-queue/src/owner_beta/runner/daily_local_state.rs) (1005) | P | 1–683 production and initial fixture context; remaining tests unread |
| [crates/job-queue/src/owner_beta/runner/daily_runtime.rs](../../crates/job-queue/src/owner_beta/runner/daily_runtime.rs) (130) | F | All 130 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/runner/daily_schedule.rs](../../crates/job-queue/src/owner_beta/runner/daily_schedule.rs) (191) | F | All 191 lines read; execution not performed |
| [crates/job-queue/src/owner_beta/target.rs](../../crates/job-queue/src/owner_beta/target.rs) (2317) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/src/paper_io.rs](../../crates/job-queue/src/paper_io.rs) (233) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/job-queue/tests/common/mod.rs](../../crates/job-queue/tests/common/mod.rs) (401) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_backup.rs](../../crates/job-queue/tests/owner_beta_daily_backup.rs) (375) | P | 148–177 actual-role users-RLS audit/repair assertions; remaining tests/helpers unread |
| [crates/job-queue/tests/owner_beta_daily_compute.rs](../../crates/job-queue/tests/owner_beta_daily_compute.rs) (505) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_publish.rs](../../crates/job-queue/tests/owner_beta_daily_publish.rs) (304) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_recovery.rs](../../crates/job-queue/tests/owner_beta_daily_recovery.rs) (304) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_runner.rs](../../crates/job-queue/tests/owner_beta_daily_runner.rs) (1118) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_schedule.rs](../../crates/job-queue/tests/owner_beta_daily_schedule.rs) (1505) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_daily_support/mod.rs](../../crates/job-queue/tests/owner_beta_daily_support/mod.rs) (389) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/job-queue/tests/owner_beta_publish.rs](../../crates/job-queue/tests/owner_beta_publish.rs) (1372) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/kis-client/tests/market_stream_transport.rs](../../crates/kis-client/tests/market_stream_transport.rs) (3898) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/src/daily_price_only/admission.rs](../../crates/market-data/src/daily_price_only/admission.rs) (487) | P | 1–352 production and initial fixture context; remaining tests unread |
| [crates/market-data/src/daily_price_only/artifact.rs](../../crates/market-data/src/daily_price_only/artifact.rs) (2316) | P | 1–1468 production; tests 1469–2316 not fully inspected |
| [crates/market-data/src/daily_price_only/calendar.rs](../../crates/market-data/src/daily_price_only/calendar.rs) (436) | F | All 436 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/candidate.rs](../../crates/market-data/src/daily_price_only/candidate.rs) (302) | P | 1–180 production; tests 181–302 unread |
| [crates/market-data/src/daily_price_only/continuation.rs](../../crates/market-data/src/daily_price_only/continuation.rs) (852) | P | 1–552 production; tests 553–852 unread |
| [crates/market-data/src/daily_price_only/contract.rs](../../crates/market-data/src/daily_price_only/contract.rs) (4206) | P | 1–110, 670–726, 2257–2425 and 3010–3410; other implementation/tests unread |
| [crates/market-data/src/daily_price_only/discovery.rs](../../crates/market-data/src/daily_price_only/discovery.rs) (434) | F | All 434 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/frontier.rs](../../crates/market-data/src/daily_price_only/frontier.rs) (1273) | P | 1–817 production; tests 818–1273 unread |
| [crates/market-data/src/daily_price_only/history.rs](../../crates/market-data/src/daily_price_only/history.rs) (1436) | P | 1–733 production; tests 734–1436 unread |
| [crates/market-data/src/daily_price_only/input.rs](../../crates/market-data/src/daily_price_only/input.rs) (56) | F | All 56 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/mod.rs](../../crates/market-data/src/daily_price_only/mod.rs) (23) | F | All 23 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/observation.rs](../../crates/market-data/src/daily_price_only/observation.rs) (639) | P | 1–330, including complete production 1–285; remaining tests unread |
| [crates/market-data/src/daily_price_only/price_extension.rs](../../crates/market-data/src/daily_price_only/price_extension.rs) (190) | F | All 190 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/reconstruct.rs](../../crates/market-data/src/daily_price_only/reconstruct.rs) (764) | F | All 764 lines read; execution not performed |
| [crates/market-data/src/daily_price_only/replay.rs](../../crates/market-data/src/daily_price_only/replay.rs) (944) | P | 1–381 production; tests 382–944 unread |
| [crates/market-data/src/daily_price_only/source.rs](../../crates/market-data/src/daily_price_only/source.rs) (4448) | P | 1–2764 production/instrumentation and 4380–4448 tail; private tests not fully reviewed |
| [crates/market-data/src/daily_price_only/supply.rs](../../crates/market-data/src/daily_price_only/supply.rs) (440) | P | 1–198 pure planning; tests 199–440 unread |
| [crates/market-data/src/daily_price_only/verify.rs](../../crates/market-data/src/daily_price_only/verify.rs) (445) | F | All 445 lines read; execution not performed |
| [crates/market-data/src/lib.rs](../../crates/market-data/src/lib.rs) (288) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/market-data/src/normalize.rs](../../crates/market-data/src/normalize.rs) (2024) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [crates/market-data/src/storage.rs](../../crates/market-data/src/storage.rs) (3656) | P | 75–350 types/errors, 350–581 control guards, 800–1060 opens, 1470–1613 bounded API, 1840–1998 legacy orphan context, 2250–2465 parser; other legacy code/private tests unread |
| [crates/market-data/tests/daily_price_only.rs](../../crates/market-data/tests/daily_price_only.rs) (2435) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/tests/daily_price_only_discovery.rs](../../crates/market-data/tests/daily_price_only_discovery.rs) (818) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/tests/daily_price_only_evidence.rs](../../crates/market-data/tests/daily_price_only_evidence.rs) (887) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/tests/daily_price_only_reconstruction.rs](../../crates/market-data/tests/daily_price_only_reconstruction.rs) (1402) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/tests/daily_price_only_snapshot.rs](../../crates/market-data/tests/daily_price_only_snapshot.rs) (1972) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [crates/market-data/tests/daily_price_only_storage.rs](../../crates/market-data/tests/daily_price_only_storage.rs) (644) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [data-pipelines/collectors/src/pipeline.rs](../../data-pipelines/collectors/src/pipeline.rs) (1226) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [data-pipelines/collectors/src/worker.rs](../../data-pipelines/collectors/src/worker.rs) (6797) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [data-pipelines/collectors/tests/service_status.rs](../../data-pipelines/collectors/tests/service_status.rs) (1352) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [deploy/compose/.env.example](../../deploy/compose/.env.example) (149) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/compose/compose.owner-beta-daily.yml](../../deploy/compose/compose.owner-beta-daily.yml) (228) | F | All 228 lines read; execution not performed |
| [deploy/compose/compose.yml](../../deploy/compose/compose.yml) (1636) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/db/Dockerfile](../../deploy/db/Dockerfile) (33) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/db/bootstrap-roles.sh](../../deploy/db/bootstrap-roles.sh) (303) | F | All 303 lines read; execution not performed |
| [deploy/db/daily-backup-verify.sh](../../deploy/db/daily-backup-verify.sh) (41) | F | All 41 lines read; execution not performed |
| [deploy/db/daily-control.sh](../../deploy/db/daily-control.sh) (31) | F | All 31 lines read; execution not performed |
| [deploy/db/integration-validation/EVIDENCE_TEMPLATE.md](../../deploy/db/integration-validation/EVIDENCE_TEMPLATE.md) (102) | F | All 102 lines read; execution not performed |
| [deploy/db/integration-validation/README.md](../../deploy/db/integration-validation/README.md) (143) | F | All 143 lines read; execution not performed |
| [deploy/db/integration-validation/actual-role-0060.sql](../../deploy/db/integration-validation/actual-role-0060.sql) (31) | F | All 31 lines read; execution not performed |
| [deploy/db/integration-validation/bootstrap-negative.sh](../../deploy/db/integration-validation/bootstrap-negative.sh) (353) | F | All 353 lines read; execution not performed |
| [deploy/db/integration-validation/preflight-0060.sql](../../deploy/db/integration-validation/preflight-0060.sql) (78) | F | All 78 lines read; execution not performed |
| [deploy/db/integration-validation/rollback-0060-guard.sql](../../deploy/db/integration-validation/rollback-0060-guard.sql) (50) | F | All 50 lines read; execution not performed |
| [deploy/db/integration-validation/static-check.sh](../../deploy/db/integration-validation/static-check.sh) (138) | F | All 138 lines read; execution not performed |
| [deploy/db/integration-validation/validate.sh](../../deploy/db/integration-validation/validate.sh) (2060) | P | 109–187 bootstrap recording gate and initial self-test delta; 1417/1480 ledger predicates; later bootstrap/0060/test wiring and unchanged remainder unread |
| [deploy/db/migrate-static-check.sh](../../deploy/db/migrate-static-check.sh) (264) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/db/owner-beta-daily-backup-audit.sql](../../deploy/db/owner-beta-daily-backup-audit.sql) (191) | F | All 191 lines read; execution not performed |
| [deploy/db/owner-beta-daily-control-status.sql](../../deploy/db/owner-beta-daily-control-status.sql) (81) | F | All 81 lines read; execution not performed |
| [deploy/db/owner-beta-daily-control.sql](../../deploy/db/owner-beta-daily-control.sql) (99) | F | All 99 lines read; execution not performed |
| [deploy/db/owner-beta-daily-reservation-restore.sql](../../deploy/db/owner-beta-daily-reservation-restore.sql) (129) | F | All 129 lines read; execution not performed |
| [deploy/db/preupgrade-recovery-audit.sql](../../deploy/db/preupgrade-recovery-audit.sql) (94) | F | All 94 lines read; execution not performed |
| [deploy/db/preupgrade-recovery-verify.sh](../../deploy/db/preupgrade-recovery-verify.sh) (49) | F | All 49 lines read; execution not performed |
| [deploy/secrets/provision-runtime-secrets.sh](../../deploy/secrets/provision-runtime-secrets.sh) (343) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/secrets/runtime-static-check.sh](../../deploy/secrets/runtime-static-check.sh) (1023) | P | selected changed staging-map/0440 serving-secret assertions; embedded inventory parser/regressions and remaining hunks unread |
| [deploy/systemd/lagrange-production-backup-verify.service](../../deploy/systemd/lagrange-production-backup-verify.service) (29) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/systemd/lagrange-production-backup.service](../../deploy/systemd/lagrange-production-backup.service) (29) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [deploy/systemd/production-backup.conf.example](../../deploy/systemd/production-backup.conf.example) (17) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [migrations/0060_owner_beta_daily_inputs.down.sql](../../migrations/0060_owner_beta_daily_inputs.down.sql) (305) | F | All 305 lines read; execution not performed |
| [migrations/0060_owner_beta_daily_inputs.up.sql](../../migrations/0060_owner_beta_daily_inputs.up.sql) (1809) | F | All 1809 lines read; execution not performed |
| [migrations/0061_owner_beta_daily_supersession.down.sql](../../migrations/0061_owner_beta_daily_supersession.down.sql) (4) | F | All 4 lines read; execution not performed |
| [migrations/0061_owner_beta_daily_supersession.up.sql](../../migrations/0061_owner_beta_daily_supersession.up.sql) (159) | F | All 159 lines read; execution not performed |
| [migrations/0062_owner_beta_daily_admitter_readback.down.sql](../../migrations/0062_owner_beta_daily_admitter_readback.down.sql) (6) | F | All 6 lines read; execution not performed |
| [migrations/0062_owner_beta_daily_admitter_readback.up.sql](../../migrations/0062_owner_beta_daily_admitter_readback.up.sql) (174) | F | All 174 lines read; execution not performed |
| [nt/live-node/tests/test_runtime.py](../../nt/live-node/tests/test_runtime.py) (479) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ci/test_ci_contract.py](../../scripts/ci/test_ci_contract.py) (564) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/backfill-production-self-test.sh](../../scripts/ops/backfill-production-self-test.sh) (630) | P | 103–114 substituted validator fixture; other fixture bodies unread |
| [scripts/ops/backfill-production.sh](../../scripts/ops/backfill-production.sh) (728) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/compose-release.sh](../../scripts/ops/compose-release.sh) (869) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/deploy-production-release.sh](../../scripts/ops/deploy-production-release.sh) (439) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/install-production-backup.sh](../../scripts/ops/install-production-backup.sh) (174) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/kis-daily-production-self-test.sh](../../scripts/ops/kis-daily-production-self-test.sh) (477) | P | 118–145 immutable-release helper/validator stubs; other fixture bodies unread |
| [scripts/ops/kis-daily-production.sh](../../scripts/ops/kis-daily-production.sh) (625) | F | All 625 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-backup-files.py](../../scripts/ops/lib/owner-beta-daily-backup-files.py) (642) | F | All 642 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-backup.py](../../scripts/ops/lib/owner-beta-daily-backup.py) (905) | F | All 905 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-context.sh](../../scripts/ops/lib/owner-beta-daily-context.sh) (36) | F | All 36 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-operations.py](../../scripts/ops/lib/owner-beta-daily-operations.py) (803) | F | All 803 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-release.py](../../scripts/ops/lib/owner-beta-daily-release.py) (652) | F | All 652 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-release.sh](../../scripts/ops/lib/owner-beta-daily-release.sh) (44) | F | All 44 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-runtime.py](../../scripts/ops/lib/owner-beta-daily-runtime.py) (437) | F | All 437 lines read; execution not performed |
| [scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py](../../scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py) (192) | F | All 192 lines read; execution not performed |
| [scripts/ops/lib/production-preupgrade-recovery.py](../../scripts/ops/lib/production-preupgrade-recovery.py) (317) | F | All 317 lines read; execution not performed |
| [scripts/ops/lib/release-image-manifest.sh](../../scripts/ops/lib/release-image-manifest.sh) (374) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/owner-beta-daily-backup-docker-self-test.py](../../scripts/ops/owner-beta-daily-backup-docker-self-test.py) (342) | P | 1–160 and invalid-dump/normal-cleanup context around 324–330; other bodies unread |
| [scripts/ops/owner-beta-daily-backup-self-test.py](../../scripts/ops/owner-beta-daily-backup-self-test.py) (870) | P | 612 mock setup and controller/reconciliation context around 670–710; other bodies unread |
| [scripts/ops/owner-beta-daily-control-self-test.py](../../scripts/ops/owner-beta-daily-control-self-test.py) (155) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-docker-self-test.py](../../scripts/ops/owner-beta-daily-docker-self-test.py) (292) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-integration-self-test.py](../../scripts/ops/owner-beta-daily-integration-self-test.py) (304) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-operations-self-test.py](../../scripts/ops/owner-beta-daily-operations-self-test.py) (398) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-release-self-test.py](../../scripts/ops/owner-beta-daily-release-self-test.py) (370) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-runtime-self-test.py](../../scripts/ops/owner-beta-daily-runtime-self-test.py) (356) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/owner-beta-daily-transport-self-test.py](../../scripts/ops/owner-beta-daily-transport-self-test.py) (351) | P | 217–260 bootstrap/actor-RLS fixtures; remaining body unread |
| [scripts/ops/owner-beta-daily-wholeflow-fixture-self-test.py](../../scripts/ops/owner-beta-daily-wholeflow-fixture-self-test.py) (274) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/prepare-production-recovery.py](../../scripts/ops/prepare-production-recovery.py) (563) | F | All 563 lines read; execution not performed |
| [scripts/ops/production-backup-transition-self-test.py](../../scripts/ops/production-backup-transition-self-test.py) (431) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/production-ops-self-test.sh](../../scripts/ops/production-ops-self-test.sh) (1251) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/production-ops-static-check.sh](../../scripts/ops/production-ops-static-check.sh) (416) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/production-preupgrade-maintenance-self-test.py](../../scripts/ops/production-preupgrade-maintenance-self-test.py) (461) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/production-preupgrade-recovery-docker-self-test.py](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py) (411) | F | All 411 lines read; execution not performed |
| [scripts/ops/production-preupgrade-recovery-self-test.py](../../scripts/ops/production-preupgrade-recovery-self-test.py) (376) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/provision-db-secrets.sh](../../scripts/ops/provision-db-secrets.sh) (613) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/run-production-backup.sh](../../scripts/ops/run-production-backup.sh) (394) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/self-test.sh](../../scripts/ops/self-test.sh) (1483) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [scripts/ops/transition-production-backup.py](../../scripts/ops/transition-production-backup.py) (422) | F | All 422 lines read; execution not performed |
| [scripts/ops/validate-production-config.sh](../../scripts/ops/validate-production-config.sh) (899) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/ops/verify-owner-beta-daily-wholeflow.py](../../scripts/ops/verify-owner-beta-daily-wholeflow.py) (717) | F | All 717 lines read; execution not performed |
| [scripts/qa/candidate-web-e2e-runner.py](../../scripts/qa/candidate-web-e2e-runner.py) (2276) | D | All changed hunks/context read; unchanged body not fully reviewed |
| [scripts/qa/test_candidate_web_e2e_runner.py](../../scripts/qa/test_candidate_web_e2e_runner.py) (1933) | P | changed-test fragments for finite error/location/step redaction; full new fixture/test delta not reviewed |
| [tests/fixtures/owner-beta-daily-install/admitter-checkpoint.json](../../tests/fixtures/owner-beta-daily-install/admitter-checkpoint.json) (1) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [tests/fixtures/owner-beta-daily-install/config.json](../../tests/fixtures/owner-beta-daily-install/config.json) (1) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [tests/fixtures/owner-beta-daily-install/producer-checkpoint.json](../../tests/fixtures/owner-beta-daily-install/producer-checkpoint.json) (1) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [tests/integration/migration-contract/bootstrap.sql](../../tests/integration/migration-contract/bootstrap.sql) (33) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [tests/integration/migration-contract/role-bootstrap.sql](../../tests/integration/migration-contract/role-bootstrap.sql) (94) | U | All body/hunks semantically unreviewed; pin/blob checked only |
| [tests/integration/migration-contract/tests/migration_contract.rs](../../tests/integration/migration-contract/tests/migration_contract.rs) (14081) | P | 12695–12746 calendar/copy projection assertions; remaining 14,000-line suite unread |

Ledger totals: F=65, D=56, P=26, U=55; 202/202 targets enumerated.

## 7. Final static closure record

Completion/static pin recheck: `2026-10-16T18:21:08Z`, before the 18:31:05.523Z bound.
All 211/211 frozen pins and the inventory hash still match; the final
candidate tree/blob check also matched all 202 bindings and source blob hashes.
Both current integrated PUML sources equal their frozen drafts; 14 concrete
adjacent source/reference byte comparisons matched the candidate.

All 202 target paths are enumerated: F=65, D=56, P=26, U=55. Full integrated
review is **INCOMPLETE**. Open findings are P1-01, P1-02, P2-01 and P2-02;
D01 is a separate stale-template gap. Appendix A is the exact residual
path/portion ledger. No residual work runs in the background.

Final local links/line bounds and whitespace checks passed; report ends
with LF and has no trailing whitespace. No compiler, tests, project
validators, network, host or lifecycle action was executed.
Changed artifact: `docs/reviews/2026-10-16-r2-integrated-release-review.md`,
lines 1–602. Its complete-file SHA256 is supplied in the final
callback, avoiding a self-referential hash embedded in the hashed file.
