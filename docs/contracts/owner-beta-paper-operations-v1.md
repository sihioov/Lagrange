# Owner-beta Paper operations v1 (R5 WP5 P0)

Date: 2026-11-06. Status: **coordinator-adopted source protocol; implementation pending**.
Baseline: `972fe37d9909626cb6dd0f665cbaf2723f3742d3`. Roadmap R5-04 / WP5;
G3, G4, G5 and R5-05 remain open. No deployment, database execution, source-origin
acceptance, release timing or real trading-day observation is established by this document.

This is a technical extension of [ADR-0010](../decisions/0010-owner-beta-paper-v1.md),
[Paper v1](owner-beta-paper-v1.md), [SQL v1](owner-beta-paper-sql-v1.md) and
[Current v1](owner-beta-paper-current-binding-v1.md). Those immutable contracts and
financial values remain unchanged. Automatic operation means the existing single
consented account; OFF/restore never resumes economic work automatically. No provider,
account, order, data-rights or notification-delivery permission is added.

## 1. Source seams and choices

The read-only audit `target/r5-completion-20261106/ops-audit/report.md` was read in
full; the following decisive seams were independently inspected. Line references
are to the baseline, not proof of installed behavior.

| Source | Consequence |
| --- | --- |
| `migrations/0067_owner_beta_paper_ledger_v1.up.sql:338`, `:379`, `:397` | Keep transition ABI, request replay and DRAIN fence; OFF/RESTORED_OFF lack a consumer. |
| Same file `:439`, `:883`, `:963`, `:1061` | Serving snapshot requires current authority; maintenance needs its own reader. Exact committed receipt replay must remain before economic authorization. |
| Same file `:61`, `:179`, `:383` | Lineage is immutable; old target recovery cannot rewrite its epoch or replace a settled receipt. |
| `crates/job-queue/src/owner_beta/paper/evidence.rs:800`, `:1245`, `:4640` | Extract checked source/DB identities at their join, retain checked resources until privileged consumption; report JSON lacks this authority. |
| `crates/job-queue/src/owner_beta/paper/runtime.rs:169` | Actual compiled revision and ELF, fixed protected process config; env/image labels alone fail. |
| `crates/api-server/src/paper_runner.rs:34`, `:77`, `:449`; `src/bin/paper-runner.rs:447` | Outer15s/cycle90s, heartbeat10s/lease60s, app4/worker4/admin2/audit2; heartbeat is propagated, unlike the stale handoff note. |
| `scripts/ops/lib/owner-beta-simulation-operations.py:1630` | Caller-held release lock and independent stopped-generation checks are reusable, not Paper proof. |
| `scripts/ops/lib/owner-beta-daily-backup.py:1677`; `owner-beta-daily-backup-files.py:24` | Combined backup exists; Paper archive/audit and no-auto-resume extension are missing. |
| `migrations/0041_paper_settlement_outbox.up.sql:223`, `:264` | Preserve active obligation, archive and notification/delivery identities, not just ledger totals. |

All new numeric limits, paths and function names below are **chosen technical v1
values**, not previously observed runtime facts. Their rationale is bounded consumption
and reuse of existing protected readers/locks. Largest observed migration is 0067;
reserve **0068_owner_beta_paper_operations_v1.{up,down}.sql** for P1. Recheck the
reservation before writing; a collision is a coordinator integration conflict, not
permission to edit 0067. Its bytes and destructive-down guard remain immutable.

## 2. Wire, identities, producers and trust

[Receipt schema](owner-beta-paper-operations-v1.schema.json) closes operation intent,
stage record, writer inventory, reconciliation summary, drain/restore receipts and
recovery decisions. [Archive schema](owner-beta-paper-operations-archive-v1.schema.json)
closes the archive root, audit and release capability. Neither parsing nor a matching
hash creates authority. Every object has an exact key set; null is explicit.

Canonical bytes: ASCII subset of UTF-8 JSON, keys in ASCII lexicographic order,
no whitespace/BOM/final LF, duplicate keys, floats, exponent numbers or negative zero.
Integers are actual integers, never booleans; counters fit nonnegative signed i64.
All strings in the new wires are ASCII patterns/enums. UUIDs are lowercase canonical,
nonnil; digests are `sha256:` plus 64 lowercase hex. Hash the exact canonical bytes
(no self-hash field). UTC timestamps use whole seconds `YYYY-MM-DDTHH:MM:SSZ`;
clock triples use integer nanoseconds. Validate civil dates separately from regexes.
Arrays are sorted by their declared identity, with no duplicates. Reject noncanonical
input rather than silently normalizing it. Legacy receipt hashes/serializers stay intact.

Maximum each intent/stage/inventory/reconciliation/drain/restore/recovery/capability:
**65,536 bytes, depth16**. Archive root/audit: **1,048,576 bytes, depth16**.
Inventory maximum64 entries; archive fixed class list, audit fixed group list. Exceeding
a limit fails closed; it never truncates history. Full target receipts retain256KiB,
existing eligibility retains1MiB, existing contract/source retain64KiB. Large row/file
sets stream into private hashed manifests using existing backup byte/file limits;
these envelopes carry roots, not unbounded embedded evidence.

Identity is `(installation_id, generation, request_key, purpose, scope, account_id,
expected_epoch, release_manifest_sha256)`. `installation_id` is a root-created UUID
persisted outside releases/backups as local identity; `generation` is one fresh UUID
per operation (restore always creates a destination-local generation). Request keys
are 1–128 ASCII `[A-Za-z0-9._:-]`, immutable within account. Stage SQL keys are
`<generation>:begin`, `:reconcile:<sequence>`, `:off` or `:restore`; caller key binds
the intent. Reuse of either key or generation with changed canonical bytes conflicts.
Equal bytes return original identities; replay is descriptive, never renewed authority.

