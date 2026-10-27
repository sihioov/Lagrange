# R4-06 G0 integration interface and early verification

Coordinator adoption, 2026-10-27; R4-06 remains DOING.
This closes the finite SOURCE dispatch interface for WP-5C3E and WP-5C4 under the
[consolidated plan](2026-10-27-r4-remaining-integration.md).
It authorizes SOURCE implementation, not execution, installation, new provider access or Owner acceptance.
Existing C0/D0/D2A-K evidence is reused; original reports and controls remain immutable.

## Shared executable and launcher ABI

| Boundary | Adopted contract | Evidence / owner |
|---|---|---|
| Official launch | NEW `--official-daemon --expected-commit COMMIT --expected-config-sha256 sha256:HEX64 --expected-receipt-sha256 HEX64`; exactly these three options, no aliases/duplicates/extra arguments. Same pin grammars as existing health. No artifact-root, receipt-path, worker-id or alternate config overrides on this mode. | Binary current closed Health parser; E adds Official variant. |
| Independent pins | Installer validates raw config, manifest and official receipt through existing release library. Launcher receives `LAGRANGE_CODE_COMMIT`, `OWNER_BETA_SIMULATION_CONFIG_SHA256` (prefixed), `OWNER_BETA_SIMULATION_RECEIPT_SHA256` (bare) from that protected binding, not from hashing the file being authorized. COMPLETE corroborates receipt; it cannot supply the independent expected pin. | C1 interface; C4 transports; E consumes. |
| Health | Existing `--health` with exactly the same three pin options. Health performs only checked config and process health reads; never DB, writer, startup, claim or restart. Exit0 healthy, invalid arguments2, unsupported7, rejected8 remain. | Binary health implementation; C4 Docker healthcheck uses this argv. |
| Official exits | New Official mode: normal fully confirmed drain0; parser2; unsupported7; startup/parent/cleanup/completion rejection9 with fixed value-free diagnostic. Signal alone is not proof of clean exit. Legacy Once/Daemon/Child exit behavior unchanged. | E implements; C4 requires clean process exit plus independent settlement. |
| Fixed paths / identity | Retain checked config's exact container paths and worker-id; host historical_root is only the RO bind source. User10001:10001; health records directory existing required tmpfs/0700 policy. No arbitrary writable input or test-root production switch. | process_config.rs, process_read/write.rs; C1/C2. |
| DB credentials | Official production uses existing component DB options with db_worker and password file. No direct URL, plaintext password, administrator/migration role or new grant. C4 mounts service-specific worker secret; never copies secret values into args/config/report. | Binary database_options, existing secret provisioner. |

C4 keeps base service disabled/build-only; only the explicit simulation overlay enables
owner_only. It must inspect actual image, config, receipt labels, singleton/generation,
mounts, credentials, network, health argv, restart policy and configured resource limits.
Config values remain externally supplied; no synthetic values become operational defaults.

## Startup obligations (E)

One private one-use startup value binds checked config + process generation + retained
official runtime + approved V3 object + the successfully completed observation sequence.
No public boolean/deserialize constructor, and no global reusable rights cache.

| Obligation | Exact consumption / failure behavior |
|---|---|
| Official provenance | Secure config read against independent commit/config pins; exact bare receipt comparison; TrustedSimulationRuntime::load_official_config; retained revalidation. Before DB effects and claims. |
| Reader | Call existing market_data::approve_historical_price_only_artifact at config.artifact_root; require V3 summary. Keep actual sealed object and pass it by reference to configured rights observer. Preserve per-child reapproval. No actual approved input is read during SOURCE work. |
| Reader identity | New official path observes effective UID and GID and requires checked10001/10001. This is source wiring, not observed host proof. Existing reader handles exact bytes/ownership/modes; external launcher handles host bind/RO policy. |
| Database observations | Invoke existing migration-ledger, worker-column-ACL, routine-metadata, RLS-metadata, trigger-metadata and configured-input-rights observers with same checked config and cancellation flag. Each succeeds only on its own completed and closed disposable connection. Keep their narrow documented scope. |
| Identity consistency | Compare config hash, database OID and worker OID across observations; compare schema and migration-owner identities where exposed. Add only read-only pub(super) getters to trigger/rights opaque observation types where needed. No external token construction. Same OIDs do not prove one snapshot or future freshness. |
| Actual worker connections | Official pool owns setup on every created connection and validates on every checkout: direct db_worker/session identity, no privileged role substitution, expected database identity, search_path pg_catalog,public; positive numeric statement/lock timeouts bounded by min(checked startup seconds *1000,30000). Observer transactions keep their own local pg_catalog/RO/RR settings. Existing queue begin timeouts15s/5s remain and are not mislabeled as RO. Reject contaminated/replaced connections; do not assume a disposable observer configures other pool sessions. |
| Recovery | Await existing exact-type recovery after observations and before admission. It can affect multiple owners as the existing dedicated-type maintenance contract already does; it is not a claim or rights grant. Preserve its individual commits and failure/unknown completion. Do not silently discard recovery errors. |
| Deadline / generation | One startup deadline from checked config plus actual monotonic generation observations. Sample only updates observation age. Cancellation/expiry closes admission immediately, but retain and await owned blocking reader/DB work; a timed-out wait is not canceled I/O. Late results cannot mint evidence. |
| Scope of readiness | These finite observations and recovery completed for this generation, not full catalog/body/RLS semantics proof, external legal rights, immutable mount proof, cross-session atomicity, freshness lease, or global DB settlement. Those retain existing G2/G3 gates. |

