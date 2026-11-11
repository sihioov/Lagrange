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

This maintenance stage chain applies to STOP/BACKUP/RESTORE generations. CONFIGURE
with a null account retains its original intent, native dispatch/cleanup custody and
exact SQL result; it does not append an account-bearing maintenance stage or invent
an account UUID. ENABLE/RECOVERY likewise retain their original root intent/custody
without fabricating a DRAINING begin or maintenance chain. A query or replay never
creates a completed stage or authorizes startup. This clarifies the existing verb
mapping in section5; the frozen intent/stage schemas and SQL arguments are unchanged.

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
Existing producer `artifact.json` uses compact sorted JSON plus one final LF;
COMPLETE hashes those original bytes, including LF. Operation-wire no-LF rules
do not rewrite this pre-existing producer format.
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

### Archive intake DAC and syscall boundary (2026-11-11)

**COORDINATOR SOURCE REPAIR; RUNTIME ACCEPTANCE PENDING.** The codec preserves
UID10001 files0600 and directories0700. UID0 with all capabilities dropped cannot
read those originals through supplementary groups alone. Preserve their metadata.
Only a registered package's actual destination RESTORE BEGIN, with its matching
active action custody, may run with CapDrop ALL and exactly DAC_READ_SEARCH added.
Check that owner relationship, release/image/journal originals and exact runtime
argv/inspection policy together. Source readback, destination reconcile/restore-off/
readback and every ordinary operation retain no added capability.

Before opening protected inputs, the native entry installs no-new-privs and an
additive process-wide seccomp filter with TSYNC; only an exact zero return succeeds.
There is no retry without TSYNC, unfiltered fallback, replacement Docker profile
or filter removal. Count installation in the original15-second operation clock.
The private successful-install guard stays borrowed through archive continuation,
dispatch, final original revalidation and cleanup. On supported Linux/x86-64,
reject other architectures, x32 syscalls and raw numbers512–547; deny handle access,
namespace-changing clone flags, unshare/setns and mount APIs. Return ENOSYS for
clone3 and verify ordinary thread compatibility. Docker's existing filter remains.

The guard observes the calling thread's effective/permitted/inheritable/ambient
capabilities, rejects extra authority before input access, and checks the exact
empty-versus-DAC_READ_SEARCH set against loaded bound purpose/action before use.
It revalidates that snapshot through archive dispatch. It does not establish the
bounding set, other threads' capabilities or host mount/network/namespace identity;
actual runtime inspection and focused kernel/container checks retain that burden.
No DAC_OVERRIDE, SETUID, SYS_ADMIN, privileged mode, host namespace, Docker socket,
serving-data permission change or additional writable bind is introduced.

An installation failure precedes dispatch inspection. It cannot prove rollback of
an existing request. The host retains the original dispatch and treats such a
BLOCKED response as unresolved, requiring exact readback. Full acceptance still
requires actual frozen-original reads, zero-cap refusal, forbidden writes/handle/
namespace/mount attempts, cross-thread filtering, owner/capability substitutions,
and source replacement/cleanup uncertainty checks. Source and mocked tests alone
do not accept this boundary or authorize production activation.

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

The existing read-only RESET request family can also use `readback` with an
artifact-free INSPECT original: its non-null account and expected epoch must match
exactly, and its frozen args still contain no request key. A fully validated
RESET-status artifact returns OK/NONE, with no mutation event or committed-result
claim. Mutation COMMITTED/ABSENT/UNPROVEN behavior above is unchanged. This query
needs neither Current nor a worker credential and cannot authorize dispatch.

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

#### Verification venue allocation (2026-11-11)

**COORDINATOR SOURCE CLARIFICATION; REQUIRED EVIDENCE NOT YET ACCEPTED.** The
private-data rule in the approved-base runbook prohibits uploading private base,
Raw bodies, credentials or raw logs to hosted CI. Preserve every row above and
the §9.4 code-readiness gate. Existing hosted compiler/PG/OS jobs own the checks
that use public or synthetic component inputs. Rows requiring the genuine checked
Source join, its retained private historical originals, post-anchor sessions or
ARM/reset substitutions additionally require same-candidate private execution on
the protected verification host. That evidence is mandatory **before** removing
the false ENABLE code gate; it is not deferred to later P4 activation.

