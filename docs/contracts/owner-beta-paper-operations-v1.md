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

**P0-B amendment:** §9 adds one bounded read-only function to this original
three-function limit. It also specifies additive private receipt columns; existing
public signatures and response shapes below remain unchanged. §9 is a proposed
source protocol checkpoint for coordinator adoption, not implementation acceptance.

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
R5-capable staging additionally retains the existing official producer's
`artifact.json` and `COMPLETE` for api-server, paper-runner and
owner-beta-paper-ops-native, bound to their extracted ELF hashes and exact compile
environment/revision. Reuse the guarded D8 receipt-transfer pattern for D1/D7; their
fixed image paths are `/usr/share/lagrange/owner-beta-paper-provenance/<bin>/`.
The ordinary source-builder route may remain available, but missing official
receipts make that image ineligible as an R5-capable reader. Never manufacture a
receipt from a label or an ELF literal. This adds no manifest version or self-hash.
Retain the original pairs under the release's `deploy/owner-beta-paper/provenance/`
and stage them at `/run/owner-beta-paper-ops/provenance/<bin>/` for the native
ENABLE/observe-reset verifier: exactly the three named bins, directories root0700,
files root0400, no-replace and retained named/FD identity. Check the exact producer
schema, COMPLETE digest, compile environment/revision and actual corresponding ELF
SHA-256 independently. Offline exact readback gains no Current/worker/provenance
dependency. The one-shot helper runs inside its exact manifest container, as §5
requires; an extracted Alpine ELF path is not a host-execution allowance.
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

## 9. P0-B: exact readback and ENABLE reset proof (2026-11-06)

This amendment freezes only the two source seams identified at `6ac6a0c8e9665aff84d6e563d49b4ab4f3c18e73`.
Status: **source protocol amendment adopted by the coordinator on 2026-11-06**. It supersedes
§4's three-public-function limit and makes §5's reset/readback requirements precise.
It does not accept P1/P2, remove their gates, implement controller/chain/recovery work,
or claim CI42179289182, PostgreSQL, compiler, deployment or observation success.
0067 and the original Paper/SQL/Current/ADR contracts remain byte-immutable.

### 9.1 One additional read-only ABI

```sql
public.read_owner_beta_paper_operation_result_v1(
    request_canonical text, request_sha256 text
) RETURNS jsonb
```

There are exactly five closed request discriminants, `CONFIGURE`, `BEGIN`,
`RECONCILE`, `TRANSITION`, `RESET`. This is an exact request/result reader, not a
query language. No table name, SQL, projection, arbitrary predicate, pagination,
actor selector, role or credential is accepted. RESET only reads the one account's
current reset boundary; it never records a reset. The schema adds `exactRequest`,
`exactReadback`, `resetBoundary`, `resetSelection`, `resetAnchor`, and `queryResult` definitions.

Every request is `{schema:"owner-beta-paper-exact-request-v1",kind,account_id,args}`.
CONFIGURE alone has null account_id (the original call does not yet know it);
others require the configured nonnil UUID. `args` has exactly the following keys.
`text`, UUID, bigint and jsonb here identify the actual SQL argument conversions:

| kind | Exact args and SQL mapping |
| --- | --- |
| CONFIGURE | `strategy_config_id` UUID, `account_name` ASCII text1..64 matching the existing name grammar, `contract_canonical` text containing the closed SQL-v1 contract, `process_binding` JSON null (convert to SQL NULL), `request_key` text. Calls configure(uuid,text,jsonb,NULL,text). |
| BEGIN | `expected_epoch` nonnegative bigint, `intent_canonical` text containing the existing closed intent, `request_key` text. Calls begin(bigint,jsonb,text); account and epoch must equal the embedded intent. |
| RECONCILE | `generation` UUID, `expected_epoch` nonnegative bigint, `inventory_sha256` digest, `request_key` text. Calls reconcile(uuid,bigint,text,text); generation must resolve to the same account. |
| TRANSITION | `expected_epoch` nonnegative bigint, `transition` exactly ENABLE/DRAIN/OFF/RESTORED_OFF/RECOVERY, `evidence_canonical` text containing the exact existing closed variant, `request_key` text. Calls transition(bigint,text,jsonb,text). No ENABLE shape extension. |
| RESET | `expected_epoch` nonnegative bigint. No mutating call; compare with current control epoch or fail40001. |

