# R2 WP-4 D5 — daily-input architecture amendments

2026-10-14. Architecture proposal correcting the six accepted D3 findings; no source
implementation or operational activation. U2 adoption and source authorization persist.
These overrides take precedence over the identified D1 sections, leaving other D1
requirements intact. Neither this report nor D4 approves a changed acquisition contract.

Frozen references: [D1](2026-10-14-r2-daily-input-design.md), SHA256
`e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670`, 926 lines;
[D3](2026-10-14-r2-daily-input-design-review.md), SHA256
`1ea75c9e1f8612475923775c5b2b49c2f3b7a4a2aa395e482d09cda1c4c9a487`, 423 lines.
Product baseline is `619634db32526032aae07ee2024a10f69fc560c4`. Shared documentation
HEAD advanced from supplied `b476cda` to `16ce54beda057e1b0d13c3e95f7976d20197c066`
during this task; frozen reference hashes still matched. D5 owns only this file.
D4 owns `docs/contracts/owner-beta-daily-etf11-input-v1.md`, which appeared during D5.
The observed 773-line draft SHA256 was
`ec8f31eb574bce06fe567291934eaf85be83c29cf1053fa8ec1814b9ee2de707`.
Its field layout, encoding and artifact hash preimages are not restated here; the
coordinator must bind the final D4 revision at implementation launch.

Unchanged: historical v2/v3 bytes and exact v3 five pins; raw-price, cash-dividend-only
daily admission under a pinned policy; separate producer/admitter; default-OFF Owner
strategy opt-in; permanent Owner/config/day/trigger-version dedup; sealed input and
strategy snapshot; current rights through publication; local polling at most once per
minute; latest eligible session only; separate manual history; no recommendation backlog.

## A1. D3-01 — interval discovery and recurring supply

Overrides D1 §§4, 6 source discovery, 8, 11 and I1/I6 supply assumptions.

**Evidence.** Daily KSD requests use `date,date`
(`crates/market-data/src/providers/kis.rs:289`); the daily wrapper selects trading sessions
(`scripts/ops/kis-daily-production.sh:503`, `:616`). Range acquisition is already
sequential and atomically complete (`crates/market-data/src/providers/kis.rs:311`), but
its manifest partition date is the range start (`crates/market-data/src/ingest.rs:835`,
`:867`). A partition-date filter loses valid evidence and a one-time catch-up leaves
the next weekend uncovered. The range CLI is separately operator-confirmed
(`data-pipelines/collectors/src/bin/kis-action-range-raw.rs:303`).

### Offline selection, fixed independently of actual inventory

1. Snapshot only complete committed Raw manifest lines in approved KIS scopes. Retain
   exact line bytes/hash, batch metadata and files. Index corporate-action requests by
   validated inclusive `F_DT..T_DT`, class, scope and page chain, **never** by
   `ManifestEntry.date`. The partition date remains unchanged lineage. A snapshot may
   contain earlier-partitioned ranges overlapping the required extension.
2. Accept a complete WholeMarket7 bundle or a complete FixedEtf11 bundle (77 groups).
   Reuse the exact request matrix and page metadata checks at
   `crates/market-data/src/providers/kis.rs:1198`; do not assemble a supposedly complete
   bundle from failed/partial Raw writes. Multiple complete bundles may jointly cover
   the required civil interval. A class/symbol chain must contain pages 1..N, N<=10,
   initial blank `tr_cont`, subsequent `N` only after exact response `M`, unchanged
   query/blank `CTS`, terminal final page, no repeated bytes or duplicate page.
3. Prove every civil day from the prior sealed frontier+1 through the proposed session
   for all seven classes and all ETF11 symbols. Whole-market coverage projects to each
   ETF only after validating every response row, including non-ETF rows. A valid empty
   completed chain is negative evidence; absent/malformed pages are gaps. Never infer
   closed-day action absence from a trading calendar. Every required ETF bar/session
   and every intervening calendar day still needs its own D1 evidence.
4. Overlap comparison is deterministic. For each day/class/symbol cell, collect all
   complete covering chains in the snapshot, validate every row first, then compare
   the complete **multisets** of rows for that symbol and `record_date`. Use all parsed
   row members with recursively ordered object keys for internal equality, preserving
   array order/scalar values; never collapse duplicates or compare only prices/cash.
   `record_date` is the existing delivery key, not payment/listing/ex-rights date
   (`crates/market-data/src/normalize.rs:1369`, `:1391`, `:1455`). A row outside its
   requested interval fails closed; future payment dates remain evidence, not availability.
5. Unequal cell results, duplicate event rows, invalid chains or contradictory empty
   evidence produce BLOCKED plus a durable conflict observation. Among equivalent
   covering chains select the lexicographically smallest tuple
   `(start,end,scope_literal,original_manifest_line_sha256,batch_uuid)`; record the other
   compared chains in D4's actions array as witnesses; selection chooses the semantic
   contribution, never deletes evidence from that array.
   Never prefer newest retrieval or WholeMarket over ETF11 to hide a disagreement.
   Select entire page chains, not individual winning rows. Snapshot iteration order,
   filesystem order and the producer's chosen batch ID cannot affect the result.
6. Once sealed, reopen the exact selected sources. An identical source frontier replays
   the existing admission. Newly committed alternative evidence for a sealed date is
   a same-date change: block automatic resealing even if computed prices match; retain
   the old admission and record the observation. A release/version change alone does
   not remap old source identities. Required-source disappearance or mutation also
   blocks. Unrelated provider/date changes do not invalidate a sealed input.

Discovery can use bounded persisted indexes, but a cached index must bind the exact
committed prefix and scan appended complete lines before claiming verification. A
configured scan limit returns `SOURCE_SCAN_LIMIT`, never a truncated success. Inventory
absence does not block implementing or fixture-testing these rules.

### Concrete recurring supplier SOURCE proposal — original D5 boundary

2026-10-21 update: the original source-only boundary below records D5's scope.
Owner's later blanket R2 authority and explicit evidence-measured budget decision
authorize the separate Wave135 runtime connection. Official KIS sample commit
2077773a69a428daf63e148182511f068cc45de3 was checked for the same six range GETs
and GB1 date distinction; no endpoint, method or pagination policy changed.
The bounded runtime now exists in `collectors::action_supply`, with durable
same-day reservations and source-derived frontier. The nonsecret protected hex
configuration and explicit immutable installer flags are documented in
[the installation runbook](../runbooks/owner-beta-daily-input.md).
This source update does not claim that the new supplier is deployed or active;
matching candidate CI/images, historical gap closure and actual input gates remain.

Keep the existing EOD bundle unchanged. Propose one additional **Raw-only**, WholeMarket7
range capture per eligible KST run, through the existing range capability; no calendar,
reference or bar request is added by this capture. This avoids changing the generic
EOD normalizer's target-date identity. It deliberately adds action requests rather than
silently substituting range data into daily publication.