Record each venue, exact candidate and compiled revisions, actual test names,
original input identities, observed results and owned cleanup separately. Only
sanitized hashes/counts/status may enter the shared receipt. Source markers,
fabricated checked facts, mocked joins and component passes cannot satisfy a
genuine row. A libtest consuming the actual checker and originals may establish
implementation behavior, but its dispatch/ELF/SQL receipts remain test-origin
and cannot supply official archive history. Keep required hosted checks intact.
This allocation adds no runner, SSH route, privilege, provider request or source
activation permission, and does not renew expired host access.

Changing the readiness constant creates another candidate: rebuild its official
images and rerun required checks. Earlier producer receipts and Source cohorts
must not be relabeled. P4 publication/mount gates, actual release/archive runtime,
G4, real pre/post-three-session observation and authenticated Owner acceptance
remain independent. No completion follows from this venue clarification.

## 10. P0-C: immutable operation and original action binding (2026-11-06)

**COORDINATOR-ADOPTED SOURCE CONTRACT (2026-11-06), NOT IMPLEMENTED OR RUNTIME ACCEPTED.** Context
`ae04d75d41904a0825c6df88a7f0cd97864aad4b`; R5-04/WP5. This section supersedes only
ambiguous root/action correlations in §§2–6 and9. It changes no financial contract,
0067 contract, SQL signature, SQL receipt preimage, archive definition or public
result. Production remains CLOSED, `ENABLE_SQL_RESET_AND_READBACK_READY=false`;
maintenance/recovery stay dormant until their real hosted and P4 proofs. No parser,
fixture, hash or report grants authority. No new role, CLI selector or automatic
activation exists. Earlier acceptance evidence is preserved.

### 10.1 Three originals, four key domains

Use **R** for immutable root `intent.json`, **A** for a new closed `actionBinding`
original, **E** for the selected original evidence bytes, and **Q** for original
`exactRequest` SQL bytes. `C(x)` is §2 canonical ASCII JSON, `H(x)` is
`sha256:` + SHA256(exact bytes). R retains the old intent schema and meanings;
it is never rewritten into the next action. A is NOT another old-schema intent.
The additional definitions are referenced by fragment only; the old schema's
`oneOf`, every old definition and archive schema remain identical after JSON parsing.

| Domain | Frozen identity / equality |
| --- | --- |
| Caller key K | `(installation_id, account_id including null, K)` in the private caller index; exact command replay returns the original generation. It is not passed to maintenance SQL. |
| Root operation | One generated G, one R. Maintenance R.request_key is `G:begin`; single-action CONFIGURE keeps K, ENABLE uses `G:enable`, RECOVERY uses `G:recover`. |
| Native action | `(G, sequence)` and the full canonical A; sequence starts0 and has no gaps. No generated retry identity. A.request_key is the SQL key, not a process PID/UUID. |
| SQL | Existing §9 namespaces, actual A.request_key, exact Q and hash. BEGIN additionally binds G and R bytes. No sidecar field enters SQL. |

Before allocating G, under the release lock, canonicalize the old intent properties
EXCEPT `generation` and `request_key` as the **command projection**. Its complete
bytes (not just a hash) include account, purpose, scope, observed epoch/floor,
predecessor, selected artifact, resume target and release. Store them as the string
`command_canonical` in new `callerBinding`, alongside installation_id, account_id,
caller_key, generation and root_intent_sha256. Projecting R must reproduce that
string exactly. A repeated K with identical command bytes resolves to the old G/R,
including after completion; changed bytes conflict. Do not re-observe epoch and
silently treat a repeated K as a fresh command. New explicit K permits a new G only
through existing state/authorization checks. CONFIGURE null is its own caller
namespace; never migrate that index to the newly assigned account.

