# R2 I2 — migration 0060 implementation boundary

Status: **implementation contract fixed; SQL and tests not yet implemented or executed**.
Coordinator scope WP-4/I2-D, Wave36, roadmap R2-04/R2-09, 2026-10-15.
Baseline: `c2c419e`; last executed candidate:
`57ca4d1390b025e7efbe9ac452c614a8b5618889`.
The [bounded I0d acceptance](2026-10-15-r2-bootstrap-negative-acceptance.md)
establishes the bootstrap prerequisite only. It does not execute 0060 or prove
runtime filesystem access, daily input admission, deployment or Owner acceptance.

The adopted [D1](2026-10-14-r2-daily-input-design.md), its
[D5 overrides](2026-10-14-r2-daily-input-design-amendments.md), the
[D6 disposition](2026-10-14-r2-daily-input-amendments-review.md), and
[D4 wire contract](../contracts/owner-beta-daily-etf11-input-v1.md) remain intact.
D5 wins over conflicting D1 proposals. This document fixes the remaining SQL
transport, receipt and harness interfaces; it neither changes D4 bytes nor selects
a production policy, Owner, entitlement, grant, request budget or activation date.

## 1. Concrete integration gaps and decisions

| ID | Observed source/contract gap | Fixed implementation decision |
| --- | --- | --- |
| I2-D01 | D4 admission has no self hash, range, Owner or DB timestamp; D5 A6 accepts one `p_admission jsonb`. Passing the wire object alone cannot provide the external hash and manifest range. | Use the exact SQL transport envelope in §3. D4 admission bytes and hash preimage are unchanged. Never hash `jsonb::text` as a D4 artifact. |
| I2-D02 | D1 polling returns boolean and completion has no receipt ID; D5 A6 requires DB-generated receipts and once-only completion. A2's calendar function has no receipt argument. | Freeze receipt-bearing signatures in §4. Calendar projection precedes admission; it can run with daily scheduling OFF. Completion and durable conflict evidence share one transaction. |
| I2-D03 | Committing the short slot transaction releases its transaction advisory lock. A later scan can outlive the 60-second start interval. | Bind mutable poll state to its latest receipt ID. An older scan cannot overwrite current freshness or complete a new admission after supersession. Do not hold a DB transaction across file IO or describe a transaction lock as a process-lifetime mutex. |
| I2-D04 | D1 requires exact policy/base binding but control lists only a policy hash. SQL cannot reopen the protected policy file. | Store its validated D4 projection as `policy_json` in release-authority-only control. The installer is responsible for matching canonical file bytes/hash; SQL compares base, instruments and versions to this protected projection. Every projection change increments activation_epoch. |
| I2-D05 | D5 A4 requires bounded queue backoff, but only lease_seconds is bound in control. The runner also reads `PRICE_BETA_BACKOFF_SECS`. | Add positive `backoff_seconds bigint` to control, bind it to the runner setting, and increment activation_epoch on change. With fixed daily max_attempts=3, retry delay is base times 1 or 2. Do not infer a one-hour absolute cap from the existing queue comment. |
| I2-D06 | `migration_contract.rs` embeds all files under migrations; its supervisor-derived effective-role pools are not direct-login evidence. | Publish the complete UP/DOWN pair only together with test/harness wiring. New daily role assertions require direct logins with current_user=session_user; existing effective-role helpers remain historical tests only. |