The exact six GET path/TR pairs remain those in
`crates/market-data/src/providers/kis.rs:1004`: paidin-capin/HHKDB669100C0 twice
(`GB1=1,2`), bonus-issue/HHKDB669101C0, dividend/HHKDB669102C0 (`GB1=0,HIGH_GB=''`),
merger-split/HHKDB669104C0, rev-split/HHKDB669105C0 (`MARKET_GB=0`), and
cap-dcrs/HHKDB669106C0; all share `/uapi/domestic-stock/v1/ksdinfo/`.
Only the dates/cadence change: `F_DT=T_DT=session` becomes
`F_DT=covered_through_civil_day+1`, `T_DT=latest_eligible_session`, with `SHT_CD=''`,
`CTS=''` and the same class extras (`crates/market-data/src/providers/kis.rs:1070`).
For a Friday frontier and Monday target this covers Saturday..Monday. An actual
calendar-proven Friday target on a closed Monday requires no new range.

Proposed protected supplier configuration: `enabled=false`, explicit activation KST
date, fixed `scope=whole-market`, an independently evidenced initial civil frontier,
`max_gap_days=14`, `max_ranges_per_kst_day=1`, `max_pages_per_class=10`,
`max_wire_gets_per_kst_day=280`. These are proposed ceilings, not claims about the
approved budget. No request precedes activation; no target exceeds the independently
proven eligible session at/after 16:30 KST; `end-start+1>14` returns `SUPPLY_GAP_LIMIT`
without automatic chunking/backlog. A later controlled catch-up can establish a new
frontier; it does not remove the recurring range requirement. Advance the frontier
only after a complete committed range and offline class/symbol/page validation, never
after job success, artifact admission or an HTTP attempt. Reconstruct it from committed
evidence on restart; a local mutable counter alone has no authority.

All calls, pages and retries are sequential under existing token reuse/shared read
coordination and the one-request/second endpoint/TR ceiling. Seven initial groups,
at most 70 logical page requests, at most four attempts per page = **280 wire GETs**,
including retries (`crates/kis-client/src/retry.rs:24`, `:33`). The existing EOD requests
are additional; G4 must budget their sum and other users of the same App Key. Retry
only already-classified transient failures, honor longer Retry-After; a deadline
ending the run aborts without Raw visibility. Never shorten broker backoff to finish.
Failed/partial runs do not advance the frontier. Persist the one-range/day attempt
reservation and consumed request budget before dispatch; restart cannot obtain a second
budget or replay an uncertain request automatically. Next eligible daily run may retry
the still-uncovered bounded interval. Schema/noncash/conflict failures require review,
not repeated network retries. FixedEtf11 is accepted offline, but is not an automatic
supplier fallback: it would raise the ceiling to 77/770/3080 and needs its own budget.

Source work may implement the **pure** planner and fixture expectations now. No new
timer/CLI execute wiring is part of that package. G4 must separately verify current
official endpoint range/date semantics and obtain approval for this exact recurring
acquisition/date/budget delta, choose actual activation/frontier/deadline/aggregate
budget, then authorize its immutable-release wiring. Existing once/day `chk-holiday`
and KSD continuation rules remain unchanged. D5 made no external-source verification.

## A2. D3-02 — separate offline calendar proof

Overrides D1 §§4–6 calendar requirements, §8 expected-session derivation and §9 source
projection. The generic calendar paths stay byte-for-byte in behavior: target filtering
at `crates/market-data/src/normalize.rs:1144`, single-target sink at
`data-pipelines/collectors/src/sink.rs:566`, and the existing three-way target-date join
at `crates/api-server/src/repos/service_status.rs:288` remain unchanged.

The isolated admitter first projects already-acquired credentialed first-page Raw,
independently of daily input availability/admission and runtime enablement. Validate
method/path/TR/query, blank request continuation, first-page identity, success/schema,
unique dated rows and Y/N open flags. The original requested BASS_DT must equal the
original Raw source target and be present exactly once. A source with surrounding
rows but no target is unusable. Preserve ignored response-continuation metadata; do
not follow it or require an extra request. Resolve every `copied_from` hop to a committed
original, matching hash, request, entitlement, date and retrieval timestamp; reject
cycles/missing originals. No copied timestamp is reset to projection time.

0060 adds the following relational projection (SQL types below are operational storage,
not D4 wire fields). All hashes use `text CHECK (~ '^sha256:[0-9a-f]{64}$')`; all listed
columns are NOT NULL except `admission_sha256` in source events. Every FK is RESTRICT.

| Table | Exact columns and keys |
| --- | --- |
| `owner_beta_daily_calendar_sources` | `calendar_evidence_sha256 text PRIMARY KEY`, `owner_user_id uuid REFERENCES users(id)`, `policy_sha256 text`, `normalizer_version text`, `source_provider text`, `source_market text`, `source_batch_id uuid`, `source_batch_date date`, `requested_target date`, `source_file_name text`, `source_file_sha256 text`, `source_manifest_line_sha256 text`, `source_retrieved_at timestamptz`, `copy_chain jsonb`, `evidence_json jsonb`, `projected_at timestamptz DEFAULT clock_timestamp()`; UNIQUE `(owner_user_id,calendar_evidence_sha256)`. `evidence_json` is one D4 DailyFileEvidenceV1Wire source object decoded to jsonb, not an entire session/manifest; exact bytes remain in the admitter-owned evidence store. |
| `owner_beta_daily_calendar_days` | `owner_user_id uuid`, `calendar_evidence_sha256 text`, `session_date date`, `session_type text CHECK IN ('TRADING','CLOSED')`, `evidence_kind text CHECK IN ('REQUESTED_TARGET','DERIVED_SURROUNDING')`, `source_row_ordinal int CHECK >=0`, `source_row_sha256 text`; PK `(owner_user_id,calendar_evidence_sha256,session_date)` and composite FK to calendar_sources. Source row ordinal is the original response-array index (object output uses 0). |
| `owner_beta_daily_source_events` | `event_sha256 text PRIMARY KEY`, `owner_user_id uuid REFERENCES users(id)`, `source_kind text CHECK IN ('ACTION','CALENDAR','BAR','REFERENCE')`, `range_start date`, `range_end date CHECK >=range_start`, `code text CHECK IN ('SOURCE_CONFLICT','SEALED_SOURCE_CHANGED','SOURCE_REVOKED','SOURCE_INTEGRITY','CALENDAR_CONFLICT','NONCASH_ACTION')`, `evidence_sha256 text`, `admission_sha256 text NULL REFERENCES owner_beta_daily_admissions(admission_sha256)`, `recorded_at timestamptz DEFAULT clock_timestamp()`. This also records pre-admission conflicts; its existence does not assert artifact approval. |