Keys retain their existing per-function grammar, including generation suffixes.
The SQL preimage is **the entire canonical exactRequest object**, with embedded
canonical JSON represented as JSON strings. Hash its ASCII bytes with SHA-256.
The four mutation functions construct this same preimage from their typed arguments
and DB-resolved account (CONFIGURE always null in the preimage), using the canonical
encoder for embedded JSON. They do not accept a new argument or trust a caller hash.
The full outer protected intent and selected artifact are separately retained and
bound by native; they cannot be reconstructed from this SQL preimage. Native must
persist both, including installation, generation, release, account (null for the
original CONFIGURE), key, epochs, original evaluation/expiry times and every exact
SQL argument. Never replace them with newly evaluated evidence for readback.

Use §2's duplicate-rejecting canonical rules recursively on embedded JSON. Maximum
exact request/readback:1MiB, depth16; each embedded SQL argument/result:256KiB,
with the existing smaller variant limits still enforced. These larger **readback**
limits only accommodate JSON string escaping, not larger mutation inputs. Legacy
fractional timestamp strings inside ENABLE evidence remain byte-exact; do not
round them to §2's whole-second format. SQL validates the original text, parses,
re-encodes and compares it byte-for-byte before trusting its supplied hash. A hash
match alone is insufficient. Compare full canonical preimage AND hash; existing
legacy `payload_sha256` remains its original serializer's digest, never relabeled
as this new request digest. No new self-hash field enters the preimage.

Return exactly `{schema:"owner-beta-paper-exact-readback-v1",kind,request_sha256,
status,account_id,current_epoch,current_state,event_id,event_epoch,
result_canonical,result_sha256,reset}`. All keys are required; unavailable fields
are JSON null. current_state is OFF/ENABLED/DRAINING/RESTORED_OFF/BLOCKED or null.
`reset` is null except for successful RESET. Closed statuses and semantics:

| status | Meaning and populated fields |
| --- | --- |
| COMMITTED | Mutation only. Exact full request and atomic stored result found. Return original result canonical bytes/hash, original event ID/epoch (RECONCILE: observation_id/fence epoch), and separately the currently observed account/epoch/state. Historical result is not current authority. |
| ABSENT | Mutation only. No request receipt or matching legacy identity is visible in this snapshot. All event/result/reset fields null. This is **not** NOT_COMMITTED and never grants permission to resubmit, choose a new key, or create economics. |
| UNPROVEN | Mutation only. Legacy identity exists without a complete exact receipt; original full args/result cannot be proved. Event/result/reset fields null. No guessed backfill, replay or authority. |
| RESET | RESET only. Matching configured account/expected current epoch and unique reset found. reset populated; event/result fields null. |

Malformed input/hash mismatch:22023; changed bytes under an occupied identity:23505;
corrupt stored digest/result or inconsistent joins:23514; wrong principal/account:42501;
stale RESET epoch:40001; missing/ambiguous reset:55000. Errors do not become ABSENT.
For UNPROVEN, provably changed legacy payload is still23505; incompleteness alone
is never evidence of equality. For CONFIGURE before any account exists, ABSENT has
null account/current epoch/state; lookup resolves only the singleton configured
account afterward, without requiring current Daily enabled/rights/strategy status.
Other requests for a missing/different configured account fail42501, not ABSENT.

Identity namespaces preserve existing uniqueness: CONFIGURE and TRANSITION share
`(account_id, owner_beta_paper_events.request_key)`; BEGIN uses both generation and
`(account_id, operations.request_key)`; RECONCILE uses `(generation,request_key)`.
A collision in either BEGIN identity conflicts. Event kind, all original args,
including expected_epoch, and generation must match. RESET has no replay key.
An exact read repeated after state/epoch changes returns the same original result;
only current_* may differ. It executes no mutation/replay function and writes no
receipt, ledger, event, observation, outbox, lock row or audit counter.