| Object | Producer | Consumer / independent checks |
| --- | --- | --- |
| intent, immutable stages, writer inventory | P4 installed root controller | P2 native bridge reopens protected originals, verifies schema/hash/manifest and SQL state. Controller observes kernel/Docker; native does not infer their truth from JSON. |
| reconciliation summary | P1 narrow SQL, encoded canonically by P2 | P4 compares expected target/receipt identities and captured backend generations; SQL independently locks/re-reads DB state. |
| drain receipt | P4 after termination and reconciliation | P2 checks protected chain/current clocks; SQL compares registered generation, latest stored reconciliation and epoch before OFF. |
| restore receipt | P5 verifier, finalized by P4 in destination | P2 checks destination-local operation and archive/audit; SQL compares restored logical roots and prior receipts before RESTORED_OFF. |
| eligibility | P2 private checked value | P2 consumes in ENABLE transaction; public report remains non-authoritative. |

Root, kernel, Docker and the installed controller are trusted. SQL cannot authenticate
arbitrary root JSON. Only the installed root controller can mount its protected receipt
and migration-owner credential into the verified native helper. Native independently
checks file metadata, canonical bytes, chain, actual selected binaries and protected
source inputs; SQL independently checks DB identities/locks/replay. A compromised root
or migration_owner is outside this threat model. No signature or `verified=true` claim
pretends to remove that trust. App/worker/admitter cannot supply lifecycle receipts.

Fresh drain/restore consumption: **age strictly less than30s** on realtime, monotonic
and boottime; no future age; same local boot; elapsed delta divergence <=1s. Chosen30s
is a final observation-to-commit bound (two15s DB windows), not a shutdown deadline or
eligibility TTL. Long drains/backups may exceed30s: obtain a new reconciliation and
append a new receipt immediately before consumption, with unchanged frozen history.
Never edit the old receipt. SQL rechecks observed realtime against `clock_timestamp()`;
native enforces boot/three clocks before and immediately after COMMIT. Clock failure
post-COMMIT is UNKNOWN requiring durable readback, not rollback fiction. Eligibility
still expires at min(evaluated+180s, next verified session cutoff); source Current still
uses its independent180s/1s policy. Reboot/restart invalidates live evidence; durable
replay can be read regardless of age but cannot authorize another transition.

## 3. Persistence, ordering and two inventories

Fixed host paths below are rooted in the already validated `install_root`:

- `.owner-beta-paper/` root:root0700; `installation-id` and `epoch-floor` root:root0600.
- `.owner-beta-paper/operations/<generation>/` root:root0700; `intent.json`,
  `00000000.json` etc and content-addressed `<digest-hex>.json` root:root0400.
  Stage sequence starts0; previous digest is null only at0, otherwise exact predecessor.
- Existing `.lagrange-release.lock` root:root0600 is never replaced/unlinked.
  All helpers receive its already-held FD in the host process; they never reacquire it.
- `epoch-floor` is an atomically replaced, fsynced integer equal to at least the highest
  locally committed control epoch; durable stage intent records the proposed epoch first.
  Advance floor before permitting runtime start. Keep a higher floor on failure.

Create files O_EXCL/O_NOFOLLOW/CLOEXEC; verify every ancestor, no ACL, regular nlink1;
fsync file, no-replace publish, fsync directory. Never mutate a sealed stage. A mutable
latest pointer is advisory only; reconstruct the unique contiguous chain. A torn temp
or missing stage remains incomplete; forked chains, changed bytes and gaps block.
No directory guessing or mtime-based success. Stages are INTENT → FENCED → STOPPED →
RECONCILED → OFF_COMMITTED, optionally SNAPSHOT → RESTORE_VERIFIED → RESTORED_COMMITTED.
`FAILED` records the fixed error and prior chain without discarding intent/evidence.

**Lock order:** take release lock; commit Paper DB fence; signal/disable dispatch
without waiting on collector/scheduler locks; wait for publisher and children to end;
then acquire collector lock, then scheduler lock, then existing Raw/Daily publication
locks in their established order. Never wait for a running scheduler while holding the
collector lock it needs for child cleanup. Never hold a Paper SQL transaction while
waiting for an OS lock, process, child or backup. Use bounded lock attempts, release
only owned locks; no force-unlock or inode replacement. Preserve Daily SQL source-lock
order inside existing economic functions; lifecycle SQL takes Paper control/account
locks and commits before Daily disable. Release lock excludes sanctioned replacement
and start throughout the handle lifetime. Stop requests are not proof of stopped state.

| Scope | Must fence, inventory and terminate | May remain alive |
| --- | --- | --- |
| PAPER_STOP | Actual API generation (including consent/manual mutations), paper-scheduler runner, every process/child that can write beta preview/target/settlement/valuation or its outbox/notification/archive; their PostgreSQL backends and claims. | Shared B witness, KIS/Daily/admitter/recommendation, simulation and unrelated writers **only if** root inventory proves they cannot write the stopped beta/control/notification subset. Current can remain live for new pre-observation; DB DRAIN prevents Paper authority. |
| COMBINED_BACKUP | Above plus B publisher and its wrappers/capture containers, legacy KIS dispatch, Daily/research/admitter/recommendation writers, simulation worker, candidate/raw/curated publishers, generic Paper, notification delivery/archiver and every other installed process/job/timer capable of mutating the full DB or archived roots. | Postgres server and read-only backup/verifier; unrelated read-only frontends only with proved read-only access. |