The calendar source identity is SHA256 of UTF-8
`owner-beta-daily-calendar-source-v1\n` + lowercase Owner UUID + `\n` + policy hash +
`\n` + `daily-raw-reconstruction-v1\n` + D4 canonical FileEvidence bytes. This is an
operational projection key, not a new D4 artifact schema. Source-row hash is SHA256 of
the parsed original row serialized compactly with recursively sorted object keys;
the whole original wire hash and ordinal preserve its byte provenance as well.
Store original source provider/market values, not generic normalized `KRX/KR` aliases.
`source_batch_date=requested_target`; surrounding `session_date` need not equal either.
Copied-chain JSON and all corresponding file/line hashes are verified by the isolated
Rust verifier before SQL submission. SQL validates D4's FileEvidence structure,
cross-field equality, target-row inclusion, row uniqueness/count and current exact
Owner/policy/entitlement scope. It must not pretend to re-open files itself. Original
retrieval <= each copy retrieval <= projection observation <= DB clock; future dated
calendar rows are valid schedule facts, but are not eligible sessions before their
cutoff. Future retrieval timestamps are invalid. Timestamp precision follows D4.

Only `daily_input_admitter` can EXECUTE
`project_owner_beta_daily_calendar(p_calendar_evidence_sha256 text,
p_file_evidence jsonb,p_days jsonb) RETURNS TABLE(disposition text,
calendar_evidence_sha256 text,reason_code text)`. It inserts source+all days atomically
under the same owner source lock used by admission. Exact repeats are REPLAY. Any
disagreeing type for the same owner/date is BLOCKED with a committed source event;
never overwrite/select the newer answer. Equal overlapping facts can coexist as
additional witnesses; deterministic query selection orders by evidence hash. A changed
source touching a sealed admission also invokes A1's same-date block rule. Invalid
evidence has no usable projection; commit a typed poll failure receipt. No direct
INSERT/UPDATE/DELETE/TRUNCATE grants to serving roles; definer functions are owned by
migration_owner. Worker/API get only scoped function results. Use FORCE RLS with
Owner-scoped policies and the existing controlled definer actor discipline.
`p_days` is a nonempty array of precisely the five day columns after the two FK columns
in A2's table; no extra keys. Derive evidence_kind from equality to requested_target
and compare it with the submitted value. Source columns describe the terminal original;
evidence_json preserves current delivery and the full chain. They are extracted from
FileEvidence, not additional caller fields. The isolated verifier supplies the hash
after canonical-byte validation; SQL's trust in that narrow role is explicit, as with
artifact admission. A BLOCKED projection also records its event in the poll receipt.

The new scoped query is structurally:

```sql
SELECT d.session_date, d.session_type, d.evidence_kind,
       s.calendar_evidence_sha256, s.requested_target,
       s.source_batch_date, s.source_retrieved_at, s.projected_at
FROM public.owner_beta_daily_calendar_days d
JOIN public.owner_beta_daily_calendar_sources s
  ON (s.owner_user_id,s.calendar_evidence_sha256)
   = (d.owner_user_id,d.calendar_evidence_sha256)
WHERE s.owner_user_id = configured_owner
  AND s.policy_sha256 = configured_policy
  AND d.session_date BETWEEN proof_start AND eligible_civil_date;
```

The enclosing definer additionally rejects any covering source conflict/revocation,
checks current rights and observation freshness, and groups equal facts once; a
disagreement blocks the **whole proof**, not merely the bad row. It never joins
`data_batches.batch_date` to a derived surrounding date and never creates generic
data_batches/trading_calendar rows. Amend `read_owner_beta_daily_source_evidence` to
read this projection plus scoped existing non-calendar evidence; keep its <=370-day
query bound. Longer cumulative histories are verified from D4's sealed prefix plus
bounded extension, not by silently truncating SQL results.

Compute `eligible_civil_date=today` at/after 16:30 KST, otherwise yesterday. Require
continuous facts from the latest claimed TRADING date through that date; missing dates
return `CALENDAR_COVERAGE_GAP`, not a weekday guess or a stale expected date. Extension
verification separately requires every civil day from the prior sealed frontier.
No bounded proof => expected unknown, not permission to run the prior input. Reuse
the cutoff/continuity semantics, not the generic DTO's lineage assertion
(`crates/api-server/src/service_status.rs:3204`). A newly eligible session is pending
until supported; after its 18:30 KST deadline it is late/blocked if unpublished.
Expected derives from calendar proof, support from admitted input, as_of from the run,
published_at from successful commit. An old publication stays visible as old. Expired
verifier receipts make current verification unknown without erasing those dates.

Dependency direction is Raw -> calendar projection -> expected-session proof;
Raw+calendar+sealed prefix -> daily admission -> reservation -> publication. Calendar
projection needs neither an admission nor an enabled scheduler, eliminating the cycle.
No fresh calendar request occurs during minute polling. Later ownership: A8-I1b owns
pure projection; A8-I2 owns SQL; I4 submits it; I5 reads the dedicated daily API projection.

## A3. D3-03 — effective ACLs and epochs

Overrides D1 §§5–6 config/control grants and §10 DOWN. Existing table-wide app grants
(`migrations/0009_grants.up.sql:27`) and permissive FOR ALL RLS
(`migrations/0010_rls.up.sql:49`) are not narrowed by another grant/policy.

0060 performs exactly this app privilege change:

```sql
REVOKE INSERT, UPDATE ON public.user_strategy_configs FROM app;
GRANT INSERT (owner_user_id,strategy_id,strategy_version,config_json,is_active,daily_enabled)
  ON public.user_strategy_configs TO app;
GRANT UPDATE (config_json,is_active,daily_enabled,updated_at)
  ON public.user_strategy_configs TO app;
```

SELECT/DELETE stay as before, subject to RLS/FKs. No new ID, owner, strategy identity,
epoch, created_at or TRUNCATE privilege. Existing HTTP INSERT/UPDATE use the retained
columns (`crates/api-server/src/repos/strategy_configs.rs:55`, `:117`). Add
`daily_enabled boolean NOT NULL DEFAULT false`, `daily_epoch bigint NOT NULL DEFAULT 0
CHECK (daily_epoch>=0)`. A BEFORE INSERT trigger requires supplied/default epoch 0 and
sets 0; epoch writes are not caller inputs. BEFORE UPDATE freezes id/owner/created_at/
strategy_id/strategy_version, rejects explicit epoch changes, and sets OLD epoch+1 iff
config_json/is_active/daily_enabled changed (IS DISTINCT FROM); no-op timestamps do
not increment. Overflow fails closed. Edit-and-restore and OFF/ON each increment twice.
Existing rows start 0, without rewriting their config JSON or historical run snapshots.

On INSERT with daily_enabled=true and on every UPDATE whose NEW.daily_enabled=true,
the trigger must require both `NEW.owner_user_id =
NULLIF(current_setting('app.actor_user_id',true),'')::uuid` and an existing locked
`public.user_roles(user_id=NEW.owner_user_id,role_id='owner')` row, plus the user row.
Use FOR SHARE, not only KEY SHARE: a non-key role update must serialize too. Missing or
malformed actor fails closed. Config owner may turn OFF after losing Owner membership;
it cannot preserve/enable true by editing another field. Use the real membership table
(`migrations/0001_identity.up.sql:25`), not a role claim in JSON/session_user. A trigger
enforces this beyond permissive RLS; a second permissive policy is insufficient.
No source approval is inferred from opt-in.

