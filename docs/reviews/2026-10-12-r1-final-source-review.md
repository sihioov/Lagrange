# R1 final source-coverage audit — WP-5 — 2026-10-12

**COV1_SOURCE_REVIEW_COMPLETE:90/90 paths source-covered at the frozen snapshot.
Initial90/85/5audit preserved below; final Web/smoke/hosted acceptance remains pending.**
This is not final WP-5/R1, hosted CI, image-build, deployment or Owner acceptance.
HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`; shared dirty candidate preserved.

## Findings and unresolved scope, ordered by priority

**COV1 / P2 — five DB harness post-initial deltas lack established independent source
coverage.** The initial WP-5 review pins different hashes for these files
(`2026-10-12-r1-review.md:275–279`). Current hashes match WP-3's post-correction
manifest and the accepted final DB execution inputs; those establish identity and
successful execution, not a subsequently inspected source delta. The retained WP-5
final-DB addendum1617–1729 reviews execution evidence and two test/SQL inputs; it does
not establish complete review of these five later harness/document deltas. Path
mentions and a passed validator were not promoted into source coverage.

| Uncovered current file | Baseline-diff scope / minimum focused follow-up |
| --- | --- |
| `deploy/db/integration-validation/README.md` |4 hunks at3–93; check corrected lane/evidence/cleanup claims against actual interface. |
| `deploy/db/integration-validation/preflight-0039.sql` |6 hunks at3–5,100–158; inspect supervisor terminal-Paper/RLS split while retaining migration-owner audit checks. |
| `deploy/db/integration-validation/preflight-catalog.sql` |New383-line file; initial reviewed version had a different hash. Establish the corrected lane, role/catalog/audit equality and transactional negative/rollback delta. |
| `deploy/db/integration-validation/static-check.sh` |6 hunks at19,52–107; inspect later marker/self-check changes without treating markers as behavioral proof. |
| `deploy/db/integration-validation/validate.sh` |52 baseline-relative hunks,15–1327. Focus post-initial evidence/summary299–445, catalog/role672–795, full-lane904–1008 and staged/teardown1120–1300. |

These are baseline envelopes, **not a claim that every listed hunk is new/unread**.
The private coverage file contains all exact zero-context headers. Minimum next
step is one bounded five-file read-only source-delta review, reconstructing the
initial reviewed snapshot where possible and pinning the current hashes. Where
that preimage cannot be established, explicitly cover the remaining corrected
hunks; do not infer acceptance. No source fix or runtime rerun is demanded solely
for this recordkeeping gap. No new product defect is asserted by COV1.

The original catalog F2 and optional O1 are not reopened as observed failures:
current catalog70–131/151–184 has exact five manifest IDs, staged six/full five
expectations and the named Paper exception; validator1277–1301 tears down/verifies
the full cluster before starting tests. These narrow checks plus the previously
accepted full DB run support their resolution. Other harness corrections remain
within COV1. DB1–DB7/test/SQL acceptance and historical failure evidence are retained.

**New behavior/security findings in the newly inspected narrow deltas: none.**
Private-supervisor **S1 resolved at source/fake level**, as detailed below.
Prior Rust/market-data/C2/API/Web source resolutions remain bound to their matching
snapshots. Web clock/slow-hydration/cleanup behavior and all remaining dynamic gates
are still pending; source clearance is not their execution result.

## Exact enumeration, snapshot and coverage map

Combined `git diff --name-only HEAD -z` with
`git ls-files --others --exclude-standard -z`, deduplicated, excluded only `docs/`.
Exactly90 paths:80 tracked changed +10 new untracked. Ignored target artifacts are
excluded. The deploy README is intentionally counted because it is outside docs/.
All90 safe file hashes and the path set matched at start, end and pre-report check.
Documentation appends by other workers are not candidate-source drift.

Private audit root: `target/r1-runtime-fxi6ltmv/wp5-source-coverage-20261012/`.
Every path is enumerated with full SHA256, package, source-review reference/scope,
status and retained baseline-diff digest/hunk headers in `coverage.json`.

| Artifact | SHA256 |
| --- | --- |
| `before.json` |`112876216f28c34124ef3de6beee2495cbe4acae521efb0fa372d695ddb5d55e` |
| `after.json` — canonical90-path/hash snapshot |`c35325382cb4ff8af0db06343c3a60384a5c52ded79d8c3a5f046082ba78ed67` |
| `coverage.json` —90 individual classifications |`11394319898d1feb3ff83d486754f7226e848e30c2884076aa2accf48469d8ec` |

**80 matching independently reviewed snapshots +5 checked narrow deltas +5 gaps.**
Matching hashes were reused only with the prior inspected scope and snapshot chain;
fullpath/hash mentions in implementation reports alone were not sufficient.

| Covered package | Paths | Reused inspected evidence |
| --- | ---: | --- |
| Rust A/B |21|`rust-lint-review.md`, complete425-hunk inspection and final hash table. |
| Rust cfg/SQL follow-up |2|Main review1733–1777; exact helper feature semantics and restored SQL bytes. |
| Workspace F/U repairs |11|`workspace-repair-review.md:1–238`, scoped hunk reads and snapshot tables. |
| Earlier Rust/collectors/market-data |21|Main initial review99–158/242–296 plus bounded test/helper addenda347–701. |
| C2 rotation |10|Main1900–1996; exact ten-file source diffs/hash table, prior guards retained. |
| DB contract/owner reconnect |2|Main703–1139/1315–1729; final test/SQL hash chain and six-line fix. |
| Web body/fixture + latest BODY/CLOCK |3|Main1315–1614/1779–1899 plus independent `web-followup-review.md`23-hunk review. |
| ENV1 |2|`web-env-smoke-review.md:1–123`, runner insertion/preimage identity and31-name inventory. |
| Web wrapper/off caller |3|Main1140–1314, full wrapper/batch helper and scoped historical off caller. |
| Serial smoke build/CI contract |2|Main1617–1729, reusing initial Raw/probe/functional source inspection. |
| Standalone locked QA probe |3|Main99–158/242–296, complete manifest/lock/source inspection. |
| Current narrow audit below |5|Collector port, CI port pair, JSON import, readiness digest and calendar alias. |

Five gap hashes equal all five entries of
`evidence-wp3.A8ziPI/source-files-final.sha256` and final validator
`evidence-wp3-final.M9YTnM/source-inputs-before.sha256`; this corroborates prior
local dynamic acceptance without manufacturing independent source inspection.

## Narrow current deltas and private S1

- **Collector F3:** `research_worker.rs:2369`, with caller/assertions2340–2397.
  Removing only the new DB_PORT55438 override reproduces prior independently
  reviewed whole-file SHA `43170279cac6bdcab71b61e883bd099bd5ca68a07d8b699b3b01859ae74a2d93`.
  Current SHA is `a2f4bdb5e31a1b1086694c168cb4880712e9d77009d1163d81e7822b464e0c00`.
  Exact QA guard2344–2347, Permanent2387 and zero price publication2396 remain;
  helper default1523 and pure configuration1550 remain55432. Completed exact and
  full-target logs match recorded hashes:1/70 passed,0 failed/ignored. No rerun.
- **CI:** `.github/workflows/ci.yml:104–108` aligns DATABASE_URL55438 and
  LAGRANGE_QA_DB_PORT55438 with QA Compose's loopback interpolation. Undoing only
  that later port/comment delta recovers initial reviewed SHA22cc7766…99e0c9af.
  Chromium placement and full workspace kis-client/test-support selection remain.
- **Web import:** `intraday-stream-universe.ts:1–3` adds only JSON import attributes;
  reverting that import exactly reproduces HEAD. All universe/admission logic is
  unchanged; current SHA d727c592…ba5453 is the reported final formatted snapshot.
- **Readiness:** `synthetic-api.mjs:19–27,130–148` is the entire28-added-line HEAD
  delta. Its canonical scoped SHA is the exact prior inspected
  `43807c27bf25dc51d8970da4d6e8fd8ded5276d6af9f23cfa9cf16d6755a8823`.
  Full-file SHA c95aa497… matches the runner report. No unseen fixture-body delta.
- **Calendar alias:** `intraday_calendar_read_state.rs:1187–1190` replaces only the
  duplicate source inclusion with the accepted shared alias; reversal equals HEAD.
  Support retains pub(super) c2_fixture at88, and core crate-visible helpers match
  the independently reviewed C2 snapshot. Parent SUPERVISOR_ENV and child forwarding
  remain1188/1382; actual C2-only/live-identity calls1264–1282 remain. No test/gate
  deletion, new suppression or widened production API.
- **Private S1:** corrected `supervise.py` SHA
  `1a2e04cf34c2a1fc8a5fb7ff6db30d2793dbd3c2e5cae6bca1064f6e290b0cc7`.
  Pre-S1 SHA9fe20bb9…e999b0 equals the previously inspected snapshot. All delta
  hunks/adjacent flow were read:226/229 reject supervision errors;274/284 also reject
  cleanup errors/timeouts while continuing owned cleanup;255 initializes failure-path
  summary variables. Original_rc, observer/product separation and identity/reap stay.
  `preparation-S1.sha256` SHA
  `ba35b8b9627562a009401dc483f9614077b35a00b0b94a3a8c494abe8615f10f`
  matches all26 entries. Parsed all15 AST-stage cases and read their actual checker
  statements: failed builder stages prevent the next stage; failed cleanup preserves
  original rc0 and later cleanup attempts; postflight preserves original0/7. Two
  retained real fake-waiter receipts preserve7/0 and observer error separation.
  These were not executed here. Private artifact is not candidate tracked source;
  genuine Docker/stream/builder compatibility and full smoke remain unverified.

## Completed evidence, diagrams and remaining gates

Completed workspace record: exact
`cargo test --workspace --locked --no-fail-fast --features kis-client/test-support`,
exit0 in687.237s, timed_out=false. Independently hashed frozen
`workspace-continuation-20261012-01/logs/workspace-exact.log`:
`d362cef5144864b26c847217417ed75470d7a45e20eed494a6afb902bcb975e6`.
Parsed235 terminal summaries:2886 reported passes,0 failures,6 ignored,180 filtered;
no FAILED summary. These are aggregate log counts, not distinct-test counts or a
strict-matrix/browser/hosted result. No mutable strict/Web logs were accepted.

Both PNG hashes match the supplied6cde6321…e9dc /29b7293c…551c.
Current component/runtime PUML SHA39ec1fd8…64fd /38aebaa8…2cc8.
F6/P3 citations109/111 resolve to current state source1524/1738/1758/1765;
component217 resolves to runtime owned anchor1840. Runtime structural text and
component structural text excluding comments/legend match HEAD; QA folding remains.
Local render exit0 is coordinator-recorded evidence, not a render performed here.
No stale checked citation found; unrelated unchanged citations were not re-audited.

Remaining: COV1 source follow-up; completed strict/default/all-feature results and
source-bound receipts; latest Web typecheck/browser inventory135 executions/134
synthetic identities with only owner-empty overlap and no skipped/retried/flaky
acceptance; full cold/functional smoke and cleanup; later candidate-commit checks,
hosted CI and image/manifest/deployment acceptance. Source review authorizes none
of those operations. Earlier actual API/runtime18/C2/DB acceptances retain their limits.

## Methods, ownership and explicit limits

Actual methods: git metadata/scoped diff/show; rg/sed source/report reads; stdlib
hashing, parsing, in-memory preimage reconstruction and AST parse only. Read complete
independent latest Web review and reused prior inspected history only at matching
hashes; no wholesale unchanged-source reread or reviewed Python import/execution.
No tests/compiler/build/runtime/resource/process/network/provider command executed.
Only tracked write: this NEW report, lines1–end; ignored private audit artifacts above.
Deviations: none in authorized scope. Missing review proof: COV1 as listed; missing
other required inputs: none. Unread source scope: remaining post-initial five-file
DB harness deltas, explicitly not accepted; other audited scoped hunks: none unread.
Not verified: remaining gates above, real S1 execution and final immutable candidate.
Further decomposition: one bounded five-file DB harness source review; no new design
or runtime rerun requested. **SOURCE_COVERAGE_AUDIT_COMPLETE; idle, no owned processes
or heavy slot.**


## COV1 independent source follow-up — 2026-10-12

**COV1 RESOLVED; all90 non-doc candidate paths are now independently source-covered
at the same frozen snapshot.** Blocking source findings: **none**. New actionable
behavior/security findings: **none**. No source repair or additional decomposition
is required. This closes the source-coverage gap, not final R1 acceptance. The
original90/85/5 assessment above remains historical evidence.

### Inspected scope and preimage method

Only the five assigned paths were newly reviewed. Read every current line and every
changed hunk: README1–92 (4 zero-context hunks), preflight-0039.sql1–158 (6),
preflight-catalog.sql1–383 (new file), static-check.sh1–124 (6), validate.sh1–1328
(52). For validate.sh, all added/context lines were covered by the complete current
source read and every removed line/hunk header was independently compared to HEAD.
No required owned hunk remains unread.

The bounded retained WP-3 package contains initial/final hash manifests, not an
established copy of the initial reviewed dirty files. No such preimage was inferred
from HEAD or invented. The permitted fallback was used: current full changed hunks,
complete surrounding control flow and the tracked HEAD diff. Earlier accepted
migration-contract/research-schema bodies were not reopened.

Minimal supporting reads: pending-target RLS at migration0014:42–66; audit owner
policies/function ACL at0039:30–61,70–101,133–144; catalog installer121–140,182–199;
migrate.sh177–232; seed-pre-0039.sql63–76; service-login.sql1–5; complete short
rollback-0039 guard/postflight files; QA Compose25–65 and bootstrap role/secret/psql
wiring54–123. These dependencies explain the actual role/transaction boundaries;
no migrations, grants, provider logic or service were changed or executed.

### Source-backed contract assessment

- **Lane identity and content:** catalog43–218 requires actual current_user AND
  session_user migration_owner, fixed envelope/release/five exact IDs, every Draft
  display/description/risk field, five exact version rows and five exact parameter
  schemas. Staged total is6 and permits only the named, correctly titled Paper row;
  full total is5 and explicitly denies Paper. Missing/extra/replaced content fails
  before fingerprint reporting. The original F2 inversion/count problem is closed.
- **Authority/RLS:** catalog221–286 checks all public table owners, exact seven-arg
  function identity/owner/safe search_path, allowed app/audit_writer EXECUTE and
  denied worker/admin/research_writer EXECUTE, enabled+forced outbox RLS and the
  one exact catalog audit payload. Direct login helper798–815 checks each of six
  roles as its own session, not supervisor SET ROLE. These checks do not purport
  to enumerate every PostgreSQL policy; unchanged production policy contracts and
  the separately accepted research-schema gate are not weakened here.
- **Terminal Paper separation:**0039:100–158 checks the terminal row only in its
  supervisor pass and quits before audit assertions. Its ordinary pass still
  reads/asserts the exact single audit event under migration_owner. Caller676–684,
  1146–1154/1191–1209/1232–1240 passes explicit terminal_only0/1 and verifies the
  supervisor count1. Migration0014's actor-scoped forced-RLS owner policy explains
  this split; audit read policy remains migration-owner-visible. No serving role,
  grant or policy is changed to make the preflight pass.
- **Negative transactions:** caller687–754 permits only the exact lane/probe map.
  SQL15–41 performs one mutation with ON_ERROR_STOP still on, then intentionally
  permits its validation error so291–313 can explicitly ROLLBACK. Caller requires
  exactly one ERROR, SQLSTATE23514 PLUS the intended reason, exact rollback marker,
  a successful normal postflight and unchanged fingerprint/audit ID/count. A
  permission, mutation, unrelated validation or rollback error cannot satisfy it.
  Positive postflights revalidate fields omitted from the compact fingerprint.
- **Conflict/down guard/reruns:** conflict377–383 uses the real immutable catalog
  installer, whose conflicting description raises23505; ON_ERROR_STOP terminates
  that transaction and disconnect rolls it back. Caller1171–1189 requires failure/
  23505 and a restored complete preflight/snapshot.0039/0041 reruns retain their
  exact ledger counts. Down-guard1212–1240 requires55000; the unchanged postflight
  proves ledger39 plus retained obligation and deletes only its named synthetic
  event. The subsequent migration-owner preflight preserves the real catalog row.
- **Full endpoint/no-op:** stage38 helper850–868 is disposable schema-only, fixed
  in-container URL, exact generated64-hex secret check and finite PGOPTIONS.
  Other staged runs and both full runs use the normal catalog entrypoint. Full
  override has no staged migration mount;952–999 uses all current up migrations,
  validates contiguous ordered ledger count/latest/checksum entries and compares
  both checksum lists and catalog/audit snapshots before recording NO_OP. The full
  cluster is independently credentialed and receives no staged application seed.
- **Ownership/cleanup:** private mktemp workdir288–293 must match the existing/tmp
  generated-prefix guard; umask077, workdirs0700 and secrets0400 remain. Stage/full
  credentials are generated; test compatibility secrets remain isolated fixtures.
  Project IDs/ports/overrides are per lane, with loopback ports distinct. Flags are
  set before partial startup. Cleanup450–483 attempts only those started Compose
  projects, logs attempts, removes only its owned workdir and preserves original
  exit status. Existing failure-trap cleanup is best-effort, not proof of absence;
  the previously accepted independent runtime cleanup evidence remains necessary.
  Added full teardown1281–1296 fails on down/inspection/nonempty owned-container
  results before clearing its flag or starting tests. O1 remains resolved.
- **Evidence/static gates:** raw files stay in the private workdir;348–364 sanitizes
  URLs/DATABASE_URL/PGPASSWORD assignments before0600 retained logs. New credential
  handling uses files/in-container environment, not values on external argv or
  echo. Summary accumulator starts empty311, appends completed probes753 and emits
  NOT_RUN only when empty372, preserving partial progress. JSON/TSV distinguish
  three lanes, checksums/reruns and typed outcomes; baseline HEAD label does not
  turn dirty source into an immutable commit. Static markers remain wiring checks,
  not execution proof; the existing explicit migration-safety audit is retained.
  All ten Cargo selections still receive DATABASE_URL and reject raw SKIP markers.

### Frozen hashes, receipts and remaining limits

All five before/after hashes match the historical audit and accepted execution
inputs. Full90-path set and all90 file hashes were checked unchanged at start,
end and immediately before this append; worker documentation appends are excluded.

| Assigned file, relative to deploy/db/integration-validation/ | SHA256 before=after |
| --- | --- |
| README.md |`b98ffaed27198de4882f6bfd13579f25f622cd9cca775805074af06c41a89405` |
| preflight-0039.sql |`eaa621285ed1f00dd42b74bceb4178763cd47c79a19ec164459392f88bb86d33` |
| preflight-catalog.sql |`f7eebd39dfcbd3544e472021450067b58d54f3ae1574bf9f1faf18c00e059a3f` |
| static-check.sh |`adb07a2152026df6210d1ea7afdd989aa9a0dca65d00f7ea2ea69f36f77d30a1` |
| validate.sh |`3300ad15cf7d1ed7398e35389493c6109655a5f9c31393710f09cf50117f5ac3` |

Under the existing private audit root, NEW `cov1-source-review.json` SHA
`26e24d2a1555ed9b02f751aa985b831ba1e327368a5b2e3f0c4d5fcde0cfab91`
records five closures/full90 equality. NEW `cov1-current-tracked.diff` SHA
`f240e9300abe2482ac3479feca5fe6a6ecb98d13b28bfa85e38c0709ad9bea72`.
The earlier coverage.json/after.json and90/85/5 classification are unchanged;
this receipt closes their five FOLLOWUP_REQUIRED entries at the same source hashes.

Actual checks: git metadata/scoped diff; numbered source reads, rg references and
stdlib hash/JSON/path-set comparison only. No reviewed script/import, syntax test,
self-test, compiler/build, DB/Docker/browser/network/provider/resource probe ran.
Owned report change: top status plus this append only; source/other reports untouched.
Deviation: permitted full-current-hunk fallback because initial dirty preimages were
not established. Missing other inputs: **none**. Unread required hunks: **none**.
Not found: new blocking/actionable defect, suppression or unrelated source edit:
**none**. Not verified here: a fresh DB run, completed strict receipts (separate
coordinator evidence), latest Web outcomes, ongoing full smoke/cleanup and hosted/
image/immutable-candidate/final R1 acceptance. Further decomposition: **none**.
**COV1_SOURCE_REVIEW_COMPLETE; idle, no owned processes or heavy slot.**