Every inventory must cover API, PAPER_RUNNER and PAPER_DELIVERY explicitly, including
proved absence. Combined scope covers every additional class named in the schema; an
absent optional producer still gets an ABSENT_AT_FENCE observation. A process occupying
multiple classes uses the same generation identity in each; terminate it only once.
Class coverage is a semantic check in addition to schema closure.

This is an authority inventory, not a hardcoded assumption about service names. Enumerate
actual manifest services, systemd units, invocation children and writable mounts plus
DB sessions. Inventory rows bind kind/role, exact generation digest, process start,
boot, cgroup/container identities, image/ELF/manifest and backend-set digest in protected
observation originals. Unlisted/ambiguous writer fails; operator can choose the coherent
combined path, never silently certify the narrower path. For PAPER_STOP shared-role
connections cannot be excluded by caller-selected `application_name`; map actual socket
endpoint and backend PID+backend_start to the captured process, or fail closed.

For a live publisher affected by combined backup/release/config change: request existing
B shutdown, publish INVALID and close OFD before changing assets, then terminate/reap
children under its own lock protocol. A failed INVALID write still closes authority.
Observe retained Docker IDs twice, exit/FinishedAt/restart=no and empty owned cgroups,
reap owned OS children/pidfd-exit with start identity. Retain exited container objects
until receipt sealed. Replacement, restart drift, missing original or unobservable
termination is UNKNOWN. A pre-fence absent generation is allowed only with launch
sources disabled, complete enumeration and repeated absence under release lock.

A finite controller stop attempt is90s (chosen reuse of cycle bound), independent of
existing runner15s operation/90s cycle/90s inner action/20s grace/30s Docker stop. Timeout
leaves DRAINING, records unresolved identities and requires another observation attempt.
No lease expiry, signal return, health JSON, shell exit or `docker stop` result proves
absence of DB commit. No unlimited retry or hidden timing increase.

After actual process exit, reconcile actual PostgreSQL backend identities: cancel and,
if necessary, terminate only captured writer backends through the existing privileged
maintenance connection; wait for their disappearance, inspect prepared transactions
and reject unexplained prepared work. Reconnection is forbidden by stopped launch sources.
Detailed backend observation/termination is root-controller work through the existing
local postgres maintenance transport (fixed SQL, captured PID+backend_start only). Do not
grant pg_monitor/pg_signal_backend to serving or migration_owner roles. Native uses
migration_owner to check captured PIDs are absent and prepared-work count is zero;
masked statistics or inability to correlate identities fails closed. It cannot obtain
that proof from an application_name filter.
Take the same Paper control/account locks as economic writes; re-read all target/session/
job-attempt/receipt/outbox state after backend termination. A late COMMIT may have won:
compare exact persisted settlement/valuation and order/fill/outbox identities. Matching
is COMMITTED, absent under the final fence is NOT_COMMITTED; changed is CONFLICT. An
unresolved DB/transport/backend observation never becomes NOT_COMMITTED by timeout.

Reconcile abandons only dead beta preview attempts using existing canceled/failed job
semantics and terminal projection; it never requeues an old-epoch preview. Preserve
READY/APPLIED rows, original results and attempts. Unsettled targets become HELD with
RECOVERY_REQUIRED event; settled-but-unvalued receipts/state/ledger remain unchanged.
Ready/exhausted outbox stays pending; inflight returns to retryable only after the exact
claim owner is dead and original attempt/delivery identity is retained. Ambiguous external
delivery remains quarantined; never mark delivered or resend by guess.

Restart: reacquire release lock, validate original manifest/chain and DB generation,
inspect actual resources anew; reread unknown SQL commit before retry. Pre-COMMIT stages
can append observations; completed exact operations only replay. A stale boot requires
new generation linked by `predecessor_generation`, re-fence from actual epoch, never
reuse old freshness. DB fence is authoritative when disk completion is missing. A floor
or stage ahead of DB never enables anything. Failed state is preserved; no automatic ON.

## 4. Forward SQL ABI and retained lineage

P1 adds only these public maintenance functions; no new role or credential:

```sql
public.begin_owner_beta_paper_operation_v1(expected_epoch bigint, intent jsonb, request_key text) RETURNS jsonb
public.inspect_owner_beta_paper_operation_v1(generation uuid, after_target uuid, page_limit integer) RETURNS jsonb
public.reconcile_owner_beta_paper_operation_v1(generation uuid, expected_epoch bigint, inventory_sha256 text, request_key text) RETURNS jsonb
```

All are SECURITY DEFINER owned by migration_owner, `search_path=pg_catalog,pg_temp`,
`lock_timeout='5s'`, `statement_timeout='15s'`; qualified relations/functions. REVOKE
PUBLIC and every serving role, grant EXECUTE only migration_owner; check direct-login
`session_user` and no SET ROLE. Save/set/restore actor explicitly on every return and
exception, including initially unset/empty/foreign actor. New operation journal and
recovery grant tables are migration-owned, FORCE RLS, no serving direct DML/SELECT.
Use exactly three new tables: `owner_beta_paper_operations` (generation PK,
installation/account, request_key unique per account, canonical intent/hash, prior/fence
epoch and event IDs); `owner_beta_paper_operation_observations` (observation UUID PK,
generation FK, request_key unique per generation, canonical summary/hash, observed_at);
`owner_beta_paper_recovery_grants` (grant UUID PK, target/lineage FK, original receipt hash,
old/active epoch, consent/process binding, decision/hash, exact T/E admissions, expiry,
settlement/valuation consumption receipt hashes). Intents/observations/decisions are
append-only; consumed-step projection updates require exact receipt equality and remain
audited by existing session events. Unique active grant per target/epoch and active
maintenance generation per configured account; a failed old generation is superseded
only by a linked begin, never removed. Private helpers receive no public grants.
Existing serving function signatures/grants
and configure/consent ABI remain unchanged; bootstrap config still requires SQL NULL
process binding and creates the deposit only once. No lifecycle function fabricates consent.