Control is migration/release-authority-only. INSERT requires enabled=false and epoch0;
first enable is a separate UPDATE. Freeze singleton ID and owner. BEFORE UPDATE reject
caller epoch changes, then increment activation_epoch for any change to enabled,
policy, entitlement reference/hash, first_auto_session, trigger_version, capacity or
the new `lease_seconds integer CHECK (lease_seconds>0)`; timestamps alone do not count.
The installer sets lease_seconds to the actual owner-beta queue setting (current
default 60 seconds, configurable at `crates/job-queue/src/bin/owner-beta-runner.rs:24`,
`:150`); worker refuses a mismatch. Never allow serving roles to set lease length.
No control DELETE after creation; no epoch reset by deleting/reinserting config:
the permanent reservation FK prevents deleting a used config. An unused new config
ID is a distinct opt-in, as in D1. No clock/cron/catch-up meaning is added to trigger_version.

## A4. D3-04 — daily publication authority and terminal immutability

Overrides D1 §§6–7 and I2/I4. Historical paths retain their current checks and bytes.
0052 exits for already-successful rows and runs only UPDATE OF status
(`migrations/0052_owner_beta_strategy_config_lock.up.sql:18`, `:47`); worker can update
output columns today (`migrations/0049_owner_beta_price_recommendations.up.sql:139`,
`migrations/0051_owner_beta_target_publication.up.sql:55`). Checking one transition is
not enough. Add daily-only guards in 0060; leave old migration files untouched.

**Run/items.** Daily run INSERT is only through reservation, initial PENDING with null
outputs/times and no items. Existing app historical INSERT columns cannot create it.
Freeze all daily identity, pins, snapshot, trigger and epoch columns. Reject daily run
DELETE. Once SUCCEEDED/FAILED/CANCELED, reject every changing UPDATE (including time,
error, factor/target hash and cash) and all item mutations. Identical row no-ops may
return unchanged. No terminal resurrection. Daily runs use PENDING while computing
(the existing publisher already does this); disallow RUNNING for the daily branch.
Items have no UPDATE/DELETE path even before success. INSERT requires the parent
PENDING and a valid current claim; items may only survive the transaction that succeeds.

Add `owner_beta_daily_publication_claims` with run_id PK/FK, job_id UNIQUE/FK,
owner_user_id FK, attempt_id UNIQUE/FK, attempt_no positive int, claimed_by nonempty
text, locked_at timestamptz, lease_seconds positive int, authorized_at timestamptz.
All NOT NULL, immutable and private to internal triggers; no serving role has DML or
an exposed receipt constructor. This is DB authority evidence, not a wire artifact.
On PENDING -> SUCCEEDED the BEFORE trigger derives and inserts this receipt from the
locked **current** job and attempt, never from supplied values. It requires:

- Job type/owner/payload/reservation/run and sealed input/snapshot equal; job RUNNING,
  current attempt_no = jobs.attempt_count, attempt.outcome RUNNING,
  attempt.claimed_by = jobs.locked_by. Lock those rows and require
  jobs.locked_at > clock_timestamp() - configured lease. The existing Rust claim
  checks are at `crates/job-queue/src/queue.rs:519`; keep them as well.
- In order: job+current attempt -> run -> control -> user/current Owner row -> exact
  entitlement rows in stable key order -> config -> admission/ancestor/source-event
  authority. Keep D1 capacity locking before reservation and job-before-run publication
  (`crates/job-queue/src/owner_beta/publish.rs:321`). All success gates hold SHARE locks
  through commit. Event writers lock the affected admission/source authority before
  appending; merely checking NOT EXISTS without a common lock is insufficient.
- Control enabled, matching activation_epoch/policy/lease; current Owner; current
  entitlement reference/hash/full cumulative range/current use permission; active,
  opted-in config with exact epoch and sealed snapshot; nonblocked admission/ancestors;
  fresh verification and same latest eligible expected session. A later eligible date
  supersedes the old pending daily run. No fallback to an earlier supported input.

Current valid config/rights/role/control read locks make revoke/edit races serialize:
publication first commits validly before revocation, or revocation first causes failure.
Restore definer actor GUC on success and exceptions; use fixed search_path and qualified
names. A denied gate must leave no success/items and then settle the job/run through
the existing failure/cancel transaction; do not strand a RUNNING job and consume capacity.
An SQL exception rolls back the attempted publication; the worker opens a fresh guarded
settlement transaction. Stale claims defer to current recovery and never settle anew.

The common source authority is concretely a transaction advisory lock on
`hashtextextended(owner_uuid::text || ':daily-source',7920)`: shared for reservation/
publication/read-proof transactions, exclusive for calendar/admission/event writes.
Take it after control/user/rights/config locks (omit config when unrelated), then lock
admission ancestors in ascending range_end/hash order, then poll receipt/state. This
specifies the source-event lock above and prevents absent-event phantom races.
API opt-in transactions lock user/Owner before UPDATE config; direct reversed-order
DML can fail by deadlock, but cannot commit a bypass. Poll-slot allocation is a separate
short transaction and never holds its row while acquiring these authority locks.

**Queue/direct DML.** Add daily-only BEFORE guards to jobs/job_attempts, identified by
the permanent reservation or the explicit daily payload tag. Validate tagged INSERT
shape immediately and require matching reservation/run at deferred commit; do not
require a forward FK before the reservation transaction has inserted all three rows.
No unmatched tagged job may commit, and no untagged job may satisfy a daily reservation.
Freeze job identity/payload/max_attempts and attempt id/job_id/attempt_no/claimed_by.
QUEUED -> RUNNING increments attempt_count exactly one, sets DB-clock locked_at, and
requires one matching new RUNNING attempt by commit. RUNNING heartbeat preserves worker
and count, requires the previous lease still valid, and stamps locked_at with DB clock;
no caller-selected future lease. Existing attempt outcomes are terminal and immutable.
RUNNING -> SUCCEEDED requires the trigger-generated receipt, matching OLD worker/count/
lease and current attempt; success may not be manufactured by first ending the attempt.
The current publisher writes run success before queue success
(`crates/job-queue/src/owner_beta/publish.rs:422`, `:442`); preserve that order.
For daily success only, triggers stamp run finished_at/updated_at at authorized_at,
then job/attempt finished_at with DB clock as they settle. Do not compare a later
clock_timestamp receipt with caller `now()` (transaction-start time) and reject a
legitimate transaction. Preserve historical timestamp behavior. Receipt and run times
become visible only on commit; do not describe them as a measured commit-completion time.

Install DEFERRABLE INITIALLY DEFERRED constraint triggers on daily-affecting changes to
runs, items, jobs, attempts, reservations and publication_claims. Check the final state
at commit, not a transient mid-statement arrangement; SET CONSTRAINTS IMMEDIATE simply
requires a consistent state sooner. Lock authority in the same order for callable
paths; unordered hostile direct DML may abort/deadlock, never bypass validation.