For STOP/BACKUP, R.artifact_sha256 is null. RESTORE binds the selected archive hash,
not a future restore receipt; R.predecessor_generation is the imported source
operation required by SQL's latest-generation rule. Ordinary maintenance predecessor
is the latest retained SQL operation, or null only if none. CONFIGURE/ENABLE/RECOVERY
bind their selected configure/process/recovery artifact once. Their generation has
one action, sequence0, no maintenance stage. INSPECT/observe-reset retain the existing
read-only path; no fabricated operation or action chain is needed.

New `actionBinding` exact fields:
`{schema:"owner-beta-paper-action-binding-v1",installation_id,generation,
root_intent_sha256,sequence,previous_binding_sha256,verb,request_key,
expected_epoch,artifact_sha256,stage_sha256}`. Digests/nulls, UUIDs, keys and counters
use §2. Verbs are only configure/enable/begin/reconcile/off/restore-off/recover.
The last three identity fields are NOT a readiness assertion. A max65,536 bytes,
depth16. Caller binding max65,536/depth16, including embedded command. Each A repeats
R installation/G, binds H(R), names its immediate predecessor H(A) (null only at0),
and is immutable. No unrelated operation, skipped/forked sequence or changed action
under an occupied identity is accepted. Reconcile ordinal N is 1-based, minimal
decimal (no leading zeros), at most18 digits; other action keys are as above or
`G:off` / `G:restore-off`. Existing original legacy key bytes are never normalized.

### 10.2 Acyclic evidence and epochs; exact unchanged SQL mapping

Let e=R.expected_epoch, f=max(e,R.local_epoch_floor)+1 from the committed BEGIN
result, and z=f+1 from OFF/RESTORED_OFF. Checked integer arithmetic is mandatory.
Neither a proposed f nor an epoch-floor file proves that BEGIN committed.

| Action | A.expected_epoch / key | A.artifact / stage | Unchanged SQL arguments |
| --- | --- | --- | --- |
| begin | e / G:begin | null / INTENT | begin(e, parse_json(C(R)), G:begin) |
| reconcile N | f / G:reconcile:N | H(inventory N) / STOPPED or latest RECONCILED | reconcile(G,f,H(inventory N),G:reconcile:N) |
| off | f / G:off | H(drain) / latest RECONCILED | transition(f,'OFF',parse_json(C(drain)),G:off) |
| restore-off | f / G:restore-off | H(restore) / RESTORE_VERIFIED after latest RECONCILED | transition(f,'RESTORED_OFF',parse_json(C(restore)),G:restore-off) |
| configure | 0 / K | H(configure artifact) / null | configure(strategy UUID,name,contract jsonb,SQL NULL,K) |
| enable | R.expected_epoch / G:enable | H(process) / null | transition(A.expected_epoch,'ENABLE',original checked evidence jsonb,G:enable) |
| recover | R.expected_epoch / G:recover | H(recovery v2) / null | transition(A.expected_epoch,'RECOVERY',original decision jsonb,G:recover) |

For every row construct Q exactly as §9.1. BEGIN's embedded intent is **R**, not A,
and Q.expected_epoch=e. RECONCILE and OFF/RESTORED_OFF Q.expected_epoch=f. Every Q
key equals A.request_key. Account comes from R, including CONFIGURE JSON null in Q;
its process_binding JSON null maps to SQL NULL, never jsonb 'null'. ENABLE Q contains
the original generated checked payload, not just E; all fractional timestamp strings
survive. RECOVERY Q.evidence_canonical is C(E). No A/caller metadata is added to Q,
legacy payloads, result bytes, database receipt columns or their hash preimages.

Drain/restore's existing `intent_sha256` is H(R), **not H(A)**; their request_key
matches A. Their existing `expected_epoch=e` and `resulting_epoch=f` describe the
operation's fence, not the OFF SQL argument/result. SQL0068:1203–1206 already compares
these fields to operations.expected_epoch/fence_epoch. Their inventory and summary
are at f; the immutable OFF event and result control_epoch are z. The stage after
commit is at z. This is the explicit correction to native `validate_identity`, which
currently conflates these epochs and hashes the invocation intent. It cannot require
`resulting_epoch=e+1` for RESTORE when the local floor is ahead.