`begin` atomically records exact intent/key and fences first: lock control row then
account advisory lock381901, advance epoch to `max(expected_epoch, local_epoch_floor)+1`,
state DRAINING (also when originally OFF/RESTORED_OFF for a maintenance generation).
Check overflow, account, manifest and unique generation. Ordinary intent's floor equals
known current epoch; restore floor may be higher from destination protected state.
Intent `purpose=RESTORE` is accepted only through isolated/destination maintenance route.
Store prior state/binding and append DRAIN_STARTED operation evidence. This shared private
implementation must preserve old transition DRAIN's +1 behavior and exact legacy evidence
shape. Legacy DRAIN lacks an operation receipt and cannot by itself finish OFF; `begin`
can attach a new fenced generation from that DRAINING epoch. No table writes from shell.

`inspect(NULL,NULL,1)` is the privileged configured/history probe; UNKNOWN/error never
means EMPTY. Non-null generation must exist, with exact installation/account binding;
readback works while DRAINING/OFF/RESTORED_OFF, Current INVALID and Daily OFF. It does not
call serving authority, grant a permit, check fresh eligibility or select a new E input.
Run pages in one read-only REPEATABLE READ transaction: page_limit1..64, targets ordered
by UUID, exclusive `after_target`, null only at beginning; next cursor null only at end.
Each response uses existing action envelope (`INSPECT_OPERATION`, `OK`) and closed data:
`generation`, `state`, `control_epoch`, `history_present`, `operation_event_id`,
`reconciliation` (schema below or null), `targets`, `next_target`. Each target projects
`target_id,lineage_id,effective_session,lineage_epoch,target_status,session_state,
settlement_sha256,valuation_sha256,ledger_before_sha256,ledger_after_sha256,
account_state_version,outbox_id` (nullable persisted fields remain null). Never return
provider data. Exact existing receipts can be compared by their saved hashes and existing
no-write replay; unknown-commit callers retain their original canonical request bytes.
Summary fingerprints include **all** relevant rows, not just the returned page.
Reconciliation canonical hashes use the narrow schema fields in lexicographic order,
JSON string escaping and decimal integer encoding; SQL must construct that exact
preimage, not hash jsonb::text. Native independently reconstructs and compares it. Encoder
bounds each response256KiB; SQL JSONB is not a canonical authority wire.

`reconcile` requires active generation/DRAINING, validated inventory digest and current
expected epoch; native/controller have independently validated that inventory. It locks
and classifies only dead work as §3, records an immutable reconciliation summary using
[the schema](owner-beta-paper-operations-v1.schema.json), including logical database,
ledger, pending, outbox, jobs roots and unresolved counts. SQL cannot assert host/backend
termination solely from a digest; native checks server sessions before calling and SQL
rechecks no active claim/backend conflict under its maintenance barrier. New observation
uses a new request key; changed evidence under an old key conflicts. Fresh SQL observation
ID/hash is persisted even if response delivery fails. Target page hashes alone cannot
replace complete-set roots. P1/P5 share the audit projection in §7. Reconciliation database_sha256 covers
control/binding plus complete economic/job/outbox state, excluding the operation/grant
journal itself; hash before appending its observation to avoid self-reference. Pending
and ledger roots include immutable original identities. OFF compares the same pre-OFF
projection. The later backup audit includes the complete operation/grant journal at its
separate snapshot boundary; it is not expected to equal the pre-OFF reconciliation root.

`transition_owner_beta_paper_v1(bigint,text,jsonb,text)` stays exact. Add these evidence
variants only: OFF accepts drain receipt; RESTORED_OFF accepts restore receipt; RECOVERY
accepts old v1 unchanged or the v2 decision below. OFF verifies latest reconciled operation,
zero unresolved commits/live claims, current generation/epoch, scope and digest equality,
then increments epoch once and clears active process_binding/digest/cohort/activation
selectors (history stays in immutable events). RESTORED_OFF additionally verifies destination
intent, imported archive/root, source audit equality and local restore generation; it also
increments epoch and clears active selectors. New epoch = prior+1, never wrap. SQL exact
request replay runs before age/stale-epoch checks and returns the original event plus
current state, without claiming that state is still OFF or eligible.

A brand-new isolated destination creates a fresh installation_id and initializes its
local floor from the archived epoch before begin; an existing destination with missing
identity/floor fails closed instead of calling itself new.
A restored old receipt cannot operate on the destination: its installation/generation is
historical, destination `begin(RESTORE)` and epoch-floor are required. Archived operation
rows are retained for audit, not made locally active. If bootstrap has never configured
an account, inventory/backup may record EMPTY but must not invent an OFF account.

**Explicit retained-session recovery.** No lineage, prior receipt, T/E, consent event,
old process hash, admission, order/fill UUID or ledger epoch is rewritten. Enable is still
fresh three-session/current-rights/consent operation. If unfinished sessions exist, its
intent must explicitly name the earliest target via `resume_target_id`; root persists
that intent and native enforces it before ENABLE. SQL independently gates new previews/
apply/automatic target creation whenever old-epoch unresolved sessions exist, until all earlier
retained sessions are VALUED. No invisible drop, skip or separate replacement account.