| Final job | Required final daily state |
| --- | --- |
| QUEUED | PENDING run, no items/outputs/receipt, no RUNNING attempts. Initial count0 or a just-failed/orphaned current attempt with attempts remaining and bounded queue backoff. Permanent reservation retained. |
| RUNNING | PENDING run, no committed items/outputs/receipt; exactly current RUNNING attempt and matching worker/lease anchor. Lease expiry itself is handled by recovery, not an automatic constraint failure. |
| SUCCEEDED | SUCCEEDED run, one matching publication receipt and current SUCCEEDED attempt (not any old success), no other RUNNING attempt, unlocked finished job; exactly ETF11 items, unique instrument IDs, valid rank/exclusion/weight rules and sum(target_weight)+cash=1. Non-null factor/target hashes; success times consistent with receipt. |
| FAILED | FAILED run, null outputs/no items/receipt, current attempt FAILED or ORPHANED, unlocked finished job. No daily PENDING/FAILED mismatch awaiting a later healer. |
| CANCELED | PENDING awaiting drain or CANCELED run, null outputs/no items/receipt. An outstanding current RUNNING attempt may retain its original worker/lease until cancel drain or expiry; no further success/requeue. |

Legitimate transitions remain: transient failure closes the current attempt FAILED and
requeues the **same** job with PENDING run; next claim increments count. Expired lease
closes the current attempt ORPHANED and requeues only with attempts remaining, otherwise
atomically fails run+job. Cancellation may win before worker drain; drain changes only
the current attempt to FAILED(canceled), or ORPHANED after expiry, clears residual locks,
and changes a still-PENDING run to CANCELED. Already-CANCELED runs need no UPDATE.
Terminal SUCCEEDED/FAILED jobs cannot change; CANCELED permits only that one-way cleanup.
Immediate settlement guards use the pre-update job worker/count/lease and the current
attempt's immutable identity. RUNNING -> QUEUED requires attempts remaining and either
an unexpired current claim (attempt RUNNING/FAILED) or an expired orphan (attempt
ORPHANED). RUNNING -> FAILED accepts an unexpired permanent/exhausted claim or an
expired exhausted orphan. A RUNNING attempt may become ORPHANED only after its matching
job's lease expires; FAILED only for a matching live failure/cancel settlement;
SUCCEEDED only with the matching success receipt/job. This permits existing failure's
job-then-attempt order and recovery's attempt-then-job order; deferred checks require
both halves by commit. No older attempt may settle the current job. Do not use a
caller-set transaction GUC as a substitute for these old/new-row checks.
Recovery verifies attempt_no=count and claimed_by=locked_by before changing anything
(`crates/job-queue/src/owner_beta/recovery.rs:228`, `:344`). These constraints apply
equally to direct DML and application transactions. Historical heal/retry behavior is
unchanged (`crates/job-queue/src/owner_beta/publish.rs:482`).

### Exact unused-only DOWN and compatibility gate

0060 DOWN takes ACCESS EXCLUSIVE locks on every affected old/new table in one bounded
transaction. As migration_owner temporarily NO FORCE RLS on inspected owned tables,
reject if **any tenant** has: a daily run; reservation; admission; admission/source
event; publication claim; calendar source/day; any admission/schedule poll receipt;
any config daily_enabled or daily_epoch<>0; any control enabled or activation_epoch<>0.
Absent or pristine disabled epoch0 control/empty poll slots alone are removable. A
config epoch increment counts even if subsequently restored/OFF. No deletion to make
DOWN pass. Test a second Owner invisible under the migration actor. Failure rolls back
all DDL/ACL changes; existing historical PENDING and SUCCEEDED rows/items must survive
both UP and an allowed DOWN unchanged.

After the guard: drop new daily triggers/functions/tables in FK order, remove daily
columns/constraints, restore exact pre-0060 0050 binding and 0052 success definitions,
restore historical pin NOT NULL checks after verifying all remaining rows historical.
Explicitly REVOKE the A3 INSERT/UPDATE **column** grants (all listed columns), then
GRANT INSERT, UPDATE ON user_strategy_configs TO app, restoring 0009's prior effective
ACL; SELECT/DELETE were retained. Remove only 0060-added privileges/functions, preserving
0049/0050/0051 existing column ACLs. The login role remains a provisioned but powerless
principal; migrations do not drop cluster roles. Restore FORCE RLS before commit.

Populated daily histories require schema0060 and a compatible reader even with mode OFF.
Add a controlled installer/rollback preflight: if daily histories exist, the target
immutable release must declare daily schema/input-reader compatibility; old manifest
without that capability fails before current-link switch. Existing rollback merely
switches the link (`scripts/ops/deploy-production-release.sh:272`). No destructive
downgrade or reinterpretation of daily rows as historical is allowed.

## A5. D3-05 — first-migration readiness and validation ownership

Overrides D1 I2/I6 ordering. Role/harness source readiness precedes **any first execution
of 0060**, including standalone full MIGRATOR test helpers. This is not an authorization
to execute migrations now. Bootstrap `daily_input_admitter` as direct LOGIN, NOINHERIT,
NOSUPERUSER, NOCREATEDB, NOCREATEROLE, NOREPLICATION, NOBYPASSRLS; CONNECT to the target
database and USAGE on public, never CREATE. No membership into or out of this role;
no app/worker path to migration_owner or admitter. Inspect pg_auth_members transitively
including SET-role paths, role attributes and PUBLIC grants. NOINHERIT alone does not
prevent SET ROLE. Unexpected existing attributes/membership abort the preflight rather
than silently accepting or revoking an unexplained grant. Existing bootstrap only
resets selected attributes (`deploy/db/bootstrap-roles.sh:96`), so this is an explicit
new fail-closed check, not a claim about current production roles.

Exact integration surfaces to amend before launching DB-dependent packages:

- `deploy/db/bootstrap-roles.sh`: role/password map, attributes/membership preflight,
  CONNECT/USAGE list (`:66`, `:106`); migration-role preflight must also run inside UP.
- `scripts/ops/provision-db-secrets.sh`, `deploy/secrets/provision-runtime-secrets.sh`,
  `deploy/secrets/runtime-static-check.sh`, `scripts/ops/validate-production-config.sh`:
  new `db_daily_input_admitter_password`; bootstrap projection uid/gid999 mode0400;
  `owner-beta-daily-admitter/db_daily_input_admitter_password` uid/gid10003 mode0440.
  Add validator required-secret classification and runtime_specs (`:828`). Producer
  uid10002 gets no secret; worker/API retain their current credentials. DB password
  never reaches process arguments/logs. Admitters connect directly as their own role.