Stage.intent_sha256 always equals H(R). A.stage_sha256 names the sealed stage **before**
this action; drain/restore.stage_sha256 equals that same digest. Stages keep their old
artifact meanings: INTENT null (RESTORE may bind archive); FENCED BEGIN result;
STOPPED inventory; RECONCILED SQL summary; RESTORE_VERIFIED verified archive/audit
root; OFF_COMMITTED/RESTORED_COMMITTED selected drain/restore receipt. Root/action
binding does not repurpose a stage's artifact field into a dispatch pointer.
There is no cycle: R → prior stage → E → A → native Q/index → subsequent stage
(arrows here mean publication order, not embedded references). R never hashes A or
a future E; receipt hashes R and prior stage; A hashes receipt and prior stage.

RESTORE preserves source and destination separately. Existing SQL0068:1225–1227
checks `source_epoch` against the imported OFF **payload.resulting_epoch**, hence
that field is the source fence epoch. The archived OFF event/control and archive
control_epoch are source fence+1 and remain unchanged. Destination floor must be at
least the archive's actual control_epoch, not merely source_epoch. Native additionally
checks the imported source root, OFF event/result and archive cross-links, including
that +1 relation; it cannot infer actual source epoch from the misleading field name.
Source installation must differ from destination as current SQL requires. In-place
restore must use the existing isolated destination route; this amendment does not
make same-installation RESTORED_OFF pass. Destination R binds the archive, source G
as predecessor, actual restored DB epoch e and protected floor. No imported caller
index, action chain, anchor or stage becomes local authority. Imported SQL receipts
remain exact historical rows, never rewritten to destination IDs.

### 10.3 Fixed storage, custody and crash ordering

All paths below are relative to `.owner-beta-paper/`, with existing protected ancestor,
root ownership, no ACL/symlink, regular nlink1, FD/name/inode and release-lock checks.
Directories0700, sealed files0400. Names are derived internally, never caller paths.

- `callers/<hex H(C({installation_id,account_id,caller_key}))>.json`: callerBinding.
- `operations/G/intent.json`: original R, unchanged.
- `operations/G/actions/<sequence padded to at least8 decimal digits>/binding.json`: A.
- In that action directory `evidence.json` is E when A has a digest; no file otherwise.
  `output/` is the durable, unique native output bind, with existing content-addressed
  originals and new fixed `dispatch.json`. `import.json` is the controller's sealed
  dispatch-index copy. Retain the actual directory FD throughout helper exit/reap.
- Native gets existing `request.json` = R and `evidence.json` = E plus fixed additional
  `/run/owner-beta-paper-ops/action-binding.json` = A, read-only. Live maintenance also
  reopens original operation stages/evidence through fixed read-only
  `/run/owner-beta-paper-ops/operation/`. This is a fixed mount, not a path in JSON.
  The stage reader recognizes only these fixed reserved subdirectories and continues
  rejecting unknown entries; it does not mistake actions/output for stage files.
  No new env/CLI selector, authority flag or arbitrary SQL/list is accepted.

Under the held release lock: (1) resolve caller index and DB observations; create/fsync
R and root directory; (2) publish/fsync callerBinding before any helper dispatch;
(3) validate/retain the current stage and evidence, publish/fsync E then A and every
new directory; (4) launch exactly one helper with retained mounts/output; (5) native
independently opens originals, validates bindings and live prerequisites, builds Q,
fsyncs each original content-addressed file, then no-replace publishes/fsyncs
`output/dispatch.json` **before any mutating SQL submission**. This dispatch is also
stored content-addressed. It uses additive `nativeDispatchV2`:
`{schema:"owner-beta-paper-native-dispatch-v2",root_intent_canonical,
root_intent_sha256,action_canonical,action_sha256,artifact_canonical,
artifact_sha256,sql_request_canonical,sql_request_sha256}`. Embedded strings retain
exact canonical bytes; artifact pair is null together. Max2MiB/depth16, Q max1MiB
and existing embedded variant limits. No output JSON is itself an authorization.
(6) execute only the fixed SQL operation; (7) persist original result/cleanup evidence;
(8) controller imports dispatch, proves committed result independently and only then
appends the appropriate stage and advances floor. Never hold SQL while waiting for
controller import, process termination or OS locks. Controller import is not a
permission acknowledgment and requires no pipe handshake before SQL.