After ENABLE, explicit `RECOVERY` with closed `owner-beta-paper-recovery-v2` decision
creates one grant for the earliest unresolved target, identified by target+lineage+old
receipt hash+old epoch+new active epoch+consent+process digest+decision hash. Decision is
root-protected operator intent, not an economic receipt. `decision_sha256` hashes the
canonical three-key projection `{effective_session,pending_target_id,step}` from the
explicit request (not the full decision containing that digest); the intent artifact hash
then binds the complete decision. `step` is SETTLE_THEN_VALUE for
unsettled, VALUE_ONLY for settled/unvalued. Grant binds exact E and current admissible
E admission plus immutable T admission, policy/financial/strategy identities. One active
grant per target/new epoch; exact replay returns original grant. It expires for new work
at the next verified KST civil midnight, or immediately on any control/rights/consent/
Current/process change. Failed/expired grants are retained; fresh explicit grant is allowed
under a new key after readback, unlike treating historical RECOVERY_AUTHORIZED as perpetual.

Existing worker scan/snapshot/settlement/valuation ABIs consume the grant through a private
helper. Snapshot returns original lineage identity plus a distinct `recovery_authority`
closed object containing grant_id, active_epoch, active_process_sha256, active_consent_id,
step, expires_at. Existing ordinary snapshots are unchanged; optional field is omitted
when no grant exists. P2/P4 readers must keep historical input identity separate from
current action authority. Original economic receipt control_epoch remains its original
lineage epoch; SQL `expected_epoch` and action runtime use new active epoch. Ordinary
non-recovery path continues demanding equality. Never relabel old bytes as current cohort.

Each new economic write rechecks grant, exact target/earliest order, current rights for
historical data and processing day, current consent, live runtime and original sealed
T/E evidence. Current Daily must independently admit exact E ancestry and policy. Only
release/control/activation identity differences explicitly recorded by the grant may be
bridged; Owner/account/strategy/financial/cost/policy/base/entitlement-content and original
T/E data semantics must match. Input reader must actually verify historical originals
under their original identities and current revocation policy; do not bypass a failed
checker by editing cohort JSON. If current authority cannot reestablish that relation,
return RECOVERY_AUTHORITY_UNAVAILABLE, stay held, block new target creation with the exact
target/reason. This deliberate whole-account fail-closed state needs corrected evidence
or a separately specified product disposition; P0 grants no cancellation/loss recognition.
It is not an accidental endless scan: expose the blocker, deny ENABLE without explicit
resume intent and deny ungranted automatic retries. Compatible grants provide the normal
path that clears the earliest target and releases later sessions.

Consume SETTLE atomically with original settlement/ledger/outbox transaction, then only
VALUE remains. VALUE_ONLY cannot settle. Consume VALUE with exact E valuation; grant
becomes terminal and next earliest may receive its own explicit grant. Already committed
receipts replay before grant/rights/epoch checks with no writes; changed receipt conflicts.
Settlement history already tagged scheduled keeps its original origin; append recovery
valuation event separately and exclude the recovered session from unattended streaks.
Unsettled recovered work uses existing RECOVERY processing origin. No new deposit or fill
for settled history. DRAIN/restore revoke remaining grant steps without deleting them.

Down0068 refuses any operation/grant/event history; empty rollback restores exact0067
function definitions/owners/grants. Nonempty rollback uses a compatible OFF release.
SQL errors: 22023 malformed;23514 integrity;23505 changed replay;40001 stale epoch;
42501 wrong principal/current rights;55000 unresolved/unsupported recovery. Bridge emits
only fixed codes, never DB/provider messages, parameters or secrets.

## 5. Native bridge and ENABLE lifetime

P2 adds **`owner-beta-paper-ops-native` in Cargo package `api-server`**, source
`crates/api-server/src/bin/owner-beta-paper-ops-native.rs`, using a narrow operations
library in `crates/job-queue/src/owner_beta/paper/operations.rs`. Its checked constructors
stay private; expose only the consuming bridge needed by the entrypoint. This coordinator
integration choice keeps D7's existing single-package `api-server` producer contract;
it avoids introducing a heterogeneous-package recipe only for this helper.
P3 builds/packages it as a second D7 Paper image ELF
`/usr/local/bin/owner-beta-paper-ops-native-bin`; no new service or scheduler. Root invokes
the binary directly in a one-shot container from the exact manifest image, entrypoint
overridden, no shell, no restart, no Docker socket. CLI is exactly:

```text
owner-beta-paper-ops-native-bin <configure|inspect|enable|begin|reconcile|off|restore-off|recover>
```

Verb mapping: begin accepts STOP/BACKUP/RESTORE; reconcile/off reuse STOP/BACKUP/RESTORE;
restore-off requires RESTORE. Other verbs match their uppercase purpose (recover=RECOVERY).
ENABLE/RECOVERY intents have root-journal generations; their unchanged SQL transition
envelope is anchored by request key `<generation>:enable` or `:recover` and immutable event.
They do not call begin, which would fence away the enabled epoch. STOP/BACKUP/RESTORE
generations additionally have the SQL operation journal.

No other args or credentials/paths/IDs via argv or ambient env approvals. Root stages
bounded input at fixed `/run/owner-beta-paper-ops/request.json` and referenced canonical
artifact at `/run/owner-beta-paper-ops/evidence.json`; originals mounted read-only,
root0400, directories0700. Request is the schema's intent, whose purpose must match the
verb; selected evidence digest must match. `inspect` can use an INSPECT intent without
persisting an operation. Configure consumes the closed `owner-beta-paper-configure-operation-v1` artifact: selected
strategy_config_id, fixed display name `Owner-beta Paper` and the exact adopted SQL
contract object. It calls existing configure(strategy_config_id,account_name,contract,
SQL NULL,request_key); Owner is resolved by existing Daily SQL authority. CONFIGURE and
unconfigured INSPECT alone allow null account_id with expected_epoch0; other intents
require the configured account. It never chooses finance values or creates consent. Root-selected DB host/name/port are
nonsecret protected connection config; passwords are mounted one file each at
`/run/secrets/db_migration_owner_password` and `db_worker_password`, root0400. No app,
admin or audit credential is a substitute. Helper refuses nonroot lifecycle entry.