Critical missing API or contradictory requirement blocks that affected path and must be
reported with exact symbol/path; unrelated integration continues. No new generic proof project.

### Common command-result limitation — bounded disposition

SOURCE inspection found the same execute/discard command-result pattern in ACL, migrations,
routine metadata, RLS metadata and trigger metadata adapters. D2K correction1 already uses
fetch_optional with a value-free None/Some classifier. E may repair ONLY the Begin/Commit/
Rollback command response handling in those five modules and their private tests, using
that established bounded pattern: None=0 rows, Some=one sentinel rejection, full response/
late errors remain the installed SQLx contract, no row payload materialization. Preserve
each existing primary/cancellation/cleanup classification and rollback/close uncertainty.
Do not alter catalog SQL, expected policies, raw decoders, limits or accepted financial data.
This is one integration correction for a shared cause, not five new review packages.
Prior SOURCE acceptances and original evidence remain historical and are not overwritten.
E also owns additive getters in configured-input-rights.rs and its regression source.
Actual SQLx command behavior and all Rust compilation remain G2, NOT RUN.

## Queue, progress and shutdown (E)

- Adopt a NEW official-only owner-filtered exact-type claim entry point in queue.rs.
  Bind config.owner as UUID in the real locked SELECT and retain the owner predicate through
  update/returned claim checks. Legacy claim_next/claim_next_for behavior stays unchanged.
  Never claim a different owner's job then invent a financial failure to settle it.
  Recovery remains the existing exact-type recovery scope above.
- Add an exact-attempt heartbeat API checking job ID/type/owner, RUNNING, locked_by,
  attempt_count, matching attempt ID/number/worker and active attempt state, plus current
  lease; update and acknowledgement in the same transaction. Existing heartbeat API is
  unchanged. No progress on failed/unconfirmed commit or stale attempt.
- Preserve existing run_simulation_once as legacy entry; use an internal observed variant
  for official execution. Distinguish closed/shutdown Idle, slot-refused Idle, and a real
  no-candidate SELECT followed by acknowledged rollback. Only the last supports a narrow
  completed no-claim cycle. No extra empty transaction solely to obtain a commit.
- Root future owns the !Send SimulationHealthWriter directly. A private bounded channel
  (capacity1, one outstanding event per producer) carries generation/attempt/epoch-tagged
  progress plus acknowledgement. Producer advances only after root apply succeeds.
  No unsafe Send/Sync or Arc/Mutex fiction, no waiting for a producer while root stops
  servicing its event receiver. Full/closed/lost acknowledgement is a typed failure.
- StartupVerified follows consumed startup evidence; Claim/Lease progress follows actual
  corresponding commits. CycleVerified follows confirmed settlement/reconciliation and
  heartbeat abort+await. Timers, pool connection, generic Idle and financial outcomes alone
  cannot mint progress.
- Root handles signal/fatal control ahead of progress; closes admission and invalidates epoch
  before awaits. Stale events receive rejection without writer apply. First fatal is sticky.
  Writer poisoning forbids reopen/repair/unlock/auto-resume; retain ownership through cleanup.
- Preserve CleanupUnconfirmed before generic interruption/error mapping, unresolved claim/
  publication/failure/recovery commit, task join failure and same-await cancellation+error.
  Await/reconcile already started commit using existing supported paths; if conclusive
  resolution is unavailable, stop admission and report uncertainty. Never infer rollback
  from future drop. Keep financial bytes and publication entitlement checks unchanged.
- Sample cadence is derived from config health_max_age (one third in checked milliseconds),
  never progress. Existing 2s idle poll and queue lease/heartbeat defaults retain their
  meanings. Stop deadline uses config stop_timeout; expiry records failure, not clean
  teardown or permission to abandon owned children/commits. Runtime timing still unverified.

## Operations and backup (C4)

Reuse C1 protected binding/floor and C2 exact generation capture/drain. Hold existing
release/maintenance locks in established order. Fence API admission BEFORE worker drain,
independently check DB settlement and preserve queued jobs/history. Process absence or
health alone is not a fence or DB settlement. Failure leaves OFF, no automatic resume,
no forced KILL/remove/substitute-container mutation. Keep actual stop deadlines.