- `deploy/compose/compose.yml` and `.env.example`: bootstrap secret mapping ready first;
  later runtime service maps admitter DB role/password-file, admitted RW tree and Raw,
  candidate/base RO mounts; producer network_mode none and candidate RW only. Admitter
  has internal DB connectivity without provider egress. UID isolation is tested, not
  inferred from service names. New services stay OFF until controlled release activation.
  If current Raw modes deny these UIDs, the controlled installer must explicitly
  provision read/traverse-only named-UID ACLs for10002/10003 on required Raw/evidence
  paths and locks; keep original ownership and prohibit group/other write. Do not
  silently weaken modes or grant write to make the new read-only seam work.
- `deploy/db/integration-validation/validate.sh`: extend bootstrap env (`:633`),
  run_role_psql mapping (`:658`), all generated secrets (`:1029`, `:1040`), direct-login/
  catalog role lists (`:936`, `:1099`), source-copy inventory and staged0060 path, actual
  role assertions, guarded DOWN stage and fixed no-SKIP test inventory (`:1315`).
  Existing official CI entry remains `.github/workflows/ci.yml:155`.
- `crates/job-queue/tests/common/mod.rs:33` and
  `crates/api-server/tests/common/mod.rs:48`: add the role/attributes/membership checks
  before the embedded full migrator, plus per-database CONNECT/USAGE and a direct-login
  admitter test pool. `tests/integration/migration-contract/role-bootstrap.sql` and
  `bootstrap.sql` need the same readiness. Synthetic credentials stay isolated fixtures.

Future official no-SKIP inventory adds migration0060 actual-role/rollback/races plus
job-queue `owner_beta_daily_schedule`, `owner_beta_daily_publish`,
`owner_beta_daily_recovery`, `owner_beta_daily_runtime`, and API `http_owner_beta_daily`.
Retain historical `owner_beta_publish`, `http_owner_beta_price_only`,
`http_owner_beta_access` and existing fixed tests. Use `cargo test --locked` in the
authorized isolated/hosted environment with DATABASE_URL and actual role connections;
missing URL/fixture SKIP is BLOCKED, not PASS. Pure market-data/factor/compute tests are
separate from DB evidence. Validate catalog ACLs, privilege-denied SQLSTATE, terminal
output tampering, current-vs-old attempts, two-connection revokes, second-Owner DOWN,
producer/admitter real UID write denials and old v2/v3 bytes.

API generation is a sequential I5 operation owning both
`apps/api-server/openapi.json` and `apps/api-server/generated/openapi.ts` as well as
`apps/api-server/scripts/openapi-spec.mjs`; the checker writes outputs
(`apps/api-server/scripts/openapi-check.mjs:33`, `:84`). They cannot be assigned to an
independent simultaneous worker. Diagrams are updated/rendered only after final real
edges exist: new role/table/function scopes, mounts/services and daily API. Evidence
comments must cite the implementation, never this proposal. D5 changes no diagrams.

## A6. D3-06 — deterministic read-only verification and durable outcomes

Overrides D1 §§3–6 Rust/SQL return semantics; D4 alone freezes all bytes/hash preimages.
The current pure wrappers allocate random batch IDs
(`crates/market-data/src/normalize.rs:381`, `:565`); copied calendar normalization calls
a persistence wrapper (`:1046`). Neither may be called by the daily verifier.

I1b exposes three read-only seams, with concrete module-local types (D4 values are
adapted without changing their encoding):

```rust
CommittedRawReader::snapshot(scope: ApprovedRawScope, limits: ScanLimits)
    -> Result<CommittedRawSnapshot, DailyEvidenceError>;
resolve_original(snapshot: &CommittedRawSnapshot, reference: &SourceFileReference)
    -> Result<ResolvedOriginal, DailyEvidenceError>;
verify_daily_sources(snapshot: &CommittedRawSnapshot, request: &DailyVerificationRequest)
    -> Result<VerifiedDailyEvidence, DailyEvidenceError>;
```

Snapshot owns exact complete manifest-line bytes plus verified metadata/immutable file
handles; it exposes interval discovery, not read_manifest's orphan reconciliation.
Existing `RawStore::read_committed_manifest` is read-only but returns parsed entries
without the exact line (`crates/market-data/src/storage.rs:1113`, `:1177`). Add a
read-only sibling seam retaining those lines and shared existing commit-lock semantics;
no mkdir/lock-file creation/reconciliation/commit. Its trusted Raw owner is an explicit
installer-verified UID, not the new reader's euid (current owner restriction at
`crates/market-data/src/storage.rs:403`). Check descriptor-relative no-follow opens,
stable lock inode, correct trusted owner, no group/other write, regular files and
hashes. Do not relax existing writer APIs or chown Raw to admitter/producer. Denied
read permission is an operational gap, not permission to fall back to unsafe opens.

ResolvedOriginal includes every verified copy hop and original retrieval precision;
bounded depth, duplicate/cycle rejection and exact committed lookup. Extract only
pure row parsing/validation from normalize.rs into callable internal helpers. Use D4's
content identity directly: no normalized batch UUID or RawEnvelope is needed. Daily
document identity is `(daily-raw-reconstruction-v1,kind,session_date,
source_evidence_sha256,document_sha256)`; hashes use D4's existing commitments. Neither
`BatchId::new()`, current time, caller-generated output UUID nor candidate ID participates.
Historical normalizer IDs/bytes and generic unsupported-action behavior remain unchanged.
Original retrieval may have lower precision than provider wire time; preserve the
recorded value and never invent lost per-file precision.

D4 intentionally leaves reconstructed document bytes opaque (its §5); freeze the
I1b reconstruction here, separate from D4's manifest/proof encoding. Each document is
compact UTF-8 JSON with recursively lexicographically sorted object keys, no whitespace
or trailing LF: fields `schema="owner-beta-daily-reconstructed-document-v1"`,
`normalizer_version="daily-raw-reconstruction-v1"`, `kind` (D4 literal),
`session_date` (D4 date), `source_evidence_sha256` (D4 commitment), `rows` (below).
Unknown document keys are rejected. This is an internal evidence document, never a
generic normalized Raw batch or data_batches record. It creates no new approval wire.

| Kind | Exact reconstructed rows |
| --- | --- |
| bars | Existing canonical_bar row objects, sorted instrument/date, exactly11; extract the pure mapper at `crates/market-data/src/normalize.rs:813` unchanged, including optional value handling. |
| reference | Exactly11 objects `{instrument,provider_symbol}` sorted instrument; instrument is canonical ETF ID and provider_symbol is the independently validated inquire-price `stck_shrn_iscd` (`crates/market-data/src/normalize.rs:862`). Names from bars are not fabricated into this document. |
| calendar | One object `{date,session_type,evidence_kind}` for the projected civil date; type/kind have A2's literals. No invented open/close timestamp. |
| corporate_actions | D4 CashDividendRow objects for the required extension, after A1 equivalent-chain selection, sorted by D4's cash-row order; empty when none. Validate all original noncash/non-ETF rows first; noncash ETF evidence blocks rather than becoming a row or factor. No adjustment, entitlement, ex-date or available_at is invented. |