The connection selector is fixed at `/run/owner-beta-paper-ops/database.json`, root0400
under the same protected0700 directory. Its closed object is
`{schema:"owner-beta-paper-ops-database-v1",host,port,database}`; host is exactly
`postgres` (the existing selected backend alias) or `/run/postgresql` (the shared local
socket directory for isolated verification), port is integer5432, and database matches
`[A-Za-z_][A-Za-z0-9_]{0,62}`. It contains no user, password or URL. The root controller
derives it from the already verified release/DB context and selected isolated server;
it cannot be supplied through argv or ambient env. Native retains/rechecks its descriptor
identity through the operation and constructs the two fixed-role connections internally.
This coordinator-frozen file detail adds no intent field or SQL ABI change. Socket
verification shares only the selected private PG socket; it does not publish a TCP port
or give the isolated verifier a delivery/provider network.

Worker login is used only for genuine eligibility/input checks; migration_owner only for
narrow lifecycle/maintenance SQL. The existing `owner-beta-runner paper-evidence` retains
reserve/link-admission/link-run/read with its original worker/admitter privilege. Do not
add lifecycle verbs or migration credential mounts to that helper or serving API/runner.

Extend checker with a private callback/consuming operation, not a serializable permit.
At `evidence.rs`'s actual checked DB/source join, retain typed date, run_id,
admission_sha256, source_event_sha256 and root_event_sha256 (do not parse opaque diagnostic
identity strings later). Retain all three sessions, current cohort, rights transaction,
Current and protected Raw/Daily/source reader handles through the lifecycle call. Revalidate
before SQL, after waiting for locks, immediately before COMMIT and at consuming cleanup.
Build existing ENABLE object directly from these checked facts, selected process binding,
expected/new epoch and current evaluated/expires times. SQL independently rechecks the
same joins/consent/current authority. No report import/export flag; three date strings or
arbitrary SQL query after checker cleanup are insufficient. Release resource/claim cleanup
failure propagates even if SQL committed; read back exact event, record outcome, and DRAIN
if enabled authority can no longer be maintained. Never report a clean rollback after
COMMIT. Existing eligibility report API remains descriptive and unchanged.

ENABLE starts from OFF/RESTORED_OFF; configure/bootstrap/readiness must work with no
serving runtime. P3 prepares canonical next-epoch process config without claiming it is
active. P2 verifies its exact bytes, actual API/runner ELF extracted from selected images,
both compile revisions, manifest and native ELF/capability pins independently of report
JSON. It does not call `TrustedPaperRuntime::load(Worker)` on the different helper ELF.
For ENABLE after OFF/restore/release-semantic reset, all three genuine source ARM
origins must be after the last reset boundary, not merely the latest three cached dates.
Native compares protected ARM clocks/events with the durable reset event, and SQL checks
corresponding reservation/run creation and session identities against that reset. A source
slot already armed before reset cannot be relabeled. If three fresh sessions are missing,
report INELIGIBLE and continue OFF observation through existing B. This is the adopted
new-observation requirement, not a second financial approval.
After ENABLE durable readback and epoch-floor persistence, P4/P3 may start exact bound
API/runner; start failure fences again. Root release lock prevents sanctioned config/image
replacement during the transaction. Existing runtime itself revalidates fixed config and
`/proc/self/exe`. Source B starts/observes future slots independently while Paper is OFF.

The one-shot helper has one writable private bind mount,
`/run/owner-beta-paper-ops/output` (root0700). It fsyncs content-addressed canonical
reconciliation/readback artifacts there; controller validates bytes and imports them
no-replace into the operation directory. No service reads this directory. A digest on
stdout without its complete validated artifact is insufficient. Per-target maintenance
readback pages stay <=256KiB; new operation envelopes remain <=64KiB.

Output is at most64KiB closed status `{schema,verb,outcome,event_id,generation,
control_epoch,artifact_sha256,error_code}`; schema `owner-beta-paper-ops-result-v1`.
Outcomes OK/REPLAY/BLOCKED/COMMIT_UNKNOWN; optional identities are explicit null.
Exit0 only OK/REPLAY,2 invalid request,3 blocked,4 unknown; exit is never proof. Fixed
errors include WIRE_INVALID, IDENTITY_CONFLICT, STALE_EPOCH, CLOCK_INVALID,
WRITER_UNKNOWN, COMMIT_UNKNOWN, READER_INCOMPATIBLE, RECOVERY_AUTHORITY_UNAVAILABLE,
EVIDENCE_UNAVAILABLE, INTERNAL_FAILURE. stdout contains no secret or free-form diagnostic.

## 6. Release, capability, aliases and helper handoff

P3 stages in `<release>/deploy/owner-beta-paper/`: `capability.json`, `process.json`,
`operations.schema.json`, `archive.schema.json`, `migration-0068.up.sql` and
`audit.sql`. Include all actual source hashes and both API/runner plus native ELF hashes
in capability; hash capability in protected install receipt, bound to manifest+commit.
Do not put image's own manifest hash inside its build-time image content (circular hash).
Root stages after final image manifest exists, verifies extracted ELF bytes and compiled
revision, then seals release; staged process.json root:10001 0440 (directory0750),
private manifest/install receipt root0600. Installed immutable releases never get patched.