Extend current installer/wrapper/backup formats and existing Daily integration rather than
introducing another framework. Simulation-only installations must route to the aware backup
path even when Daily is OFF. Preserve independent maintenance-tool commit versus application
commit/config/receipt identity. Source/target reader capability and monotonic63/64/v1+v2
floor must be checked even when simulation is OFF. Unsupported target or incomplete
simulation history/config/floor rejects; no fallback into legacy backup.

Backup/restore include the exact historical input tree and required approval/config/receipt/
reader-floor/release identity alongside existing DB/Raw/Curated/Daily classes, without
secrets or general filesystem capture. Restore validation covers existing63/64 schema,
v1/v2 rows/pairs/jobs/attempts/Owner/entitlements and pinned modes/hashes. Retain all history.
No automatic services after maintenance. Actual backup, restore, drain, install and runtime
inspection are NOT RUN here. Existing privileged wrappers' root/system operations may be
authored but must not be executed in SOURCE scope.

### API handler completion — coordinator disposition after continuation1 recovery

Both E and C4 were idle/pending0 and their continuation1 outputs were recovered before
this document update. C4 remains incomplete; E source review is pending. Coordinator
changed only `crates/api-server/src/runtime.rs` and `crates/api-server/Dockerfile` to
return failure on HTTP drain timeout and declare `org.lagrange.api.http-drain-contract=v1`
on the immutable image. Existing audit cleanup is retained; this contract does not prove
audit or DB settlement. Timeout and in-flight success regressions are SOURCE, NOT RUN.

C4 must verify this capability on the exact manifest-bound API image before mutation,
including actual executable/argv identity and absence of executable-shadowing overrides.
A container label, old image exit0, process absence, health, or created-only container does
not establish handler completion. Capture exact generation, suppress restart, TERM that ID,
await successful exit and recheck generation/exit/OOM/error under the bounded deadline.
Keep source/target API identities separate during transition and independently establish DB
settlement after the API fence and simulation drain. Unknown or unsupported states reject.
Do not reject unrelated Daily-only apply/rollback unconditionally or let unknown simulation
history fall through to legacy backup. Complete the original C4 integration in its same scope.

The exact additive preservation exception and source diff are in
`target/r4-expansion-implementation/wave24-supervision/api-drain-disposition/`.
Original worker controls and all before copies remain immutable. Actual compile, image build,
handler drain and DB/backup/restore verification still require the separate G2/G3 scope.

### Continuation2 recovery and bounded correction handoff

Both E and C4 are actually idle/pending0 and their continuation2 outputs have been recovered.
E F1–F3 are SOURCE accepted; compile/runtime remain unverified. C4 SOURCE acceptance is
withheld for start ownership/cleanup uncertainty, source/target generation handoff across a
release switch, and incomplete regression snapshots. The combined review is in
`target/r4-expansion-implementation/wave24-supervision/c4-continuation-2-recovery/source-review.md`;
the exact brief and freeze are in sibling `c4-continuation-3-control/`. Same C4 owner, five
source paths; E stays idle. Original reports and controls remain immutable.

Before any start may occur, retain exact ownership to reconcile a lost response and perform
bounded TERM-only cleanup. Preserve primary and cleanup uncertainty independently; unknown
state does not prove OFF and blocks another start. Carry independently verified source/target
bindings and stopped generations across the atomic switch under the existing release lock.
A bounded protected handoff record may retain facts, but never substitutes for fresh identity,
process, reader or DB checks. Explicit operator start may perform the existing ordinary Compose
replacement/start of the proven stopped owned service for the verified target; no force-recreate,
force-remove, rm/down/KILL, automatic resume or foreign-container mutation is allowed. Do not
check the old stopped container as if it already had the target identity. Incomplete or
interrupted handoff remains pending/rejected, never legacy/empty.

The coordinator concurrently owns only the four previously assigned WP6B harness files for
SOURCE registration; explicit baseline exceptions are in continuation3 freeze. Other shared
documents stay frozen until the next idle handoff. No execution authority is added.

## WP-6B early verification sheet — commands prepared, NOT EXECUTED

Current HEAD is d100c81f31dcd4495271e4318a82d28f6cb74aff with uncommitted SOURCE changes.
HEAD alone does not identify the candidate. Use the frozen input manifest for SOURCE review;
before hosted dispatch create/review a separately authorized committed candidate SHA.
No placeholder SHA, fabricated workflow result or self-derived external pin is acceptable.