All objects including reused cash/bar rows obey this internal document's recursive
key ordering; this does not change D4's own struct-key encoder. Keep all compared
same-kind FileKeys in its D4 DocumentCommitment; D4 evidence separately retains the
full source metadata/copy times and unselected action chains. Exact source/version
changes therefore affect identity even if numerical rows match. Golden fixtures pin
these bytes in I1b; future mapping changes require a new normalization version.

VerifiedDailyEvidence has private fields/constructor, consumed by the independent
admitter after reopening/copying files; it is not an approved artifact. Only admitter
storage+DB commit creates a handle usable by the daily loader. Producer cannot construct
approval, write admission directories, project calendar SQL or enqueue jobs.

Replace `(admission_sha256,inserted)` with:

```rust
enum DailyAdmissionOutcome {
    Admitted { admission_sha256: ContentHash },
    Replay { admission_sha256: ContentHash },
    Blocked { reason: DailyBlockCode, event_sha256: ContentHash },
}
```

SQL `admit_owner_beta_daily_input(p_receipt_id uuid,p_admission jsonb)` returns `(disposition text,
admission_sha256 text NULL, reason_code text NULL, event_sha256 text NULL)` with exact
ADMITTED/REPLAY/BLOCKED shape matching the enum. Expected data conflicts are results,
not exceptions. Under the common source/admission lock: unchanged admitted identity
returns REPLAY; new valid next frontier inserts ADMITTED; changed same-date source or
contradiction inserts the idempotent source/admission event and returns BLOCKED. Never
return an old admission as REPLAY for conflicting incoming evidence. DailyBlockCode
has precisely A2 source_events' six code literals. No provider/body/free-form SQL text.
The adapter commits BLOCKED like the other two, and validates receipt ownership/stage.

Add `owner_beta_daily_poll_receipts`: `receipt_id uuid PRIMARY KEY` DB-generated,
`stage text CHECK IN ('ADMISSION','SCHEDULE')`, `started_at timestamptz`,
`completed_at timestamptz NULL`, `result_code text NULL`,
`source_frontier_sha256 text NULL`, `admission_sha256 text NULL` FK,
`event_sha256 text NULL` FK to source_events; timestamps are DB-owned. New polls reserve
a receipt and the persistent next_poll_at>=start+60s in a short transaction before
scanning; stage advisory lock prevents overlap. Slot acquisition returns receipt_id,
not just bool. Slot crashes leave a started receipt, not VERIFIED. Completion is once
only; repeat identical completion is REPLAY, conflicting completion fails. Admission
outcome+conflict event+receipt completion+poll_state update share one transaction.
BLOCKED clears current verified admission, preserves historical rows and commits the
event link; a lost commit reply rereads the receipt/unique event key. Store failure
receipts for parse/IO/gaps with no purported admission, and never manufacture a conflict
hash from a raw error string. For source events, SHA256 preimage is UTF-8 newline-joined
values, with final LF: `owner-beta-daily-source-event-v1`, lowercase Owner UUID,
source_kind, code, range_start ISO date, range_end ISO date, source-observation hash. No
caller-chosen salt/time. This operational event key does not redefine a D4 artifact
hash. For multiple affected descendants, keep one source event and append each D1
admission-block event; its optional admission link names the earliest affected admission.
The source-observation hash is SHA256 of `owner-beta-daily-source-observation-v1\n`
plus a compact array of the observed D4 FileEvidence values ordered by full canonical
bytes (preserve conflicting variants; discard only byte-identical repeats). It needs
no normalized document, admission or approved manifest. Hash mismatch/missing-file
events can bind the expected committed FileEvidence being checked; a permanently
blocking event need not preserve a corrupt provider body. A malformed source with no
parseable identity yields only an INPUT_BLOCKED poll receipt, never a fabricated event.

Scheduler receipts have no source-event unless reporting a known event. Poll-state
freshness remains D1's 180 seconds. Admission exceptions/rollback are IO/integrity
failure, not REPLAY. Freshness is an observation boundary; an unobserved later Raw
append is not something SQL can promise to detect before the next allowed poll.

## A7. Evidence, followups and verification limits

Actual D5 checks: read AGENTS/roadmap/plan/D1/D3 and the primary source locations above;
read-only Git HEAD/status/diff inspection; SHA256 and line counts for frozen D1/D3;
report path/line citation checks and whitespace diff check. No runtime, compiler, test,
DB, container, provider, production, role-attribute or filesystem-UID behavior was
executed/verified. A source citation is evidence of code, not of production state.
Results: 41 explicit file:line citations and two relative report links resolve, no
out-of-range citations or trailing whitespace; both diff whitespace checks exit0.
Frozen D1/D3 hashes and line counts match the opening record. Product-directory diff
against619634d is empty. D5's sole changed file is this new report; the concurrently
created D4 contract is owned by its separate worker and was only read here.

Unresolved G4 inputs: actual committed Raw inventory/coverage and mutation history;
actual principals/membership/permissions; production rights and policy/base pins;
supplier activation/frontier, aggregate request budget/deadline, current official range
semantics and approval of the exact recurring acquisition delta. They do not block
pure implementation or isolated fixtures. D4 appeared during drafting; its §§3–7/10
were cross-checked for copy chains, intervals, surrounding-calendar bindings and opaque
normalized document commitments. A6 supplies the latter's reconstruction without
altering D4's schema. Bind the final D4 revision before I1b launch. Other required source
evidence not found: none. D2 remains the Web scope authority.

Deviations from D5 scope: none. Architectural changes from frozen D1 are exactly A1–A6,
including the added calendar/source/poll/publication authority tables and earlier
bootstrap readiness needed to close D3. No plan or frozen report was edited. Background
work started by D5: none. Permission waits: none. Native delegation: none. Memory uses
only the actual hook envelope; this proposal is not stored as an adopted product rule.

## A8. Replacement staged implementation assignments

These replace D1 §12 where noted; they are proposed briefs, not execution. Each package
is assignable Luna/max after its listed inputs are frozen; ambiguity is returned to the
coordinator, never resolved by weakening approval. No package may expand its allowlist.
Names below partition ownership; shared integration files have one named owner, with
explicit sequential additions after dependencies land. Further architecture decomposition
needed: none beyond binding D4's completed wire contract and D2's Web brief.

1. **I0 — role/harness readiness (before any0060 run).** Own
   `deploy/db/bootstrap-roles.sh`, `scripts/ops/provision-db-secrets.sh`,
   `deploy/secrets/provision-runtime-secrets.sh`, `deploy/secrets/runtime-static-check.sh`,
   `scripts/ops/validate-production-config.sh`, `deploy/compose/compose.yml`,
   `deploy/compose/.env.example`, `deploy/db/integration-validation/validate.sh`,
   `crates/job-queue/tests/common/mod.rs`, `crates/api-server/tests/common/mod.rs`,
   `tests/integration/migration-contract/role-bootstrap.sql`,
   `tests/integration/migration-contract/bootstrap.sql`. Implement A5's full maps,
   preflight and harness role pools first. I0 alone returns later to these same files
   to wire I2 SQL inventory/I4 OFF runtime services; do not give them simultaneously to
   I2/I6. Acceptance: complete static maps and later official actual-role/no-SKIP run;
   no first0060 attempt until role setup is demonstrably ready.