Step5 must occur even for CONFIGURE/null and a generated ENABLE Q. Publication
failure means no SQL submission. Native retains R/A/E/Q, all original descriptors,
ancestors/output and live checked resources through waits, pre/post COMMIT and cleanup;
rechecks bytes plus named identities, not just an in-memory hash. Controller retains
lock and action/output descriptors through import and child exit/reap. No use after
close. An action with published A cannot be replaced after helper failure. Each
output belongs to only that action; dispatch.json conflicts on different bytes.
If dispatch.json already exists with identical bytes, native does not submit the
mutation again: return unresolved custody for the separate readback verb. Repeating
prepare_action with identical A/E only reopens the retained action; repeating it
with changed bytes conflicts. Importing identical dispatch/result is no-write replay.
A readback alone appends no stage. Any repair of a missing local completion stage
belongs to the separately resumed live controller after independent committed-result
and current resource/chain checks; it must not infer stopped state from COMMITTED.

Crashes between1–2 leave an undispatched root: under lock reopen that exact G/R and
complete the same caller mapping, or block ambiguous roots; never allocate a second
G for the same command/K by guessing. Crash after3 records a pending invocation even
without native output. Recover the known action's fixed dispatch.json, not a scan for
JSON shapes or newest hash. A valid index may be imported after crash before result
or before import.json; identical import is no-write replay. A torn/missing dispatch,
missing referenced original, changed inode/bytes or helper possibly alive remains
incomplete/unknown; absence is not permission to dispatch again. Exact original
backend disappearance, prepared-work exclusion and final-fence protocol remain §3.
A surviving helper can still commit after timeout; do not append next action while
unresolved. Known no-submit failure is distinct from lost acknowledgment, and does
not authorize economic retry. No automatic new key or new generation.

After successful reconcile, refresh appends a new A and new SQL observation with
new reconcile ordinal and new RECONCILED stage, keeping earlier evidence. OFF can
consume only the latest complete original observation and stage. Once OFF A is
sealed, its evidence/key cannot be refreshed in place; resolve it first. If it was
not committed and evidence is unusable, a linked new maintenance generation requires
the existing explicit restart/fence procedure. Reboot needs a new generation for
live work, but does not invalidate historical exact readback.

### 10.4 Offline readback and compatibility

Readback loads original R, A, E, Q from private custody, with the same fixed mounts
plus `sql-request.json`. Verify their closed wires/full byte correlations, root and
action installation/G, Q kind/key/account/epoch and E hash. Use the original action
verb to select parsing, not 'readback' as a new intent. It needs **no current stage
chain, Current, Daily, worker secret, provenance, capability, reset anchor, fresh
clocks or current epoch equality**. A.stage_sha256 remains the original digest;
readback does not traverse or regenerate the old stage. Protected original R/A/E/Q
and native custody are required, not replaceable by a public report. The local
caller index and latest operation state are not current authorization gates on this
read-only path. Missing originals fail closed; no reconstruction from SQL hashes.

Run only §9's exact reader under migration_owner/read-only RR with existing limits;
no begin/reconcile/transition/RESET/inspect fallback. COMMITTED recovers original
result/id/event_epoch with separate current_epoch/state; ABSENT/UNPROVEN remain
COMMIT_UNKNOWN. No replay proof authorizes startup or clears cleanup failure. Lost
COMMIT acknowledgment, expired evidence, later OFF and invalid Current use identical
Q bytes. Repeated readbacks use fresh DB snapshots; no same-snapshot polling fiction.