Sources: [D4 admission](../contracts/owner-beta-daily-etf11-input-v1.md#L446),
[D5 receipts](2026-10-14-r2-daily-input-design-amendments.md#L587),
[embedded migrator](../../tests/integration/migration-contract/tests/migration_contract.rs#L67),
[effective-role pool](../../tests/integration/migration-contract/tests/migration_contract.rs#L6461),
[runner settings](../../crates/job-queue/src/bin/owner-beta-runner.rs#L150),
[retry formula](../../crates/job-queue/src/queue.rs#L775).

These are coordinator implementation decisions within the adopted contract. No new
independent review or executable acceptance is claimed. The first SQL candidate
must be reviewed against these decisions before its one hosted validation run.

## 2. Ownership and assembly order

The coordinator implements directly; the existing prohibition on further worker
delegation continues. The six I2 files are:

- `migrations/0060_owner_beta_daily_inputs.up.sql`
- `migrations/0060_owner_beta_daily_inputs.down.sql`
- `tests/integration/migration-contract/tests/migration_contract.rs`
- `deploy/db/integration-validation/preflight-0060.sql`
- `deploy/db/integration-validation/actual-role-0060.sql`
- `deploy/db/integration-validation/rollback-0060-guard.sql`

Sequential I0 integration owns `deploy/db/integration-validation/validate.sh`,
`static-check.sh`, `README.md`, and `EVIDENCE_TEMPLATE.md`. Existing bootstrap SQL
and two Rust common helpers may change only for a concrete readiness defect;
their acceptance is not permission for unrelated rewrites. New daily test helpers
stay in the existing migration-contract test target; no Cargo dependency change
is expected. The official CI workflow and old migrations 0001–0059 stay unchanged.

Assemble in order: catalog/ACL/epoch and shape checks; calendar/receipt/admission
authority; reservation/publication/queue guards; complete guarded DOWN; actual-role
tests and sequential harness wiring. Each is an implementation slice of **one**
0060 pair, not an independently deployable partial migration. A partially assembled
0060 must not be dispatched: every full MIGRATOR consumer would see it immediately.

Source/static inspection is local. Cargo, SQL, project helpers, containers and
fixtures run only later in the existing hosted disposable validation workflow.
No provider, production or runtime operation belongs to I2. I6 retains diagram
updates and rendered evidence before structural integration into main.

## 3. Catalog and SQL transport

Implement the ten tables from D1/D5: control, admissions, admission_events,
reservations, poll_state, calendar_sources, calendar_days, source_events,
publication_claims and poll_receipts. All names have prefix `owner_beta_daily_`.
Use their exact columns/keys in D1 §5 and D5 A2/A4/A6, with these refinements:

- Control adds D5 `lease_seconds`, §1 `policy_json` and `backoff_seconds`. All are
  required; policy_json is an exact D4 Policy object. No inserted control is enabled
  or has a nonzero activation_epoch. Owner/id remain immutable; deletion is denied.
- Poll receipts also retain derived owner_user_id, policy_sha256 and activation_epoch.
  Poll state adds `latest_receipt_id` referencing its current receipt. Stage strings
  are uniformly `ADMISSION` and `SCHEDULE`; lower-case D1 proposal strings are obsolete.
- Receipt linkage and immutable evidence FKs use RESTRICT. State/receipt updates are
  internal only. No serving role gets direct DML on any of the ten tables.
- Config column names are D5 `daily_enabled` and `daily_epoch`, not D1's earlier
  prefixed spelling. Run daily fields and exact tagged payload remain D1 §7.
- Every added table is owned by migration_owner with ENABLE/FORCE RLS. Actor-scoped
  app reads must also require current Owner membership. Internal definer policies
  remain explicit under FORCE RLS; no ambient cross-tenant actor access is added.

`p_admission` is a SQL-only object with **exactly** four keys:

```text
admission_sha256: ContentHash of exact canonical D4 admission bytes
source_frontier_sha256: D5 A6 source-observation hash for the verified scan
admission:       complete DailyPriceAdmissionV1Wire object
manifest:        complete DailyPriceManifestV1Wire object
```

It contains no caller-selected Owner, DB status, timestamp, path or activation epoch.
SQL derives these from protected control and the active receipt. Persist this exact
JSON projection as admissions.evidence_json and compare all descriptors on replay.
The transport is not an admitted artifact and is never serialized as admission.json.
Before submission the narrow admitter must validate canonical bytes, byte limits,
hashes and the verified bundle. SQL independently validates exact JSON key sets/types,
scalar/count bounds, cross-field equality, current protected policy/base, parent
prefix and next session. SQL trusts that isolated role for byte/hash correspondence
and actual source reads; a SQL fixture does not prove either.

Admission columns derive content/manifest/policy/parent from `admission`, range from
`manifest`, and counts from the matching manifest/admission counts. All daily inputs
have range_start=2016-08-29. The first real continuation has 2,453 sessions, 26,983 bars
and one added session; D4's small parser golden is never a trusted base fixture.
Later inputs must contain the parent sessions/daily_sources prefix unchanged and
exactly one additional proven TRADING session, with no skipped intervening session.
Current scope/entitlement must cover the whole cumulative range.

Control's protected policy projection is the SQL comparison authority. Only the
controlled installer may supply it; a matching caller hash does not authorize a new
base or ETF set. No real policy hash is selected by this document. SQL stores no
provider response bodies, credentials or free-form diagnostics.

## 4. Callable authority and receipt state machine

All functions are in public, owned by migration_owner, with qualified object names,
fixed `search_path=pg_catalog,pg_temp`, bounded timeouts, explicit PUBLIC revocation
and exact per-role EXECUTE grants. Trigger-only helpers have no serving-role EXECUTE.
Restore the prior actor GUC on normal and exceptional exits. No caller-set GUC is an
authorization token. SQLSTATE 42501 is privilege denial; 23514 is invalid shape/state;
55000 is guarded DOWN refusal. Expected source conflicts commit typed outcomes.

The final SQL interfaces are:

```sql
-- daily_input_admitter only; zero rows means rate-limited/no usable control.
take_owner_beta_daily_admission_poll_slot() RETURNS TABLE(receipt_id uuid);
-- worker only; additionally requires enabled control.
take_owner_beta_daily_schedule_poll_slot() RETURNS TABLE(receipt_id uuid);

-- daily_input_admitter only; days use D5 A2's exact five-key objects.
project_owner_beta_daily_calendar(p_receipt_id uuid,
  p_calendar_evidence_sha256 text, p_file_evidence jsonb, p_days jsonb)
  RETURNS TABLE(disposition text, calendar_evidence_sha256 text,
    reason_code text, event_sha256 text);

admit_owner_beta_daily_input(p_receipt_id uuid, p_admission jsonb)
  RETURNS TABLE(disposition text, admission_sha256 text,
    reason_code text, event_sha256 text);

-- The event object is specified below; only the admitter may report source faults.
block_owner_beta_daily_input(p_receipt_id uuid, p_event jsonb)
  RETURNS TABLE(disposition text, admission_sha256 text,
    reason_code text, event_sha256 text);

-- Non-admission completion: INPUT_PENDING, INPUT_BLOCKED or IO_FAILED only.
finish_owner_beta_daily_admission_poll(p_receipt_id uuid, p_result_code text,
  p_source_frontier_sha256 text) RETURNS text;
-- worker only: completes its exact schedule receipt; no VERIFIED source assertion.
finish_owner_beta_daily_schedule_poll(p_receipt_id uuid, p_result_code text)
  RETURNS text;

read_owner_beta_daily_source_evidence(p_start date, p_end date)
  RETURNS TABLE(evidence jsonb);
owner_beta_daily_schedule_candidates()
  RETURNS TABLE(strategy_config_id uuid, expected_session date,
    admission_sha256 text, config_epoch bigint, strategy_id text,
    strategy_version text, config_json jsonb);
reserve_owner_beta_daily_run(p_config_id uuid, p_expected_session date,
  p_admission_sha256 text, p_config_epoch bigint, p_strategy jsonb)
  RETURNS TABLE(run_id uuid, job_id uuid, disposition text);
lock_owner_beta_daily_run_gate(p_run_id uuid) RETURNS boolean;

-- migration/release authority only; no new serving-role grant.
revoke_owner_beta_daily_input(p_admission_sha256 text, p_evidence_sha256 text)
  RETURNS boolean;
```

Calendar success disposition is PROJECTED or REPLAY, with its hash and null reason/event;
conflict is BLOCKED, null usable hash and a typed reason/event. Success leaves the
receipt open for subsequent calendar sources and admission. BLOCKED finalizes it
with the durable source event. Admission returns D5 ADMITTED/REPLAY/BLOCKED exactly.
ADMITTED/REPLAY completes its receipt as VERIFIED only after admission gates pass;
external finish cannot assert VERIFIED. Finalize all outcomes and poll state in the
same transaction. A conflicting or superseded incomplete receipt raises 23514 without
changing admission/current freshness; a completed identical operation returns REPLAY.

`p_event` has exactly source_kind, range_start, range_end, code and evidence_sha256,
using D5 A2 scalar sets. The admitter may report every listed source fault except
SOURCE_REVOKED, which remains explicit release authority. SQL derives Owner, affected
admissions and deterministic D5 A6 event identity. `revoke_*` appends the D1 REVOKED
admission event under the same source fence; it does not invent a provider observation.
No identifiable observation means INPUT_BLOCKED without a fabricated event hash.

Slot acquisition takes a stage transaction lock, checks DB time, creates its receipt
and moves next_poll_at to at least start+60s, then commits before file IO. Completion
rechecks stage, derived authority and latest_receipt_id. Old completed receipt replay
does not refresh current state. A crash remains an incomplete historical receipt;
a later slot supersedes it without deleting evidence. Receipt timestamps are DB-owned.
Schedule completion permits CHECKED, DEFERRED or BLOCKED and never sets admission-stage
freshness. Scheduler gates require the latest admission receipt to be VERIFIED, with
matching admission/policy and age <=180 seconds. A failed later observation clears it.

`read_*` is admitter-only and preserves D5's <=370 civil-day bound and dedicated
calendar projection; generic data_batches/trading_calendars are not created or altered.
Candidates/reserve/gate are worker-only. Config/control mutation is not a side effect
of any of these calls. Calendar expected-session proof works before scheduler opt-in.

## 5. Locks, queue and historical compatibility

Use D5 A4's exact order. Reservation first takes the existing owner capacity
advisory lock with seed7919. Then control, user/Owner,
exact rights in stable order, config, owner source advisory lock 7920, ancestors in
ascending range_end/hash, and finally receipt/state. Publication takes job/current
attempt/run first, then the same authority order. Writers of calendar/admission/events
take the source lock exclusively; proof/reservation/publication take it shared.
No poll-slot row stays locked while acquiring higher-order authority locks.

Permanent reservation identity is owner/config/session/trigger_version. Repeated
calls return the original job/run even after configuration or input changes, subject
to current read authority. Capacity deferral creates no row. Payload and all pins,
epochs, snapshot and current attempt identity are immutable. The caller supplies
neither job/run IDs nor queue payload; SQL constructs the exact D1 §7 daily tag.

Immediate guards enforce valid transitions and current old-row claim ownership.
Deferred constraint triggers verify both directions of reservation/run/job/attempt/
claim/item linkage at commit, including hostile direct DML and SET CONSTRAINTS
IMMEDIATE. Use D5 A4's five-row final-state matrix verbatim. Terminal daily output
cannot be edited, erased or resurrected. Items may only commit with success; the
publication claim is produced by the success trigger from actual locked authority.

Lease timestamps use DB clock. Daily retry timestamps use the protected backoff and
current attempt number; no arbitrary far-future available_at. Preserve job-then-attempt
failure and attempt-then-job recovery ordering, with deferred final checks. Cancellation
allows the documented one-way drain only. Recovery must compare claimed_by with the
job's worker; the current RecoveryAttempt struct does not contain claimed_by, so I4
must supply it. I2 fixtures exercise the intended SQL transitions directly; they do
not claim the existing runtime already decodes daily payloads.

Backoff interval arithmetic is checked and timestamp overflow fails closed. SQL stamps
the daily retry time from the protected setting; it does not accept a caller's chosen
delay. This does not change historical retries or select a new production queue setting.

Historical five-pin untagged payloads, snapshots, outputs, timestamps and manual
behavior remain exact. Copy the existing historical branches of 0050/0052 into the
new function replacements and restore those exact definitions in DOWN. Do not edit
old migrations or reuse 0050's empty-run-table guard. Add defaults without rewriting
legacy JSON. Daily guards return unchanged for unrelated historical/queue rows.

## 6. Migration and rollback acceptance

UP rechecks the provisioned role attributes, CONNECT/USAGE, absence of CREATE and
direct/transitive membership paths before DDL. It creates no login. All new ACLs and
RLS become effective in the same transaction. Narrow app config INSERT/UPDATE using
D5 A3's exact column lists, not additional permissive grants. Freeze config identity,
increment daily_epoch on real changes, allow OFF after Owner-role loss, deny retaining
true after loss. Both epoch overflow and explicit caller epoch writes fail closed.

DOWN locks every affected table ACCESS EXCLUSIVE with finite timeouts, temporarily
removes FORCE only on inspected owned tables, and checks **all tenants**. Refuse for
any daily run/admission/event/reservation/claim/calendar row/poll receipt, any nonzero
config epoch or opt-in, or activated control history, including restored-OFF state.
Only absent/pristine disabled control and empty poll slots are removable. Restore
FORCE RLS before commit. Failure must roll back DDL, ACLs and RLS state unchanged.

Allowed DOWN removes only daily additions, revokes the new config column grants,
restores previous table grants, NOT NULL/discriminant checks and exact historical
functions/triggers. It leaves the provisioned login inert. Never delete daily evidence
to make rollback pass. A populated feature requires a compatible reader with mode OFF;
production release compatibility enforcement remains I6.

## 7. Required named actual-role cases

All names below are **SPECIFIED_NOT_IMPLEMENTED**, not written test definitions or
execution evidence. Each becomes a separate Rust test in the existing migration_contract
target with the prefix `daily_0060_`. Parameterized subcases print unique fixed labels;
an aggregate pass is not evidence that every label ran. Preserve existing test names.

| Suffix | Distinguishing assertions |
| --- | --- |
| role_preflight | Every forbidden attribute/membership/CREATE case; direct-login positive control; denied migration leaves no daily objects |
| up_preserves_histories | Actual 0059 fixtures: v2/v3 × PENDING/FAILED/CANCELED/SUCCEEDED, exact stored payload/provenance/output/item/timestamp comparison after UP |
| repeat_is_noop | 60 checksums, same historical rows/catalog, no seeded control or automatic opt-in |
| catalog_authority | Ten table owners/RLS/FORCE, all function search_path/owners/EXECUTE and effective column ACLs |
| direct_principal_denials | app/worker/research_writer/admin/admitter direct logins, CREATE/SET ROLE/DML/EXECUTE denial and rightful positive controls |
| config_owner_optin | Member enable/preserve-true denied; matching Owner accepted; OFF after role loss allowed |
| config_epoch | No-op, edit/restore, OFF/ON, immutable IDs/strategy/owner/created_at, explicit epoch and overflow |
| control_epoch | Disabled epoch0 insert; all authoritative fields increment; no caller epoch/reset/delete |
| calendar_projection | Original requested target, copy chain, future retrieval, exact days/ordinal/kind, equal overlap replay and conflicting overlap committed event |
| calendar_before_admission | Valid proof while control OFF with no admissions/configs; no generic calendar writes |
| calendar_gap_cutoff | Continuous civil days, gap, before/after 16:30 KST, future schedule facts, latest eligible date and 370-day bound |
| admission_transport | Exact outer/nested keys and types, protected policy/base, cross-field/hash descriptors, count/prefix/next-session rejection |
| admission_parent_chain | First real base counts, one-session extension, no gap, immutable parent prefix and blocked ancestor |
| admission_conflict_replay | Exact replay preserves first timestamps; same-date variant commits BLOCKED and stable event; lost-reply reread converges |
| poll_rate_and_crash | Two direct connections, one slot per 60 seconds, incomplete crash receipt, next-slot supersession |
| receipt_authority | Wrong stage/Owner/policy/epoch, stale completion, duplicate/conflicting completion and caller VERIFIED denial |
| event_and_freshness | Event key idempotence, descendant effects, malformed-unidentified observation, latest failed receipt and 180-second expiry |
| reservation_dedup | Two schedulers/lost reply, one permanent key/job/run, snapshot unchanged after edits; no partial commit |
| capacity_deferral | Existing shared capacity lock, no reservation or attempt consumed when full |
| latest_only | Missing newest admission never falls back; first_auto_session, disabled control, expired rights, unknown expected date |
| direct_daily_spoof | Tagged orphan job/run, mixed/unknown payload, wrong Owner/config/pins/epoch and unmatched reservation cannot commit |
| valid_publish | Actual current claim, exact ETF11 items/ranks/weights/cash, hashes and receipt; all queue/run/attempt outcomes commit together |
| terminal_immutability | Changing every terminal run/output/item/job/attempt identity is denied; identical permitted no-op stays identical |
| claim_fences | Wrong/old worker/attempt, forged future lease, stale heartbeat, receipt construction and pre-ended attempt success denied |
| retry_and_recovery | Live transient retry and expired orphan, same job/run, bounded backoff, count progression, exhausted atomic failure |
| cancel_drain | Queued/running cancellation, live drain/expired orphan cleanup, no success/requeue after cancel |
| deferred_consistency | Every mismatch in D5's five-state matrix rejected at COMMIT and SET CONSTRAINTS IMMEDIATE; no items leaked |
| revoke_publish_races | Both orders of config edit/restore, OFF/ON, control disable, entitlement/Owner removal and source event vs publication |
| cutoff_while_waiting | Authorization is re-evaluated after a blocked gate passes an eligible-date boundary |
| down_unused | Pristine control/empty slots allowed; exact historical rows, ACLs, functions/triggers and FORCE state restored |
| down_all_tenants | Every refusal category individually, second Owner hidden by actor, restored-OFF epoch history; no deletion to pass |
| down_atomicity | Denied DOWN leaves ledger/schema/data/ACL/RLS unchanged; allowed DOWN then UP restores source contract |

Use real two-connection lock observations and bounded handshakes, not sleeps as proof
that an update waited. Use disposable synthetic fixture authority only. Capture secret
and row snapshots in memory; emit fixed case IDs, status and SQLSTATE, not payloads,
passwords, URLs or free-form SQL/provider errors. Independent fixture databases avoid
suite-wide serialization. Always close pools and remove the owned scratch DB.

## 8. Official harness handoff and exit gate

Extend the existing staged lane through0059, then seed/inspect historical fixtures,
preflight0060, apply0060, inspect catalog/actual roles, no-op rerun and guarded DOWN.
Retain its earlier0039/0040/0041 tests and disposal. Add explicit before/after snapshots
and named labels for the unused and second-Owner DOWN cases; failures are not SKIP.
Keep the independent empty full-release lane and require 60 ledger/checksum rows on
both passes. Tear it down before the DB-test lane, as today.

The existing DB test invocation remains
`cargo test --locked -p migration-contract --test migration_contract -- --nocapture`
with DATABASE_URL supplied only by the official disposable harness. New direct-role
pools must use structured connection options and assert current_user=session_user,
including research_writer and migration_owner; effective SET ROLE pools do not count.
Raw subprocess logs stay private. Add sanitized named daily test/case rows and actual
process exit status to the harness evidence so future recovery does not repeat the
older UNKNOWN_NOT_PRINTED gap. Retain the current twelve labels and bootstrap83 cases.

Source disposition, both migration files, all case definitions, fixed inventories and
I0 sequential wiring must be complete before one exact-candidate hosted dispatch.
Query all-event existing runs for that exact head first. Never rerun/cancel/duplicate
old runs. Validate all jobs/checkout SHAs/checksums, direct test rows, Clippy/format,
no-SKIP and cleanup. A missing row remains UNKNOWN unless a failed phase proves it
NOT_EXECUTED. No local fixture execution substitutes for this gate.

This contract closes the Wave35 request to freeze I2 interfaces and acceptance scope.
Next deliverable is the complete0060 SQL/test/harness candidate. I2 implementation,
provider/copy reconstruction/admission, I3–I7, runtime/Owner and final C_R2 stay open.
R3 priority is unchanged; the user's continuation was taken as continuing existing R2.