2. **I1a — D4 pure UNAPPROVED wire, unchanged coordinator scope.** Own only
   `crates/market-data/src/daily_price_only/mod.rs`, `contract.rs` in that directory,
   `crates/market-data/src/lib.rs`, `crates/market-data/tests/daily_price_only.rs`.
   Implement the frozen D4 signatures/goldens, no Raw/DB/approval/runtime. Registration
   changes needed by I1b return sequentially to I1a after I1b files exist; no missing-file
   stubs or concurrent mod.rs edits. D5 neither expands nor blocks this pure package.
3. **I1b — offline evidence/admission library.** After D4/I1a, own new
   `crates/market-data/src/daily_price_only/{source,verify,calendar,artifact,admission,supply}.rs`,
   existing `crates/market-data/src/storage.rs`, `crates/market-data/src/normalize.rs`,
   new `crates/market-data/tests/daily_price_only_storage.rs`,
   `crates/market-data/tests/daily_price_only_evidence.rs`,
   `configs/evidence/owner-beta-daily-price-policy-v1.json` (reviewed policy, no activation).
   Implement A1/A2/A6 plus D1 safe storage; supply.rs only plans, makes no calls. Fixtures:
   interval partition mismatch, Friday/Monday/holiday gap, complete7/77, equivalent and
   contradictory overlaps, copied calendar target/surrounding/future times, original
   target absent, deterministic repeat IDs, zero writes during verification, no-replace
   and UID/redirect mutation cases. No provider/collector edit or live supplier wiring.
4. **I2 — schema/transaction authority.** After I0 readiness and frozen D4/A1–A6
   interfaces, own `migrations/0060_owner_beta_daily_inputs.{up,down}.sql`,
   `tests/integration/migration-contract/tests/migration_contract.rs`,
   `deploy/db/integration-validation/{preflight-0060,actual-role-0060,rollback-0060-guard}.sql`.
   Implement D1 with all overrides here, including calendar projection, effective ACL,
   epoch, receipt, immediate/deferred daily guards and exact DOWN. No edits to old
   migrations or I0 files. Provide fixed function signatures and fixture test names to
   I0 for sequential harness wiring. Acceptance includes actual-role direct DML denial,
   terminal immutability, valid retry/cancel/recovery, two-connection races, same-date
   BLOCKED commit, lost reply convergence, all-tenant guarded DOWN and unchanged v2/v3.
5. **I3 — loader/factor/target.** Retain D1 I3's exact file allowlist and historical
   preservation tests (`docs/reviews/2026-10-14-r2-daily-input-design.md:825`). Depend on
   I1a/I1b; no wire redesign. Only admitted handles enter daily compute. Registration
   of runtime modules stays under I4's public owner_beta::runner boundary.
6. **I4 — local runtime and atomic queue integration.** Retain D1 I4's exact files
   (`docs/reviews/2026-10-14-r2-daily-input-design.md:846`); depend on I1b/I2/I3. Implement
   calendar-before-admission, committed three-way outcomes/receipts, persistent minute
   gates, latest-only reservation, publication lock order and all legitimate A4 recovery
   transitions. No queue.rs edit anticipated; source-only review must return a concrete
   dependency if one becomes necessary. Daily SQL gate failure must settle capacity.
   Add the four named no-SKIP test suites to I0's sequential inventory handoff.
7. **I5 — dedicated API/DTO/generation.** Retain D1 I5's file allowlist
   (`docs/reviews/2026-10-14-r2-daily-input-design.md:870`) and additionally own
   `apps/api-server/generated/openapi.ts`. Implement dedicated calendar/daily SQL reads,
   actual Owner opt-in, expected/support/as_of/published separation and stale/blocked
   receipts. Generic service_status calendar source scopes stay unchanged. Own both
   generated outputs sequentially; hand frozen DTOs to D2/Web, no parallel generation.
8. **I6 — controlled release and diagram integration.** Retain only these D1 I6 files:
   `deploy/compose/owner-beta-static-check.sh`,
   `scripts/ops/deploy-production-release.sh`, `scripts/ops/compose-release.sh`,
   `scripts/ops/lib/release-image-manifest.sh`, `crates/job-queue/Dockerfile.owner-beta-runner`,
   new `scripts/ops/owner-beta-daily-static-check.sh`,
   `docs/runbooks/owner-beta-daily-input.md`,
   `docs/diagrams/{component_architecture,runtime_deployment}.{puml,png}`.
   I0 owns Compose/secrets/validators; request its sequential integration after I4.
   Implement old-reader rollback refusal, immutable OFF service/image mappings, and
   evidence-bound diagrams. Prepare G4 recurring supplier delta from A1; do not wire
   its execute path until the separate acquisition gate is resolved. Real UID/egress,
   hosted tests and local diagram rendering occur only under later coordinator authority.
9. **I7 — coordinator integration/Web.** D2 supplies exact Web paths; coordinator alone
   owns roadmap/status/plan/execution, CI sequencing and acceptance. Preserve distinct
   source, isolated-check, runtime and Owner observation evidence. G4 actual operational
   approval is the final boundary; source review or passing fixtures does not replace it.


### Wave134 query-date clarification — 2026-10-21 KST

For PaidinSubscription only, the query/cell date is the strictly parsed
`sub_term_ft` (eight ASCII digits, valid date). GB1=1 is subscription-date
selection; the independent GB1=2 PaidinRecord chain remains mandatory and uses
record_date. Other classes keep record_date. Every original row still passes
record/right/rate/symbol validation before the universe filter. A target ETF
subscription in the extension is NonCashAction even when its record date is
older. No date is rewritten, no source is dropped to hide a conflict, and no
subscription date becomes an availability timestamp. This corrects the blanket
record_date statement above; complete seven-class coverage remains mandatory.

Primary source: [official KIS paid-in query contract](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_llm/domestic_stock/ksdinfo_paidin_capin/ksdinfo_paidin_capin.py)
and [response field mapping](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_llm/domestic_stock/ksdinfo_paidin_capin/chk_ksdinfo_paidin_capin.py).
The official sources were read only, never executed. Eight actual non-ETF rows
across Sep3/7/9/10/11 match this distinction. A changed read-only reconstruction
at15:40:01UTC verifies all five dates; the unchanged Raw manifest hashes remain
2009b5ea…/d85be240…. The Aug29..Oct20 inventory now has13verified civil days and
40missing. Calendar coverage Sep24..Oct6 remains absent; no eligibility or
admission is asserted. Four new regression functions plus33existing cases and
market-data all-target Clippy pass. The two diagnostic copy include-path build
failures and corrected build are retained; production sources were not involved.