Legacy without action-binding.json stays on its existing exact parser/custody-v1
path for supported configure/inspect/enable/begin/readback (ENABLE gate remains
false). Missing A is not a fallback for a bound action: the controller knows which
original format it sealed and refuses stripping/downgrade. No synthesized A, index
v2 or new receipt is attached to legacy SQL history. Existing standalone original
begin reads remain possible even though it has no per-action continuation; new
maintenance uses this bound protocol. Legacy dormant reconcile/off/restore/recover
must not bypass its closure. Existing schema shapes, original bytes, keys, hashes,
public result/exit meanings and exact-reader behavior stay unchanged. RESTORE begin
may bind an archive in R but A.artifact is null; native must stop trying to parse
that archive as its invocation E. Live RESTORE requires separate protected archive
verification through the original operation store before dispatch.

### 10.5 Worked traces and disjoint implementation handoff

The shared `tests/fixtures/owner-beta-paper-operations/action-bindings-v1.json`
contains exact canonical originals and digests. Its outer label is
`SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN`, never a production input field. Traces:

- STOP: R(e0,floor0,key G:begin,artifact null); stage0 INTENT. A0 begin/e0/Q embeds R
  → committed fence1 → FENCED; inventory1 → STOPPED. A1 reconcile:1/e1 → summary1
  → RECONCILED. A2 reconcile:2/e1 with refreshed inventory2 → summary2 → new
  RECONCILED. A3 off/e1 uses drain(expected0,resulting1,H(R),latest stage/summary2)
  → original OFF event/result2 → OFF_COMMITTED2. No mutation of A1 or summary1.
- BACKUP: same0→1→2 under COMBINED_BACKUP, then SNAPSHOT at2 binding combined archive;
  no fake RESTORED_OFF in the source. Every required writer class still needs proof.
- RESTORE: source BACKUP Gs/Is has pre0,fence1,OFF event2/archive control2. Destination
  Id differs; Gd differs, R.predecessor=Gs, R.artifact=archive hash, restored DB e2,
  local floor9. begin(2,R,Gd:begin) → fence10; reconcile( Gd,10,inventory,key ) →
  RECONCILED10; RESTORE_VERIFIED10; restore-off(10,E,Gd:restore-off), E has
  expected2,resulting10,source_epoch1,destination_floor9,source Gs/Is,equal source/
  restored audit hashes → RESTORED_OFF11. Historical source OFF stays at2; floor
  stays at least11, never regresses to archive2 or mistaken receipt fence1.
- CONFIGURE: R.account=null/e0, A0 configure/e0, Q.account=null and
  Q.process_binding=null → SQL NULL; result creates account at0. R/A/Q keep null;
  no account-bearing INTENT/FENCED stage is invented.
- ENABLE/RECOVERY: separate single-action Gs with no begin/stages. ENABLE e2→3,
  process at3 and exact generated Q retained; RECOVERY at active3 preserves lineage1
  and original receipt/T/E identities. It does not increment active epoch or create
  a DRAINING chain. Their existing eligibility/rights/grant checks remain mandatory.
- Unknown commit: retain original ENABLE Q at e2. After helper crash, expiry,
  Current INVALID/Daily OFF and later control OFF5, the same read returns original
  ENABLE event_epoch3/result3 with current_epoch5; cleanup remains unresolved.
  ABSENT with original backend alive remains unknown; restart imports the same
  dispatch bytes and never reissues ENABLE.
- Caller replay: same installation/account/K and command projection → same G/R;
  changed release, artifact, floor, purpose, predecessor or epoch under K conflicts.
  A new K is a separate request, never a workaround for unresolved prior work.