Persistent `.owner-beta-paper-reader-floor` root:root0600 contains exact canonical
capability floor `{schema:"owner-beta-paper-reader-floor-v1",minimum:1}`. Any history,
config/control, operation journal, old Paper floor or requested ON requires a validated
Paper-capable reader even in OFF. UNKNOWN inventory fails. Reserve floor before current
switch. Floor loss is failure, not permission for a legacy release. Preserve existing
Daily/simulation floors conjunctively. Capability includes source-owned schema, migration
and audit SQL digests; old capability string without matching code is insufficient.

Root binding maps process file to **`/run/owner-beta-paper/config.json`** in both API and
paper-scheduler, read-only. Consumer aliases stay exactly:
`/data/owner-beta-paper-evidence/{events,current}`, `/run/owner-beta-daily/{config,policy}.json`,
`/data/raw`, `/data/owner-beta-daily/admitted`, `/data/owner-beta-artifacts`.
Mount current **directory** read-only, not leaf. Preserve each source metadata rule;
no full release/env/secret/calendar/private witness tree mount. Root source binding
retains host paths/hashes; aliases do not rewrite semantic identity. Helper gets these
same read aliases plus private operation/manifest selectors and only its two secret files.
Restore publishes INVALID with no OFD writer. Only existing B can later mint fresh CURRENT
under its original boot/generation/guard rules; never serialize/restore an OFD capability.

Exact host helper API (Python, no generic shell success protocol):

```text
paper_release.stage(install_root, target_release, commit, manifest_sha256, process_binding)
paper_release.verify(install_root, target_release, commit) -> VerifiedPaperRelease
paper_release.reader_guard(install_root, target_release, commit, history_present, reserve)
paper_operations.begin(context, release_lock_fd, purpose, scope, request_key) -> Operation
paper_operations.drain(operation) -> DrainedHandle
paper_operations.revalidate(handle) -> None   # raises typed failure
paper_operations.commit_off(handle) -> immutable event identity
paper_operations.restore_off(handle, archive_root, destination_audit) -> immutable event identity
paper_operations.resume(context, release_lock_fd, request_key, earliest_target) -> event identity
```

These handles are private constructors, own expected generation/live lock lifetime,
no Deserialize/public `verified` flag. `drain` leaves locks held until caller exits its
scope. P5 receives the handle, never a bool/hash as a stopped capability; revalidate before
and after snapshot and before publication. Host CLI controller requires `--lock-fd N`
from official wrapper, action enum and request key only; root config selects all paths.
P3 alone edits shared shell gates, Daily coexistence and deployment handoff. Keep existing
Paper-enabled rejection until P1/P2/P4 consumed interfaces pass together. Coexistence
requires verified installed context; Daily disable/maintenance must remain possible even
when Paper is broken. No legacy static systemd activation or second source scheduler.

## 7. Paper-aware combined archive and isolated restore

New file format magic **`LAGRANGE_PAPER_COMBINED_FILES_V1\n`**, profile
`paper-combined`, root schema `owner-beta-paper-archive-v1`. Do not reinterpret old
Daily/simulation formats. Even a Paper-only deployment selects this format if any Paper
history/config/floor exists. Required classes: existing raw,curated,daily,config,floor,base,
simulation_input/config/floor/receipt/manifest/reader/approval, plus paper_evidence,
paper_operations,paper_config,paper_floor,paper_contract,paper_manifest,paper_reader.
Every class has a stream digest, entry count and byte total; arrays use the schema
lexicographic class/group order (writer rows sort by writer_class,kind,identity_sha256).
Absent optional legacy
history is explicit EMPTY with validated inventory and empty stream hash, not omission.
No credentials/env secrets, lock authority, ephemeral request password files or socket
objects are archived. Retain protected source originals; nonsecret manifest/config/policy
projections preserve hashes of private originals without copying credentials.

One common COMBINED_BACKUP generation and retained writer/publication fences cover DB
snapshot and files. The maintenance connection opens REPEATABLE READ, obtains exported
PG snapshot for dump and source logical audit; hold transaction until dump completes.
Hash snapshot binding in archive, never expose a live snapshot token as authority.
For a configured installation archive/audit account_id is required. An entirely
unconfigured installation remains on existing Daily/R4 backup only if the locked SQL
inventory proves no Paper history/config/floor/journal; inconsistent presence fails. Stream
all file manifests under held publication locks, compare before/after source roots and
writer generations; reject differences. Encrypt staging with existing approved backup
mechanism and private limits. Only publish after isolated actual PostgreSQL restore and
full audit equality. Unknown/missing Paper inventory must not fall back to legacy backup.

Audit schema `owner-beta-paper-audit-v1` fixes groups: schema_security, paper_control,
paper_lineage, paper_ledger, paper_outbox, paper_jobs, paper_operations, daily, simulation,
source_files. For each, source-owned audit.sql declares exact table/column projections,
PK ordering, JSONB normalization and relationships; no SELECT * or count-only proof.
The source_files group is produced by P5 from protected file manifests and compared
against SQL reference sets; SQL does not pretend to read/hash arbitrary host files.
Canonical streamed row bytes use length-prefix framing, not ambiguous concatenation;
root = SHA256 of ordered `(table-name, row-key, canonical-row)` frames, each ASCII
component prefixed with decimal byte-length and `:`. Empty stream uses SHA256(empty).
Count and byte-length are diagnostics, hash and cross-link checks are mandatory.