| Order / environment | Exact target or command | Required authority / prerequisite |
|---|---|---|
| 1 earliest compile, existing hosted Rust job | `cargo +1.97.1 clippy -p job-queue --all-targets --all-features --locked -- -D warnings` | Hosted/compile permission and candidate commit; job's installed pinned toolchain/dependencies. No DB needed for this compile gate. Existing all-workspace clippy still required finally. |
| 2 pure Rust seam regressions, same hosted job | `cargo +1.97.1 test -p job-queue --lib owner_beta::simulation_execution:: --locked -- --test-threads=1` | Explicit test authority; existing child tests also perform process/kernel operations. Use hosted harness preparation, not local SOURCE permission. Verify nonzero discovered test inventory and exact new names. |
| 3 CLI/process | `cargo +1.97.1 test -p job-queue --bin owner-beta-simulation-runner --locked -- --test-threads=1`; `cargo +1.97.1 test -p job-queue --test owner_beta_simulation_process --locked -- --test-threads=1` | Same candidate/build provenance setup from scripts/ci/build-r4-verification.sh; process/syscall execution authority. No new approved input access. |
| 4 actual DB | Existing deploy/db/integration-validation/validate.sh registrations: job-queue owner_beta_simulation_execution, owner_beta_simulation_v2_migration, owner_beta_simulation_v2_execution; api-server http_owner_beta_simulations, http_owner_beta_simulations_v2 plus durable replay lib lane. | Disposable hosted PostgreSQL and existing prepared roles/migrations; DB/integration authorization. Use existing run_db_test wrapper and strict complete test-name evidence. Include actual observer SQL/default/decoder/command behavior; not just migration-text hashes. |
| 5 ops mocked suites | Existing focused release/runtime tests plus NEW test_owner_beta_simulation_operations.py and test_owner_beta_simulation_backup.py under python3 -B; existing test_ci_contract.py with real PyYAML. | Offline product/test execution authorization. Before running, inspect every effect boundary; no full fakeDocker attempt remains and no yaml import stub counts as full-suite pass. |
| 6 same-SHA final CI | Existing .github/workflows/ci.yml required aggregate and .github/workflows/research-smoke.yml; full workspace Clippy/tests, API/Web/OpenAPI/browser, research/candidate builds, PG lanes and provenance. | Hosted/network/build permissions; existing CI prepared environment and bounded run budget. Reuse only eligible exact deterministic receipts; dynamic tests and final CI actually run. |
| 7 operational acceptance | Concrete commit-pinned install/drain/rollback/backup/restore/approved-input/Owner runbook after G1/G2 evidence. | Separately authorized exact inputs, host, counts, resource/cleanup plan and availability effects; no production allowance inferred here. |

No new workflow or remote dispatch is authorized by this sheet. WP-6B integration writes
remain the four existing harness files named by the consolidated plan. Register E/C4
test names together after integration. Earliest compile failure is repaired by the existing
implementation owner, without another analysis-only worker.

SOURCE now permits bounded text/static/hash/mode/scoped Git and exact installed rustfmt
only on owned changed Rust. No tests/compile/SQL/Docker/host/network/actual-input actions.
G0 SOURCE interface is adopted; G2/G3 authority and evidence remain OPEN.



### Continuation3 common-idle SOURCE handoff

E remains accepted and idle. C4 continuation3 was recovered at actual idle,
activeTurn null and pending0. The original report, preliminary/final manifests,
all command evidence and controls remain immutable. Review closed F1–F3 at
SOURCE level after a coordinator two-file repair: during replacement creation,
the old generation stays in the release handoff while the start journal has no
target generation until capture. Explicit reconciliation tolerates the adjacent
api-started/worker-replacing journal boundary only with fresh source/target proof.
The regression source covers both upgrade and rollback with creation response
loss before/after replacement, nonzero creation and handoff-write interruption.
At SOURCE handoff, all tests were NOT RUN; uncertain API completion stays pending.

WP-6B has 292 private process tests in source. The Linux R4 execution inventory
registers 289 of them; three `#[cfg(not(target_os = "linux"))]` cases are not
compiled there. It also registers the expanded runner/CLI inventories, 16 DB
execution names and the two operations/backup Python suites.
The evidence template and static marker now both require 16 DB rows. Existing
hosted jobs and required aggregation are unchanged. No registration is an actual
SQLx/PG result; real observer projections/defaults/raw decoding/command-response
coverage must be established in the execution gate. Runbook and diagram sources
now describe the optional OFF overlay and explicit maintenance/handoff path.
PNGs were not rendered at SOURCE handoff under the Docker prohibition.

The final source manifest and limitations are in
`target/r4-expansion-implementation/wave24-supervision/final-source-handoff/`.
Current uncommitted HEAD is still not a runnable/releasable candidate identity.
A reviewed committed candidate and separate execution authority are required
before the first compile/test/hosted dispatch. No additional analysis-only worker
or retry of completed source audits is required by this handoff.