**Controller/journal package (Python only):** retain Journal.create/open compatibility;
add private `Journal.bind_caller(caller_key, root_intent_canonical) -> Operation`
which atomically checks/publishes callerBinding as above (caller lookup before G
allocation belongs to controller). Add `Operation.prepare_action(action_canonical,
artifact_canonical_or_none) -> ActionCustody` and `Operation.open_action(sequence)`;
construct only through protected private factories. Add read-only
`Journal.open_custody(generation, sequence) -> Custody` for restart: it checks named
protected R/A/E/Q/index originals and retained ancestors without calling the live
Operation._check/revalidate stage-chain path. Live prepare/import retains the full
chain requirement. The custody opener cannot append a stage or dispatch a mutation.
`ActionCustody` owns/borrows
live operation, action and output descriptors, exposes `stage_inputs(destination_fd)`
and `import_dispatch(output) -> Custody`; import reads only fixed dispatch.json and
checks v2 against that retained action/root. Replace operation-wide `_dispatch_hash`
with per-action identity. `_validate_exact_request` takes the retained root + action
context, never fabricates a mutable synthetic intent. Keep old import_native_dispatch
for v1 only; v2 has no caller hash-only selection. Custody.stage_readback writes R/A/E/Q
exactly; cleanup import keys by action and SQL digest, preserving requires_drain.
Implement controller stage append only after independently verified SQL result, with
native chain proof separate. Both bound consumers must validate the explicit purpose/
scope pairs STOP/PAPER_STOP, BACKUP/COMBINED_BACKUP, RESTORE/COMBINED_BACKUP; the
current RESTORE-iff-COMBINED comparison wrongly rejects BACKUP. The bound journal
successor check admits destination RECONCILED → RESTORE_VERIFIED →
RESTORED_COMMITTED without source OFF_COMMITTED/SNAPSHOT stages. Source BACKUP
keeps RECONCILED → OFF_COMMITTED → SNAPSHOT; STOP ends at OFF_COMMITTED.
No null-account or single-action operation gets a maintenance stage. Tests: actual root/no-ACL/name/FD replacement, caller
replay/conflict, cross-action import, crash at each fsync boundary, missing index and
late commit. Python owns its focused verification, no Rust/SQL edits.

**Native package (Rust operations.rs/wire.rs/protected.rs and focused tests only):**
add private closed `ActionBinding` (untrusted) and private retained `BoundOriginals`
factory; split `ProtectedInputs` opening into legacy, bound-live and bound-readback.
Add `parse_exact_bound(bytes, &RootIntent, &ActionBinding)` for Q: the current
`ExactRequest::from_bytes`/`validate_transition` also hardcodes fence=e+1 and cannot
be reused unchanged for a floor-ahead RESTORE, even on readback. Its bounded parser
still validates all original closed SQL variants; correlation supplies max(e,floor)+1.
`parse_root_intent` validates old root shape/semantics without requiring later verb
keys; `parse_action(bytes, &RootIntent)` validates bindings. Private
`correlate_bound(&RootIntent,&ActionBinding,Option<&Artifact>,&ExactRequest)` applies
§10.2; `parse_bound_artifact` uses root for fence/history and action for SQL key/current
epoch/E hash. Keep old parsers unmodified in meaning for legacy. `BoundOriginals`
retains descriptors; no public Deserialize/Clone checked permit, report conversion or
serialized checked value. Live maintenance factory independently reopens the unique
original chain and checks DB/result and protected inventory/restore provenance;
readback factory deliberately needs no live chain. `persist_bound_dispatch` implements
step5 fixed index before SQL, retaining originals through cleanup. Test shared vectors,
legacy compatibility, null SQL binding, restore floor, negative substitutions and
readback after expiry/current invalid; actual root OS tests and hosted SQL
lost/late-COMMIT/cancellation before enabling any path. Keep all current false gates.
No SQL/preimage, source authority, public result or controller edits in this package.

Coordinator integrates these disjoint consumers, updates roadmap/progress, registers
hosted proof and has P3/P4 stage the fixed mounts and refreshed schema/capability hashes
in a NEW immutable release; never modify an installed release. Runtime mount/structure
changes later require their evidence-backed diagrams. Schema/vector checks alone do
not establish runtime/P4/R5/Owner acceptance. No irreducible finance or permission
choice was required by this amendment; implementation and real proofs remain open.