Coverage: accounts/bindings/control/consent/events; previews/targets/lineage/sessions/
session events and settlement/valuation receipts; cash ledger, positions, orders/fills,
daily_equity; jobs/attempts/reservations/recommendation publications; active and archived
paper_settlement_outbox, notifications.source_key, notification_deliveries(notification_id,
channel) and all delivery/attempt/status identities;0066 source slots/admissions/runs;
all Daily/R4 history; schema migrations/checksums, function definitions/owners/grants,
FORCE-RLS/policies/triggers/constraints and role membership. Source_files covers every
referenced Raw/Daily/historical/source original, contract/cost/approval/config/floor and
release capability pins. P1/P5 must freeze exact audit projection hash in release.

First compare restored DB/files **before** any RESTORED_OFF mutation against source
roots, then append destination operation and compare a second audit whose only allowed
delta is the exact local fence/restore events, epoch/active-selector clearing and revoked
recovery grants. Do not normalize away missing economic or notification rows to get equality.
Restored verifier runs isolated postgres with no published port/network delivery; native
maintenance may use only its isolated local DB connection. No API/worker/witness/provider
or notification agent is started. Verify grants/RLS with real logins in isolation; SQL
catalog hashes alone are insufficient. Cleanly reap verifier children; failed archive stays
private/unpublished. verify-latest proves an isolated copy, never production restore.

Real destination restore uses new installation-local generation and external epoch floor,
never archived current binding/live lock. All services remain disabled/RESTORED_OFF. Before
any later notification consumer starts, reconcile archived delivery identities against
retained destination delivery history. If external delivery after the snapshot is unknown,
quarantine that identity; no automatic resend or new notification key. New economic resume
still requires §4/§5 and new observation; no inherited Daily auto-resume branch for Paper.

## 8. Packages, disjoint ownership and evidence gates

| Package | Exclusive implementation scope and dependencies | Required failure tests |
| --- | --- | --- |
| P1 SQL | NEW0068 up/down; existing paper_0067 integration module/discovery fixture and r5-paper-sql.py only as needed. Implements §4, audit projection contract supplied to P5. Depends P0. | real PG wrong role/SET ROLE/unset-empty-foreign actor, stale epoch/key/gen, changed replay, malformed receipt, imported generation, interrupted COMMIT and readback after Current invalid/Daily OFF, dead/live claims, grants pre/post settlement and expiration, old immutable lineage, exact replay zero writes, history/down guard. |
| P2 native | NEW job-queue operations module and narrow library surface; NEW api-server native operations bin and only its Cargo entry if needed; job-queue evidence.rs/contract.rs/mod.rs, historical-vs-current grant validation in Paper input/custody modules if required. No serving API/HTTP DTO or ops edits. Depends P1 ABI; P3 packages exact ELF through the existing api-server D7 producer. | fake eligibility JSON, fact/date/run swaps, wrong protected origin, missing OFD, clock/TTL changes during lock wait, cleanup after commit, wrong compiled revision/ELF, bootstrap OFF/no consent, historical input rejection, no source-helper privilege growth. |
| P3 release | NEW paper-release.py/tests; shared build recipes/runtime/Compose/install/validator/compose-release/Daily gates/status policy. Adopt concurrent compile-revision fix by handoff, never overwrite it. Depends P0; stage OFF independently, activate only P1/P2/P4 integrated. | wrong/missing binary/revision/hash, wrong alias/leaf mount, unsafe metadata, floor UNKNOWN/history/legacy rollback, partial stage/current switch, semantic drift, disabled-only fallback and coexistence. |
| P4 controller | NEW paper-operations.py/tests and runbook; explicit API native runner/session/action scope for recovery-authority field consumption, maintenance delivery gating and actual cleanup only after handoff from P2. No Web/HTTP DTO or P3 shared shell changes. Depends P1/P2/P3. | stop during SQL wait/commit/blocking callback, PID/container replacement/restart drift, unresolved backend/prepared transaction, every stage disconnect/reboot, lock inversion, expired receipt, fail-closed grants, repeated explicit recovery then next target, real process/reap tests. |
| P5 backup | Existing production backup wrapper, daily-backup.py/files.py and relevant backup tests; NEW deploy/db/owner-beta-paper-backup-audit.sql and Paper tests. Depends P0 audit ABI, completes after P1/P4. No lifecycle reimplementation. | missing classes/rows/originals, mixed generation, DB/files race, altered ledger/outbox/delivery/archive identity, corrupted encrypted set, old reader, restore replay/destination floor, no-network/delivery and no-auto-ON, actual isolated PG restore equality. |

P1/P5 agree a single exact projection; P1 owns SQL helper definitions, P5 owns audit.sql;
P3 pins its hash only after both agree. P2 supplies `recovery_authority` wire before P4
changes API internal decoder; unrelated WP4 Web/HTTP DTO ownership remains untouched.
No two workers edit one file. Coordinator owns plan/ROADMAP, test registration/required
CI aggregation, diagrams and final same-candidate integration. This P0 edits none of them.

Required hosted jobs remain actual PostgreSQL migration+named Paper tests, existing
backup Docker harnesses, CI/research-smoke on the same candidate; new tests must be
registered, no implicit discovery or SKIP acceptance. Pure fixtures establish protocol
handling only. Existing default15s operation,90s cycle,90s inner action,10s heartbeat,
60s lease and4/4/2/2 pool configuration are not relaxed. Actual release ELF under CPU1/
2GiB plus heartbeat renewal, bounded cleanup and capacity observation belongs to WP7;
debug90s functional evidence is not normal15s performance. Production builds later obey
one service per Compose graph, Cargo2/Compose1 and batch health checks.

P0 is complete only as a reviewable source specification/schema. P1–P5 implementation,
G3 integration, actual installation/drain/restore/timing, fresh B-origin pre3 sessions,
separate post3 unattended Paper sessions and Owner acceptance remain unverified followups.