Function authority: SECURITY DEFINER, migration_owner owner, qualified names,
`search_path=pg_catalog,pg_temp`, lock_timeout5s, statement_timeout15s. Explicitly
revoke PUBLIC/app/worker/admin/audit_writer/research_writer/daily_input_admitter;
grant EXECUTE only migration_owner. Require direct-login session_user=migration_owner,
role setting `none`, LOGIN and no superuser/createdb/createrole/replication/bypassrls.
Save actor, set to DB-resolved Owner before tenant reads, restore on every return
and exception (unset may return PostgreSQL's empty placeholder). A foreign caller
actor is ignored/restored, never used to select another tenant. FORCE RLS stays on;
no caller direct table SELECT/DML grants. This reader uses plain SELECT, no FOR UPDATE/
SHARE or advisory locks, and no serving-authority/Current/worker checker dependency.
Ordinary readback runs in a fresh read-only REPEATABLE READ transaction with native
outer15s bound; after an unresolved result discard its snapshot before another read.
The client sets LOCAL lock_timeout5s and statement_timeout15s **before** submitting
the SELECT, in addition to the function settings; do not rely on an in-function
timeout change to bound an already running statement.
A RESET check inside the existing ENABLE write transaction is also a read-only
statement and observes that transaction's held locks/state. No new connection role.

### 9.2 Atomic receipts and uncertainty

Keep exactly the existing three new journal tables. Add nullable private columns
`request_canonical text`, `request_sha256 text`, `result_canonical text`,
`result_sha256 text` to **events, operations, operation_observations**. The four are
all null for older/unrelated rows or all nonnull and mutually validated. No history
UPDATE/backfill. Newly successful configure/transition write them on their event;
begin on its operation; reconcile on its observation, in the same transaction as
all original side effects. Construct the original OK envelope before INSERT, with
its already chosen UUIDs, and store it exactly; do not UPDATE an immutable event.
Implement the configure replacement in0068 at its original event INSERT, preserving
the0067 function signature/outputs/deposit behavior; a wrapper that calls legacy then
updates its immutable event is forbidden. The same insertion rule applies to ENABLE
after legacy validation: factor/reproduce its final insertion in0068 so the receipt
is present in that INSERT, rather than calling legacy to insert and then patching it.
All successful new executions in these four families require a receipt. Validate
stored receipt against event/operation/observation account, kind, key, epoch and
result ID. Existing no-write replay outputs keep their adopted shapes; receipt
lookup precedes stale-state/freshness/current-authority checks, validates the full
original preimage, and performs no writes. Legacy rows keep legacy replay behavior
where applicable but never acquire an exact receipt through replay.

The reset-only reader needs no receipt columns to identify old durable events.
The P1 followup must extend its private audit projection and guarded down migration
for these columns; the archive envelope/schema itself does not change. Atomic
receipts must be included in archive roots, retained through restore, and protected
by the existing immutable triggers/RLS. No fourth table or new role is introduced.

Before any mutating SQL is submitted, native fsyncs the exactRequest and original
intent/artifact bindings into the existing private durable output/journal handoff.
Failure to persist aborts before submission. An unresolved request stays pending
under that identity even after reboot/helper exit. At timeout, cancellation, lost
COMMIT acknowledgment, or post-COMMIT cleanup failure, retain it and use only this
reader. A timeout before COMMIT is safe rollback only after a definite rollback
acknowledgment on that same transaction; a dropped handle/socket or timeout is not
an acknowledgment. COMMITTED proves durable mutation, including commit-before-timeout
and commit-after-timeout. It does not prove successful cleanup or authorize start.
ABSENT while the original backend/transaction may live remains COMMIT_UNKNOWN.

The reader deliberately offers no negative terminal proof. NOT_COMMITTED requires
§3's independent original backend identity disappearance, prepared-work exclusion,
no reconnect/resubmit, final fence and fresh read after that fence. If that existing
protocol cannot cover an unconfigured CONFIGURE, keep it unresolved; this amendment
does not create a controller recovery path for it. A fence, replay or second mutation
must never be issued merely to make a read succeed. No automatic economic retry.
After cleanup failure plus COMMITTED ENABLE: preserve the cleanup error, record the
committed event, forbid startup and hand off the existing explicit DRAIN procedure;
do not report rollback or silently turn readback success into clean ENABLE success.

### 9.3 Exact reset boundary and causal ARM evidence

The latest applicable durable reset is the unique event for the configured account
and Owner with maximal **control_epoch** among CONFIGURED, OFF and RESTORED_OFF.
CONFIGURED is epoch0 and is the initial observation reset. OFF and RESTORED_OFF
retain their actual resulting epoch and event ID. Ignore occurred_at and UUID order
when selecting; if two reset events occupy the maximum epoch, or an event exceeds
current epoch, fail55000. Consent/DRAIN/ENABLE/RECOVERY are not resets. A consent
change may advance current epoch without replacing the reset; still require exact
current expected_epoch at ENABLE. Missing reset/history is failure, never epoch0
by assumption. A failed/rolled-back transition creates no reset.

Release/semantic/Owner/strategy/cost/policy/rights reset uses the already specified
begin → OFF (or destination RESTORED_OFF) after quiescence and selection of the new
assets, **before any qualifying ARM**. This applies even if Paper is already OFF.
No inferred reset from file mtime, release name, opaque ID or newest timestamp. Do
not reuse an earlier OFF to bless a later semantic change: the protected operation
and selected release/cohort must bind the new reset. Restore selects a destination
local reset above the protected epoch floor, never the imported maximum timestamp.
Imported files/receipts remain historical. Simple process restart is not itself a
semantic reset; the adopted current-authority and interrupted-session rules remain.

The local no-replace selection for one reset is
`{schema:"owner-beta-paper-reset-selection-v1",installation_id,reset,
release_manifest_sha256,cohort_sha256}` at
`.owner-beta-paper/reset-selections/<reset-event-uuid>.json` (0700/0400).
Root seals it once, after that reset commits and the new selection is verified,
before any anchor. Every later anchor for that reset must match it; a changed
selection requires a new durable reset, never replacement of the file. This also
binds initial CONFIGURED to its first selected release/cohort. On an established
installation a missing selection fails closed, not permission to initialize it
against historical ARM evidence. Its producer is the existing trusted root
controller under the release lock; native validates its protected original via
fixed `/run/owner-beta-paper-ops/reset-selection.json`. This is a narrow local
handoff record, not SQL-authenticated or imported authority. P4 must implement its
publication before ENABLE activation; that controller work is not accepted here.

Closed `resetBoundary` is `{account_id,event_id,kind,control_epoch,occurred_at_us,
payload_sha256}`; kind is CONFIGURED/OFF/RESTORED_OFF. `occurred_at_us` is the exact
nonnegative integer microseconds since Unix epoch of DB occurred_at, never rounded
seconds or floating-point arithmetic. Reject overflow/nonfinite times. The digest
is the stored original event payload digest, not a new canonical digest of it.

DB wall time is not ARM causality. Introduce a **local protected reset anchor**:
`{schema:"owner-beta-paper-reset-anchor-v1",installation_id,reset,
release_manifest_sha256,cohort_sha256,boot_id,observed}`. `observed` is §2's actual
kernel clock triple, sampled by native only **after** a successful committed-view
RESET query. Root keeps the anchor no-replace/fsynced at
`.owner-beta-paper/reset-anchors/<canonical-sha256-hex>.json` (directory0700,
file0400; full existing ancestor/FD/ACL/nlink rules). It is generated locally under
the held release lock after verifying selected release/ELFs/capability and protected
Current's cohort; it is not a caller timestamp, SQL parameter or importable permit.
OFF observation may start B sufficiently to publish Current; anchor creation does
not require three past sessions. The anchor records a completed DB observation of
the reset, not the reset's estimated kernel time. SQL cannot authenticate it.

For each checked session require a retained local anchor with exactly the same
installation, reset identity (all fields), selected manifest/cohort and ARM boot.
At the genuine source/DB join, capture both ARM_INTENT and ARMED events from the
already validated source chain. Require ARM_INTENT and ARMED `observed` triples
strictly after the anchor on all three clocks; equal ticks fail. Their inherited
arm boot/generation/nonce/parent, slot, reservation, invocation and cohort must
match the RESERVED→source→admission→run chain. Source canonical event digests and
sequence ordering are evidence, not meanings inferred from UUIDs. Require
anchor.realtime_ns > reset.occurred_at_us*1000, with checked arithmetic. Native's
clock is not used to substitute for the independent SQL creation-time checks.

Monotonic/boottime comparisons occur only within the same kernel boot. After reboot,
create a new local anchor by another committed-view RESET read **before new ARM**.
It refers to the same reset if no reset occurred; it does not restart the three-day
count. Historical sessions from other boots remain eligible only with their own
retained original local anchors and protected source chains. Never order boot UUIDs,
compare monotonic counters across boots or certify past ARMs using a new anchor.
Missing anchors fail EVIDENCE_UNAVAILABLE; no reconstructed/backdated anchor.
Restore must not import anchors as destination authority, even on the same kernel.
A realtime rollback before the recorded boundary fails comparison; any clock failure
inside the source ARM lifecycle fails its existing rules. An anchor is durable
historical evidence without30s/180s expiry; do not apply a fresh-observation age or
one-second **multi-day** drift limit to it. Live Current/eligibility and source clock
checks retain their original bounds. Monotonic causal order plus the genuine chain,
not wall timestamps alone, supplies the ARM-after-reset proof.

SQL independently holds the control row then account advisory381901 lock, checks
expected_epoch and derives this same reset before new ENABLE (after exact replay).
Retain every0067 session/expected-calendar/current-consent/rights/cohort comparison.
For each of exactly three sessions join recommendation_runs r → daily_reservations dr
(run_id/job_id) → source_runs sr (run_id/job_id) → source_slots ss (slot_id) →
source_admissions sa (slot_id) → daily_admissions a (admission_sha256).
Additionally require r.strategy_config_id=c.strategy_config_id, sr.job_id=r.job_id,
dr.trigger_version='owner-beta-daily-trigger-v1'; preserve all0067 owner, session,
config/activation epoch, full cohort, admission/source/root hash equalities.
Require **ss.reserved_at, dr.created_at, r.created_at, sa.linked_at, sr.linked_at**
strictly greater than reset.occurred_at and no later than SQL clock_timestamp() at
validation. Null/equal/future/pre-reset times fail23514, including a transaction
started before reset whose r.created_at is transaction-start time. Do not substitute
finished_at/publication timestamps or lexical run IDs. SQL timestamps constrain DB
history; they cannot reject a pre-reset ARM with delayed writes alone, so native's
independent ARM check is mandatory. Keep all three latest consecutive verified
sessions ending at expected session; no arbitrary replacement dates or fourth day.

### 9.4 Native callable seam and bounded followups

Existing `with_native_checked_paper_action(pool, guard, consumer)` remains the only
consuming checker seam. Replace its Vec<Value> native session facts with a private
`CheckedEnableSession` struct, constructed only at verify_database_readback_mode's
successful require_run join, containing typed date/run/job/attempt IDs, slot and
reservation IDs, admission/source/root digests, full checked cohort digest, and
`CheckedArmOrigin {boot_id,generation,nonce,invocation_id,arm_intent_sha256,
armed_sha256,arm_intent_observed,armed_observed}`. Source parent identity and the
actual chain remain held by the borrowed source reader, not copied into a report.
Expose crate-private read-only accessors through CheckedPaperAction; no public
constructor/Deserialize/Clone permit or conversion from report/diagnostic strings.
Serialize only the existing five-key session SQL projection at the consuming call.

Add private `read_exact_result(&mut PgConnection, &ExactRequest)` and
`read_reset_boundary(&mut PgConnection, account_id, expected_epoch)` adapters using
only §9.1. Add `CheckedResetAnchor` with private protected-reader constructor and
retained FD/name identity; `check_enable_reset(&CheckedPaperAction,
&ResetBoundary, &[CheckedResetAnchor])` borrows rather than exports authority.
Exact internal signatures (all crate-private; no serializable checked value):

```rust
async fn read_exact_result(connection: &mut sqlx::PgConnection,
    request: &ExactRequest) -> Result<ExactReadback, ErrorCode>;
async fn read_reset_boundary(connection: &mut sqlx::PgConnection,
    account_id: uuid::Uuid, expected_epoch: i64) -> Result<ResetBoundary, ErrorCode>;
fn check_enable_reset(action: &CheckedPaperAction<'_>, reset: &ResetBoundary,
    anchors: &[CheckedResetAnchor]) -> Result<(), ErrorCode>;
```

`CheckedEnableSession.session_date` is chrono::NaiveDate; run/job/attempt/reservation
and ARM boot/generation/nonce/invocation identities are uuid::Uuid. Slot/cohort/event/
admission digests are validated String values (exact sha256 grammar), never parsed
for causal order. Both observed fields use a private ClockTriple with three i64
nonnegative nanosecond fields. `ExactRequest`/`ExactReadback` are untrusted closed-wire
types, not authorization; their constructors perform §9.1 validation. `ResetBoundary`
has UUID account/event fields, a closed three-kind enum, i64 epoch/microseconds and
validated digest. CheckedResetAnchor owns retained protected descriptors and the
verified selection; neither its constructor nor the checked session constructor is
exposed outside its validating module. No new public library API is required.

Keep source, Current, rights transaction, original inputs, anchors and release lock
alive through SQL wait, immediately before COMMIT, immediately after COMMIT and
checker cleanup. Before SQL read the boundary; after transition returns with locks
held, reread RESET at resulting epoch on the same connection and compare **all reset
fields**, then recheck source/anchor identities and live clocks before COMMIT.
A concurrent reset before lock acquisition yields stale epoch; after acquisition it
waits until COMMIT. Any changed reset/cohort/anchor invalidates the action. Cleanup
failure after COMMIT follows §9.2, never a successful permit result.

Add two deliberately separate one-argument native verbs, `readback` and
`observe-reset`. Keep all eight existing verbs/results unchanged. New verbs return
`owner-beta-paper-ops-query-result-v1` (same closed fields/outcomes/exit meanings as
ops-result-v1, but verb only readback/observe-reset). `readback` retains the original
request.json/evidence.json and reads exact SQL request bytes from the additional
fixed root0400 `/run/owner-beta-paper-ops/sql-request.json`; verifies their correlation,
including exact generated ENABLE payload preserved before sending, then uses only
migration_owner. It works with Current INVALID, Daily OFF, no worker credential,
expired eligibility and later epochs. Its output artifact is exactReadback; COMMITTED
maps REPLAY, ABSENT/UNPROVEN map COMMIT_UNKNOWN, malformed/conflict maps BLOCKED.
It must not call begin/configure/reconcile/transition or re-run eligibility.
Transport/reader timeout remains COMMIT_UNKNOWN. For readback, generation is the
original intent generation, event_id is the recovered original event/observation
or null, control_epoch is the readback current_epoch, artifact_sha256 hashes the
complete persisted exactReadback. Known malformed/conflict results have no artifact
unless a complete valid readback exists; fixed error codes remain the original set.

`observe-reset` uses an INSPECT intent and existing process-binding evidence plus
capability/ELF/protected Current inputs; it checks account/expected epoch, selected
next-epoch binding and release/cohort while OFF/RESTORED_OFF, issues RESET, samples
local boot/clocks and emits resetAnchor to the durable output bind. It uses no worker
DB role and no full eligibility checker. After query/clock sampling recheck protected
identities and repeat RESET at the same expected epoch; changed state/identity fails.
Root publishes that original anchor under the held release lock before observation
sessions. ENABLE receives a fixed read-only mount
`/run/owner-beta-paper-ops/reset-anchors/`, containing only1..3 selected local anchor
files named by canonical hash, one per distinct ARM boot among the three sessions.
No caller path/list/glob selector; bounded enumeration, exact names and originals.
Publication/mount staging belongs to P4 integration, not this P0-B implementation.
Successful observe-reset returns OK/NONE, intent generation, reset event_id, observed
current epoch and the complete persisted anchor digest; failure emits no anchor.

P1 owns0068 up/down, its migration-contract tests and audit projection only: add
receipt columns/read function/atomic writers and full-preimage replay, reset checks
in the ENABLE wrapper, grants/RLS/timeouts/down guards. Stop delegating **new** ENABLE
straight to legacy before these checks; reuse its validation after the added lock/
reset checks. Do not edit0067 or weaken its joins. Other gate/recovery/chain fixes
remain separate. P2 owns operations.rs/wire.rs/protected.rs, evidence.rs/financial.rs
and their focused tests only: typed facts, private adapters/anchor readers, durable
original-request retention and new read-only verbs. Remove the explicit false
ENABLE gate only after these two implementations and their focused hosted tests
pass; P4 publication availability and unrelated gates remain independent blockers.
No P1/P2 native sub-delegation. No P3 ops/Docker/Compose changes in these packages.

### 9.5 Required verification matrix (not executed by this amendment)

| Case | Required result / owner |
| --- | --- |
| Initial configure, ordinary OFF, destination RESTORED_OFF | Exact unique epoch reset; three genuine post-anchor sessions pass all existing checks. SQL+native. |
| Release/semantic change while already OFF | New begin/OFF before anchor/ARMs; old manifest/cohort anchor fails. Native; controller staging later. |
| Pre-reset ARM with all DB writes post-reset | SQL timestamps may pass; typed native ARM check must fail. Native. |
| Post-reset ARM with old/equal/future slot/reservation/run/link time | SQL rejects23514 independently. SQL. |
| Run/date/job/attempt/reservation/root/admission/cohort swaps | Genuine join or matching three-session projection rejects; no reconstruction from report. Both. |
| Reset timestamp rollback/tie; duplicate reset epoch | Epoch selects latest despite wall order; equal max epoch fails55000, never UUID tie-break. SQL. |
| Reset racing ENABLE lock wait; stale expected epoch |40001 or changed-boundary rejection; no new ENABLE event. Both. |
| Different ARM/current boots; restart without reset | Old local anchors keep valid past sessions; new boot anchor precedes new ARM; cross-boot clock arithmetic rejected. Native. |
| Clock rollback, missing/backdated/imported/replaced anchor | Reject; no inferred history or age-based bypass. Native. |
| Direct wrong role, SET ROLE, superuser; unset/empty/foreign actor |42501 for principal violations; allowed direct login restores actor on every success/error/absence path. SQL. |
| Each CONFIGURE/BEGIN/RECONCILE/transition variant, exact/changed key bytes | Original exact result recovered; changed args including epoch conflict23505. Same generation changed intent conflicts. Both. |
| COMMIT before timeout / after timeout; cancellation before COMMIT | Durable event/receipt atomically visible or UNKNOWN; definite rollback acknowledgment distinguished. Both. |
| Cleanup fails after committed ENABLE | Readback COMMITTED plus preserved cleanup error, startup forbidden; explicit drain handoff. Native. |
| ABSENT with original backend still live or prepared work | COMMIT_UNKNOWN; no mutating retry/new key. Native and SQL session test. |
| Current INVALID, Daily OFF, expired inputs, later control epochs | Exact readback still works without worker checker; historical result separate from current state. Both. |
| No-write reads/replays | Run reader in READ ONLY transaction; compare complete relevant row/receipt/outbox sets before/after repeated reads and exact mutation replays. SQL. |
| Canonical/hash/malformed/oversized/deep/duplicate/fractional-time cases | Reject invalid wires; preserve valid old timestamp bytes and original results exactly. Both. |
| Missing receipt on historical row; archive/down | UNPROVEN without history writes; roots include columns; nonempty down refuses and empty down restores0067. SQL. |

Use hosted compiler/PG jobs for runtime evidence. Small stdlib canonical/schema
checks prove syntax and selected examples only. Source inspection cannot establish
actual role/RLS, timeout/commit behavior, genuine ARM capture or runtime acceptance.
