# R2 WP-4 D1 — cumulative daily ETF11 input and scheduling design

Date: 2026-10-14, Asia/Seoul. Inspected source baseline:
`619634db32526032aae07ee2024a10f69fc560c4`. **Architecture proposal for coordinator/D3
review; no implementation, deployment, runtime, compilation, or DB verification.**
The Owner adopted U2 on 2026-10-14; adoption is not pending. This document does not
independently adopt additional product behavior or activate anything.

Scope: R2-04/08/09, D1 only. Read the repository AGENTS, ROADMAP, adopted
[U2](2026-10-13-r2-daily-input-decision.md),
[implementation brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md),
[prior contract/recovery analysis](2026-10-13-r2-contract-and-recovery.md), and
ADR-0005/0006. Coordinator owns progress/adoption documents. D2 owns the Web map.
All `file:line` citations below refer to inspected source, not proposed files.

## 1. Recommended minimum boundary

Keep the historical reader, registries, five-pin wire format, manual HTTP submission,
factor/target hash preimages, and v2/v3 replay. Introduce **daily ETF11 v1**, with
its own immutable manifest and independent admission, and a tagged daily job payload.
Reuse `owner_beta_price_recommendation`, its protected queue lifecycle, result tables,
strategy equations, capacity lock, and current entitlement infrastructure. Extend their
guards with an explicit daily branch; do not masquerade daily pins as historical pins.

Use two file/DB-isolated processes for daily production and admission, using subcommands
of the existing Owner-beta runner image. Add the minute scheduler to the existing
Owner-beta runner. There is no new provider client, account binding, generic READY
publication, strategy subprocess, PAPER scheduler, or new workspace member.

This needs multiple implementation packages, specified in section 12. It does not need
further product adoption for the already-adopted U2 rules. Operational initial frontier,
policy/base release pins and collection budget remain G4/G5 inputs, not guessed values.

## 2. Source map and constraints that affect the design

| Boundary | Inspected evidence and implication |
| --- | --- |
| Current priority/authority | `docs/ROADMAP.md:12`, `docs/ROADMAP.md:264`, `docs/ROADMAP.md:278`, `docs/ROADMAP.md:282`; U2 `docs/reviews/2026-10-13-r2-daily-input-decision.md:24`, `docs/reviews/2026-10-13-r2-daily-input-decision.md:44`. Source completion is not three-day/Owner acceptance. |
| Existing rights | `docs/decisions/0005-kis-personal-use-entitlement.md:6`, `docs/decisions/0005-kis-personal-use-entitlement.md:20`; preserve reference/hash and private single-owner scope. `docs/decisions/0006-etf11-ten-year-historical-coverage.md:18`, `docs/decisions/0006-etf11-ten-year-historical-coverage.md:165` preserve old artifacts and exact replay. |
| Historical admission | `crates/market-data/src/historical_price_only_approval.rs:42`, `crates/market-data/src/historical_price_only_approval.rs:68`, `crates/market-data/src/historical_price_only_approval.rs:249`, `crates/market-data/src/historical_price_only_approval.rs:268`: active v3, exact five pins; registry hash selects v2/v3 replay. Do not change these constants/registries to extend dates. |
| Historical scope | `crates/market-data/src/range_to_canonical_v3_price.rs:23`; `configs/evidence/kis-historical-price-only-v3-approved-artifacts.json:1`: 2016-08-29..2026-08-28, 2,452 sessions, 26,972 bars. These are repository commitments, not verification of host files. |
| Artifact durability | `crates/market-data/src/historical_price_only_v3_artifact.rs:1071`, `crates/market-data/src/historical_price_only_v3_artifact.rs:1983`, `crates/market-data/src/historical_price_only_v3_artifact.rs:2111`: descriptor-safe reads, revalidation, no-replace publication. Reuse the discipline without broad refactoring of the historical implementation. |
| Raw lineage | `crates/market-data/src/storage.rs:36`, `crates/market-data/src/storage.rs:58`; `crates/market-data/src/contract.rs:297`: exact file hashes, request metadata, batch retrieval and `copied_from` original retrieval. Never replace copied-source retrieval with new batch time. |
| Daily raw-price contract | `crates/market-data/src/providers/kis.rs:223` requests `FID_ORG_ADJ_PRC=1`, one requested date, fixed ETF11. `crates/market-data/src/normalize.rs:716` parses bars; at `:750` it filters to target date after validating rows. Daily verifier must additionally check exact ETF11 row coverage and raw-price request semantics. |
| Actions | `crates/market-data/src/providers/kis.rs:82`, `crates/market-data/src/providers/kis.rs:289`, `crates/market-data/src/providers/kis.rs:1070`: seven logical action classes over six endpoint paths; paid-in has two GB1 classes. Daily fetch uses whole-market `SHT_CD=''`, not the historical 11×7 matrix. `crates/market-data/src/range_to_canonical_v3.rs:465` contains the historical cash-only validator. `crates/market-data/src/normalize.rs:1487` intentionally rejects unsupported dividend normalization. |
| Calendar | `crates/market-data/src/normalize.rs:1086` validates first-page rows; `crates/market-data/src/providers/kis.rs:132` documents snapshot reuse without another acquisition. `crates/api-server/src/repos/service_status.rs:260` binds calendar/version/Raw DB lineage. `crates/api-server/src/service_status.rs:3204` enforces bounded continuous coverage, 16:30 eligibility and 18:30 due time. A maximum date alone is insufficient proof. |
| Collector boundary | `data-pipelines/collectors/src/pipeline.rs:673`, `data-pipelines/collectors/src/worker.rs:1462`, `data-pipelines/collectors/src/worker.rs:2661`: collection/normalization/publication/recovery remain distinct. A Raw commit or generic dataset READY is not daily admission. |
| Factor/target coupling | `crates/factor-engine/src/price_only.rs:29`, `crates/factor-engine/src/price_only.rs:103`, `crates/factor-engine/src/bars.rs:221`; `crates/job-queue/src/owner_beta/target.rs:183`, `crates/job-queue/src/owner_beta/target.rs:325`: historical type and five pins are in constructors and hash preimages. Add daily output types; preserve historical bytes. |
| Input/snapshot | `crates/job-queue/src/owner_beta/input.rs:24`, `crates/job-queue/src/owner_beta/input.rs:157`, `crates/job-queue/src/owner_beta/input.rs:218`: immutable strategy snapshot, canonical hash, private constructors. `crates/job-queue/src/owner_beta/compute.rs:67` reopens the exact historical pins, not latest input. |
| Manual enqueue | `crates/api-server/src/repos/owner_beta.rs:321`: actor transaction, owner capacity lock, client-key replay, active config lock, exact config snapshot and atomic job/run inserts. Keep `owner-beta:price-only:v1:<client-key>` separate. Capacity lock is precisely `hashtextextended(owner_uuid::text,7919)` at `crates/api-server/src/repos/mod.rs:37`. |
| 0049 | `migrations/0049_owner_beta_price_recommendations.up.sql:4`, `:83`, `:128`, `:346`: dedicated runs/items, constrained grants/RLS, immutable queue lineage. Existing input-kind check permits only history; historical pin columns are NOT NULL. |
| 0050 | `migrations/0050_owner_beta_strategy_snapshots.up.sql:6`, `:47`, `:119`: old empty-table guard cannot be reused on populated histories; binding checks exact five top-level payload keys, five pin keys and strategy fields. |
| 0051/0052 | `migrations/0051_owner_beta_target_publication.up.sql:28`; `migrations/0052_owner_beta_strategy_config_lock.up.sql:11`: success needs factor+target+cash, exact active config is locked through success by a noncallable definer trigger. Preserve both guarantees. |
| Worker settlement/recovery | `crates/job-queue/src/owner_beta/runner.rs:157` exact-type claim, lease monitoring and bounded compute; `crates/job-queue/src/owner_beta/publish.rs:321` job-before-run locking; `:356` entitlement lock; `:380`..`:442` atomic items/run/queue settlement. `crates/job-queue/src/owner_beta/recovery.rs:131`, `:228`, `:267` protect stale claims and reconcile the same run. |
| Entitlement limit | `migrations/0035_recommendation_entitlement_lock.up.sql:8` locks user and qualifying entitlement rows, but checks data date, not Owner role or today's validity. Daily authorization must add those checks. Do not claim the helper alone proves current daily authority. |
| Config/identity mutation | `crates/api-server/src/repos/strategy_configs.rs:106` updates config JSON/active state, `:133` deletes. `migrations/0001_identity.up.sql:7`, `:25`: user identity and `user_roles`; the current Owner role value is `'owner'` (`migrations/0053_owner_managed_equity_universe_v2.up.sql:34`). |
| API read contract | `crates/api-server/src/http/owner_beta.rs:267`, `:387`, `:565`: historical support comes from common artifact dates; run DTOs expose true as_of. `crates/api-server/src/http/dto.rs:569`: existing pins are required historical strings. Add separate daily provenance rather than fake five pins. |
| Runtime isolation | `deploy/compose/compose.yml:1034` profile-gated runner, UID 10001 image, worker secret, historical leaf RO, internal DB-only network (`:1614`). Producer is `research_writer` (`:603`), with Raw/Curated writes (`:624`). Existing data-init owns Raw/Curated (`:490`), so admission must live outside those writable trees. |
| Provisioning/release | `deploy/db/bootstrap-roles.sh:66` fixed login list; `deploy/secrets/provision-runtime-secrets.sh:160` worker-secret projection. `scripts/ops/deploy-production-release.sh:2`, `:194`, `:233` immutable release/manifest and atomic current link. `scripts/ops/compose-release.sh:279`, `:721` control installed execution modes. Extend these controlled paths only. |
| Latest schema | `migrations/0059_research_execution_status.up.sql:1` is existing; reserve **0060_owner_beta_daily_inputs.{up,down}.sql**. Do not rewrite 0049..0059 or their checksums. |

The exact base pins remain:

| Pin | v3 value |
| --- | --- |
| candidate_content_sha256 | `sha256:b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d` |
| artifact_manifest_sha256 | `sha256:53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6` |
| stage5_manifest_sha256 | `sha256:5b2900594e03e322d4e81adaa79fc6c40ae70035213786972ed5cb7a22870d2a` |
| action_manifest_sha256 | `sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550` |
| approval_registry_sha256 | `sha256:5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707` |

Source for the complete set is the v3 registry above and
`docs/decisions/0006-etf11-ten-year-historical-coverage.md:155`. No active pin is
retargeted. The base approval means the base's existing evidence is accepted; daily
code must not manufacture historical per-day reference/calendars absent from that base.

## 3. Daily artifact, policy, admission and Rust interfaces

Names in this section are proposed exact interfaces. New market-data module:
`daily_price_only/{mod,contract,verify,artifact,admission}.rs`. All public errors are
finite value-free variants; never forward provider bodies, paths, keys or arbitrary messages.

```rust
pub struct DailyPricePolicyV1 { /* private validated fields */ }
pub struct DailyPriceCandidateV1 { /* private, NOT an approved input */ }
pub struct AdmittedDailyPriceInputV1 { /* private, verified immutable bytes */ }
pub struct DailyPricePinsV1 {
    pub content_sha256: ContentHash,
    pub manifest_sha256: ContentHash,
    pub admission_sha256: ContentHash,
    pub policy_sha256: ContentHash,
}
pub fn build_daily_price_candidate(
    base: &ApprovedHistoricalPriceOnlyArtifact,
    previous: Option<&AdmittedDailyPriceInputV1>,
    evidence: &DailySourceEvidenceV1,
    policy: &DailyPricePolicyV1,
) -> Result<DailyPriceCandidateV1, DailyPriceError>;
pub fn verify_daily_price_candidate(
    candidate: &OpenedDailyCandidateV1,
    base: &ApprovedHistoricalPriceOnlyArtifact,
    previous: Option<&AdmittedDailyPriceInputV1>,
    source: &ReadOnlyDailyEvidenceStore,
    policy: &DailyPricePolicyV1,
) -> Result<VerifiedDailyAdmissionV1, DailyPriceError>;
pub fn read_admitted_daily_price_input(
    admission_root: &Path, pins: &DailyPricePinsV1,
    policies: &TrustedDailyPolicyRegistry,
) -> Result<AdmittedDailyPriceInputV1, DailyPriceError>;
```

`AdmittedDailyPriceInputV1` has read-only `pins()`, `bars()`, `sessions()`,
`base_pins()`, `range_end()` accessors. Private constructors require verification;
no `Deserialize` into an approved type and no public constructor from a Vec of bars.
File verification does **not** attest current authorization/revocation: the worker
must also obtain the DB gate in sections 5–7. Offline replay can verify a retained
revoked artifact, but cannot publish it as currently authorized.

Use fixed ordered serde structs, sorted arrays, integer counts, decimal strings,
ISO dates and RFC3339 UTC instants. Reject unknown fields, duplicate JSON keys,
duplicate instruments/dates/source IDs, unsafe names and extra leaf files. Content hashes
are `sha256:<64 lowercase hex>`; code commits are 40 lowercase hex. Hash exact bytes,
not re-prettified JSON. Store no wall clock generation time in deterministic preimages.

`configs/evidence/owner-beta-daily-price-policy-v1.json` is a reviewed, release-pinned
policy artifact, not a mutable operator override. It fixes the full five-pin base,
ETF11 identity set, raw-price request semantics, cash-only treatment identifier,
schema/normalizer versions, allowed local source kinds, complete-coverage rules and
resource bounds. Its byte hash enters every candidate/admission/job/output. Initial
base/policy approval is recorded at the release gate; do not invent a policy hash now.
Runtime activation selects one exact compiled/trusted policy hash. Historical policy
bytes remain readable for replay after a future policy change.

The manifest is a new JSON shape, conceptually:

```json
{
  "schema": "owner-beta-daily-etf11-input-v1",
  "capability": "PRICE_RETURN_ONLY", "audience": "OWNER_ONLY",
  "vendor_snapshot": true, "strict_pit": false,
  "base": {"kind": "historical_price_only_v3", "pins": "<exact five-pin object>"},
  "parent_admission_sha256": null,
  "range_start": "2016-08-29", "range_end": "<new complete session>",
  "instruments": ["<exact sorted ETF11 instrument IDs>"],
  "sessions": ["<all base and added common sessions>"],
  "daily_sources": ["<one DailySessionEvidenceV1 per added session>"],
  "bars": {"file": "bars.ndjson", "sha256": "<hash>", "size_bytes": 0, "row_count": 0},
  "normalization": {"file": "normalization.json", "sha256": "<hash>", "size_bytes": 0},
  "policy_sha256": "<hash>", "materializer_version": "daily-price-only-v1",
  "code_commit": "<producer release>", "content_sha256": "<hash>"
}
```

The quoted angle-bracket values are schema notation, not fixtures. First parent is
null and must extend v3 by exactly its next proven trading session. Later parents
name the prior admitted daily input. Each candidate extends **one** session; an
offline catch-up can construct a chain, but never enqueue historical recommendations.
Every artifact contains the full cumulative bars and all daily lineage. This simple
copy-on-write representation avoids a mutable merged dataset/rolling policy. Reject
limits (manifest 16 MiB, bars 128 MiB, 20,000 sessions) explicitly; never truncate.
These are parser/resource guards, not permission to acquire that much data.

`content_sha256` hashes canonical manifest content excluding itself, including the
bars and normalization descriptors. `manifest_sha256` hashes final manifest bytes including that content
hash. `admission.json` contains schema `owner-beta-daily-etf11-admission-v1`, the
content/manifest/policy hashes, parent admission, exact base pins, verifier code commit,
normalizer version, evidence commitment and counts. Its hash is `admission_sha256`;
it contains no admission ID/time/self-hash. DB `admitted_at` records the first successful
admission transaction's timestamp; the row becomes visible only at commit. It is not
a PostgreSQL commit timestamp. Rerun changes neither it nor the immutable bytes.

`DailySessionEvidenceV1` has exact `session_date`, 11 identities, per-kind ordered
source-file commitments, normalized-document commitments, calendar civil-date coverage,
and action commitments. Each `DailyFileEvidenceV1` contains provider/market, source batch
ID/date, original batch.json hash, **exact manifest-line byte hash**, filename/kind,
wire-byte hash/size, endpoint/TR/query/continuation commitment, recorded retrieval instant,
and optional copied-from chain. Include the original source file identity and time
after resolving copied evidence; reject cycles or disagreement. Distinguish
`retrieval_precision="batch"` from copied-source provenance: existing FileEntry does
not record a separate per-request acquisition timestamp. Do not invent finer timestamps.
Reference quotes are retrieval-time vendor snapshots, including when bars are historical;
never label the quote as having been retrieved on the bar's date.

## 4. Verification and visibility algorithm

1. Reopen the approved v3 base by all five pins. For every next input, verify the
   parent admission and its immutable prefix. Preserve prior session rows, original
   source commitments and times byte-for-byte. Validate entire cumulative common
   session list, increasing and unique, with exactly 11 rows per trading session.
2. Independently enumerate the committed local Raw manifest records for the relevant
   dates/kinds. Read exact batch metadata and stored wire bytes with bounded, descriptor
   safe reads; verify hashes/counts/mode/entitlement reference. The producer's chosen
   list is not sufficient: another conflicting committed delivery must be detected.
   Fixture mode is accepted only by test seams, never production admission.
3. For daily bars check exact allowed request shape (`FID_ORG_ADJ_PRC=1`, target start=end,
   D period, fixed symbol), no pagination, JSON/rt_cd/schema/row validity, exact target
   observation and 11 distinct identities. Parse every returned row; any overlap with
   an already-sealed historical/daily date must agree, otherwise block correction.
   Do not import adjusted Curated closes. New `HistoricalPriceOnlyBar` representation
   may be reused as arithmetic data, but new rows' adjusted OHLC equal their raw OHLC
   because this admission policy admits no noncash adjustment. Validate that the v3
   prefix has its approved raw/adjusted semantics; never silently coerce it.
4. Independently reconstruct bars/reference/calendar normalization using the existing
   parsers and validation rules. Expose narrowly scoped pure helpers in `normalize.rs`
   for these three kinds; retain the generic four-kind normalizer's behavior. Compare
   any persisted normalized documents to the reconstructed exact lineage and rows.
   Produce daily-specific normalized evidence inside the candidate if generic action
   normalization blocked. Thus cash-only validation can succeed without weakening
   generic corporate-action/Curated publication. Admission is still based on complete
   Raw and independently reconstructed normalization, not a successful generic READY.
5. Require all seven logical KSD classes, complete bounded page chains, unchanged query
   continuation rules, no repeated page bytes, and valid fields even for non-target rows.
   For whole-market daily queries, validate every row before filtering exact ETF11 codes;
   target rows must have an unambiguous matching symbol and request-range date. For each
   ETF11 nonempty row, only fully validated cash-only dividends with zero stock dividend
   rate and no stock-payment contradiction may be excluded. Record count/hash/source/time,
   never a total-return adjustment or inferred ex-date. Reject all ETF11 noncash actions,
   including the generic normalizer's otherwise representable bonus issue. Empty complete
   classes prove absence; missing files/pages never do. Do not demand the historical
   77-file matrix from the daily whole-market query, or broaden the provider request.
6. Require verified calendar rows for every civil date between the prior frontier and
   target. TRADING dates must equal the added session sequence; CLOSED dates have no
   price rows. Bind each row to its actual first-page Raw source, copied-source chain
   if reused, normalized commitment and DB calendar/version/source binding where present.
   Reject calendar disagreement, source mismatch, or nonfuture-time violations. A dates-only
   XKRX operational schedule does not replace credentialed calendar proof. Do not apply
   intraday's 36-hour evidence limit to historical EOD. For action coverage see the
   unresolved closed-day evidence point in section 11; it must fail closed, not assume
   that CLOSED implies no corporate action.
7. Candidate ID/content must converge for the same bytes, policy and release. No
   `now()`, random UUID, traversal order, mtime or latest-pointer value in that identity.
   A changed producer/verifier release is explicit lineage; reuse an existing admitted
   date with unchanged sources instead of re-admitting it under the new release.
8. Separate verifier repeats steps 1–6 using original Raw/manifest evidence and its own
   reads, compares its reconstructed content to the candidate, copies validated bytes
   to its own admission staging directory, fsyncs files/directories and uses no-replace
   atomic rename. **Never hardlink candidate files**: a producer retaining a writable
   descriptor must not be able to mutate approved bytes. Reopen/hash the final leaf.
9. Only after durable leaf verification, commit the DB admission through its guarded
   function. Consumers require both matching DB admission and immutable leaf. A crash
   before DB commit leaves an invisible recoverable orphan; a lost commit reply is
   resolved by exact admission hash lookup. A DB row with absent/damaged files fails
   closed. No cleanup removes admitted evidence, even after revocation.

Same-date conflict is a permanent **automatic-path block**, not last-writer-wins.
An exact same source/bytes retry is a no-op. A different source set for an admitted
session is not automatically a harmless refresh, even if rounded prices match.
Append a conflict event under the admission fence; block that input and descendants.
No revised daily admission or second recommendation for that date is automatically
created. Human-reviewed correction/revision is a later explicit promotion procedure.
Validation cannot detect a broker correction never retrieved; disclose that limit.

## 5. Principal, storage and DB model

Proposed host tree, outside both Raw/Curated and the installed immutable release:

```text
<ARTIFACTS>/owner-beta-daily/candidates/<content-hash>/manifest.json,bars.ndjson,normalization.json
<ARTIFACTS>/owner-beta-daily/admitted/<admission-hash>/manifest.json,bars.ndjson,normalization.json,admission.json
<ARTIFACTS>/owner-beta-daily/producer-state/       # locks/poll checkpoint only
<ARTIFACTS>/owner-beta-daily/admitter-state/       # locks/poll checkpoint only
```

Policy bytes are read-only release content. All artifact leaf names are derived from
validated hashes, not payload paths. Root-owned parents prevent replacement of a
whole admission root. Separate trusted UID 10003 owns admission/state, mode 0750;
files 0440, reader group 10001. Candidate producer UID 10002 owns candidate/state;
Raw collector UID 10001 cannot write those roots. No group write, writable ancestor,
symlink, host Docker socket, CAP_DAC_OVERRIDE, or alternate writable admission bind.
Check UID/GID availability in the installer; collisions fail rather than reuse an
unrelated principal. These numeric IDs are a proposal, not observed host assignments.

| Process/principal | Files | DB/network |
| --- | --- | --- |
| Existing collection, `research_writer`/10001 | Existing Raw/Curated writes; no daily roots mounted | Existing allowed acquisition only; no admission/function privilege |
| `owner-beta-daily-producer`, 10002 | Raw and historical base RO; candidates/state RW; admitted parent RO | `network_mode: none`; no DB/provider secrets |
| `owner-beta-daily-admitter`, 10003 | Raw, historical, candidates RO; admitted/state RW | New login `daily_input_admitter`, no inheritance/membership in other runtime roles; internal `owner-beta-db` only; no KIS secret |
| Existing runner, worker/10001 | Historical+admitted RO, no Raw/candidate mount | Current protected queue privileges; narrow schedule/gate functions; cannot admit/revoke policy or write config |
| API, app/10001 | Existing historical RO; no new Raw/admission write | Owner-scoped daily metadata/read projection and opt-in update; cannot admit, schedule automatically, or mutate daily lineage |
| Installer/migration authority | Protected policy/control provision and file ownership | Explicit release control; not a serving credential; never distributed to the above processes |

New login is provisioned by existing bootstrap/secrets machinery, **not CREATE ROLE
inside a migration owned by non-CREATEROLE `migration_owner`**. Migration requires
the role to exist, owns objects as migration_owner, sets safe search_path and revokes
PUBLIC EXECUTE in the same transaction. New role is NOSUPERUSER/NOCREATEROLE/
NOCREATEDB/NOBYPASSRLS, not a member of worker/research_writer/app/admin.

0060 creates these tables (all FKs ON DELETE RESTRICT, no cascading erasure):

| Table | Exact minimum columns/keys and meaning |
| --- | --- |
| `owner_beta_daily_control` | singleton `id='etf11'`, `owner_user_id uuid`, `enabled boolean DEFAULT false`, `activation_epoch bigint`, `policy_sha256 text`, `entitlement_reference text`, `entitlement_sha256 text`, `first_auto_session date`, `trigger_version text CHECK ='owner-beta-daily-trigger-v1'`, `max_jobs_per_owner int`, `updated_at timestamptz`. No row or false means OFF. Gate values provisioned by controlled release; max capacity must equal current API capacity config. |
| `owner_beta_daily_admissions` | `admission_sha256 text PRIMARY KEY`, owner, policy, content and manifest hashes, `parent_admission_sha256 text NULL` self-FK, `range_start date`, `range_end date`, `session_count int`, `bar_count int`, `evidence_json jsonb`, `admitted_at timestamptz DEFAULT clock_timestamp()`. Unique `(owner_user_id,range_end)` permanently rejects automatic same-date revisions across policy changes. No update/delete grants; immutable trigger. Exact hash/shape constraints. |
| `owner_beta_daily_admission_events` | `event_sha256 text PRIMARY KEY`, `admission_sha256` FK, owner, `kind CHECK IN ('CONFLICT','REVOKED')`, `evidence_sha256 text`, `created_at timestamptz`. Append-only; deterministic event identity over kind/admission/evidence. Event on an ancestor blocks descendants; never delete/revoke the historical v3 registry by this mechanism. |
| `owner_beta_daily_reservations` | owner, config ID, `session_date date`, `trigger_version text`, `run_id uuid UNIQUE`, `job_id uuid UNIQUE`, `admission_sha256` FK, `strategy_config_sha256 text`, `config_epoch bigint`, `activation_epoch bigint`, `created_at`. **PK `(owner_user_id,strategy_config_id,session_date,trigger_version)`**. Composite binding to run/owner/job/config/date is validated in a deferred constraint trigger. Never TTL/delete; status comes from linked run/job. |
| `owner_beta_daily_poll_state` | `stage CHECK IN ('admission','schedule') PRIMARY KEY`, `next_poll_at timestamptz`; nullable `last_completed_at`, `last_result_code`, `verified_admission_sha256` FK and `source_frontier_sha256`. Guarded compare/update of DB time, committed before scanning, provides one local-state cycle per stage per 60 seconds across replicas/restarts. Admission-stage completion records bounded health/evidence observation, not a new immutable admission. Empty/failed acquisition does not grant a poll. Producer has equivalent flock+fsync file checkpoint because it has no DB. |

Add `owner_beta_daily_enabled boolean NOT NULL DEFAULT false` and
`owner_beta_daily_epoch bigint NOT NULL DEFAULT 0` to `user_strategy_configs`.
BEFORE UPDATE increments epoch on actual changes to strategy ID/version/config JSON,
is_active, or daily_enabled. Client cannot set epoch. Reject negative/overflow. No-op
updates do not increment it. OFF→ON creates no new config ID; dedup survives all toggles.
The INSERT/UPDATE guard additionally requires an Owner role and actor/config-owner
equality for an explicit opt-in; generic app config privileges must not let a Member
enable it by direct DML. Existing non-Owner configs with default false remain valid.
Disable/delete uses normal Owner-authenticated config authority. A referenced config
cannot be deleted because its existing run/history FK already restricts deletion.
Control has its own BEFORE UPDATE epoch trigger: changes to enabled, owner, policy,
entitlement pins, first_auto_session, trigger version or capacity increment
activation_epoch. Neither caller nor a disable can reset it. A prior activated epoch
therefore remains visible to the guarded DOWN even after mode is OFF.

Extend existing run rows with `trigger_kind text NOT NULL DEFAULT 'MANUAL'`,
`daily_pins jsonb NULL`, `daily_config_epoch bigint NULL`,
`daily_activation_epoch bigint NULL`, `daily_trigger_version text NULL`.
Make the five historical pin columns nullable **only** with a replacement discriminated
CHECK: historical input_kind + MANUAL requires all five, no daily fields; daily
input_kind + DAILY requires all daily fields, all historical pins NULL. Historical
default stays unchanged. Exact daily pins keys/types are checked and admission-bound.
Keep the existing status/capability/audience/PIT/success-output constraints intact.
Add permanent reservation FK/constraint checks; keep results in the same items table.

Every new table has ENABLE+FORCE RLS. App SELECT sees only `app.actor_user_id` and
current Owner-only scope; it cannot SELECT another owner's state by ID. Worker may
read scheduling/admission/reservation state under a dedicated worker policy but has
no direct INSERT/UPDATE/DELETE on any of it. `daily_input_admitter` gets only bounded
source metadata via the function below and EXECUTE on admission/poll functions, no jobs/config/results
DML. App gets only the daily_enabled **column** update addition (existing config
authority retained), no epoch write. Migration-owner policies explicitly cover its
definer functions under FORCE RLS. No table-level grant may accidentally override a
column restriction. Do not add ambient tenant GUC impersonation to the scheduler.

## 6. SQL service interfaces, transaction order and races

Concrete proposed SQL entry points; functions derive authoritative state, rather than
trust producer/client flags. Return typed status codes, never arbitrary DB/provider text.

```sql
-- daily_input_admitter only; repeated identical admission returns the first row
admit_owner_beta_daily_input(p_admission jsonb)
  RETURNS TABLE(admission_sha256 text, inserted boolean);
-- daily_input_admitter only, for independently detected conflict
block_owner_beta_daily_input(p_admission_sha256 text, p_evidence_sha256 text)
  RETURNS boolean;
-- migration/release authority only, explicit revocation (no API auto-correction)
revoke_owner_beta_daily_input(p_admission_sha256 text, p_evidence_sha256 text)
  RETURNS boolean;
-- separate grants prevent a definer current_user/session_user ambiguity
take_owner_beta_daily_admission_poll_slot() RETURNS boolean; -- admitter only
take_owner_beta_daily_schedule_poll_slot() RETURNS boolean;  -- worker only
-- admitter only; range <= 370 civil days; exact configured Owner/policy scope
read_owner_beta_daily_source_evidence(p_start date, p_end date)
  RETURNS TABLE(evidence jsonb);
-- admitter only; DB owns completion time; no free-form error text
finish_owner_beta_daily_admission_poll(p_result_code text,
  p_verified_admission_sha256 text, p_source_frontier_sha256 text) RETURNS void;
-- worker only, no caller-chosen owner/date/config/input/limit
owner_beta_daily_schedule_candidates()
  RETURNS TABLE(strategy_config_id uuid, expected_session date,
    admission_sha256 text, config_epoch bigint, strategy_id text,
    strategy_version text, config_json jsonb);
-- worker only; rechecks observed values under locks before inserting
reserve_owner_beta_daily_run(p_config_id uuid, p_expected_session date,
  p_admission_sha256 text, p_config_epoch bigint, p_strategy jsonb)
  RETURNS TABLE(run_id uuid, job_id uuid, disposition text);
-- worker only: run identity derives tenant/input/config, never arbitrary actor
lock_owner_beta_daily_run_gate(p_run_id uuid) RETURNS boolean;
```

`read_owner_beta_daily_source_evidence` returns only the necessary columns from
`data_batches`, `trading_calendars`, `trading_calendar_versions` and the configured
entitlement. Filter KRX/KR, credentialed EOD Bars/Reference/CorporateActions/Calendar,
exact target interval, and selected entitlement; never return provider bodies, user
identity details or another source family. No broad table SELECT grant is required.
It does not claim a normalized `data_batches` row exists when generic normalization
failed: the verifier's Raw replay and daily normalization commitment are explicit
separate evidence. For calendar eligibility, require the existing three-way binding;
missing projected calendar rows block scheduling until the existing controlled supply
path supplies them. Do not synthesize that DB projection inside admission.
Only fixed completion codes VERIFIED, INPUT_PENDING, INPUT_BLOCKED and IO_FAILED
are accepted by `finish_*`. VERIFIED requires an existing matching policy/admission;
the other codes cannot carry a supposedly verified input. The scheduler requires a
VERIFIED observation for the selected admission no older than 180 seconds, in addition
to all authoritative DB/input gates. This is a liveness/evidence check, not a guarantee
that no new source can arrive after the scan. API reads this scoped observation and
reports unknown/blocked when it is absent, expired or failed.

All definer functions have exact PUBLIC/runtime revocations, migration_owner ownership,
fixed `search_path=pg_catalog,pg_temp`, schema-qualified names, bounded statement/lock
timeouts, and no dynamic SQL. Current role membership and config ownership are required
inside SQL, not just in HTTP. Existing 0052 success trigger gains a daily branch calling
the same internal gate; the gate implementation is not an unrestricted cross-tenant
writer. A scoped actor GUC needed for existing RLS is derived from the immutable row
and restored before return, including exceptional paths; it is not a caller parameter.

`admit_*` locks control FOR UPDATE, checks selected policy/base, existing parent and
absence of ancestor events, exact one-session extension and current scope/entitlement.
DB cannot inspect files: only the isolated verifier can assert successful file validation.
It compares all persisted descriptors on conflict, not just ON CONFLICT DO NOTHING.
First-write timestamp is DB-owned. Existing same-date different admission atomically
records a conflict and returns BLOCKED without replacing the admission. The verifier
also checks **known earlier date conflicts before admitting a later date**.

`block_*` and `revoke_*` acquire control FOR UPDATE before appending events, including
descendant effects. Publication/enqueue hold control FOR SHARE. This gives a single
global daily-input fence and avoids locking an unbounded chain in different orders.
An append wins before publication => no publication; publication commits first =>
subsequent reads show the retained result with revoked/conflicting input status.
Revocation does not erase a result that was valid at its publication linearization point.

Reservation transaction:

1. Derive owner from the single enabled control row; acquire the exact existing capacity
   advisory lock `pg_advisory_xact_lock(hashtextextended(owner::text,7919))`.
2. Probe the permanent reservation by owner/config/day/trigger version. If present,
   return that run/job regardless of later config/input changes; **do not recompute its
   identity** or produce a new job. For outward status, still enforce current read access.
3. Lock control FOR SHARE, then user, Owner-role row, exact entitlement row(s), config
   FOR SHARE, and admitted input. Require enabled controls, current Owner, active config,
   explicit daily opt-in, same epochs/policy, complete/unrevoked input and current
   entitlement reference+hash. Entitlement must cover the entire cumulative data range
   from 2016-08-29 through as_of, recommendation use/dataset, and be ACTIVE/valid on the
   current KST date; lock through commit. Do not authorize with any unrelated ACTIVE row.
4. Derive DB clock KST cutoff and latest expected trading session with continuous
   credentialed calendar/source proof. Recheck `p_expected_session` equals it and is
   >= control.first_auto_session. Choose admission **exactly ending at that session**,
   not the maximum available older input and not an artifact with future bars. Check
   p_admission and all four pins against that row. Empty/missing/gapped calendar or
   latest unadmitted session returns BLOCKED; it does not fall back to historical v3.
5. Require `p_strategy` exact four-key shape of OwnerBetaStrategySnapshot; ID/version/
   config JSON must equal the locked config. Use existing Rust canonical strategy hash
   constructor; payload and run carry exactly its hash. SQL compares the supplied hash
   and snapshot binding as 0050 does; the worker recomputes that canonical hash before
   any result. Do not substitute `jsonb::text` hashing (different byte representation).
6. Count all Owner QUEUED/RUNNING jobs under the shared capacity lock using current
   deployed API ceiling. Capacity deferral inserts no reservation and consumes no retry
   attempt. Allocate job/run UUIDs only for the winning insert. Construct payload inside
   this transaction from locked values, insert job then run then reservation. A deferred
   constraint trigger requires the daily run's reservation at commit. Any failure rolls
   back all three. A conflicting reservation returns the existing identity after retry.

The deferred daily consistency trigger also rejects a committed SUCCEEDED run whose
job/attempt did not settle successfully, whose item membership is not exact ETF11, or
whose mandatory factor/target/cash fields are missing. It validates both directions
of job/run/reservation linkage on relevant changes. Calculation correctness remains
the trusted worker's deterministic validation responsibility, not a claim that SQL
recomputes factor arithmetic. No source/admission principal receives result-write rights.

Queue key is `owner-beta:daily:v1:<owner UUID>:<config UUID>:<YYYY-MM-DD>`.
It is derived from the permanent four-field PK, **not** config hash, input hash,
activation epoch, release commit or attempt number. Those are sealed values, not
dedup dimensions. Trigger version cannot be bumped by a deploy just to rerun a day.
Status FAILED/CANCELED/SUCCEEDED never deletes the reservation. `max_attempts=3`
uses the existing job retry mechanism; no scheduler-generated replacement jobs.

Publication keeps existing queue claim/attempt → run row lock order, then acquires
control → user/Owner role → entitlement → config → admission gate locks. Check
lock-claim attempt/worker/lease/payload equality, sealed strategy and daily factor/target
hashes; current config JSON/ID/version and **epoch**; current activation epoch;
opt-in and active flags; current exact entitlement; and no admission/ancestor event.
Also require as_of still equals the latest expected eligible session: an older
unpublished daily claim is canceled as `DAILY_SESSION_SUPERSEDED` once a newer
session is eligible, preventing a delayed recommendation backlog. All item inserts,
hashes, cash, finished_at and queue success commit together. `published_at` is that
successful run's finished_at, captured with clock_timestamp() at the daily success
transition and visible only after commit; it is not an exact DB commit timestamp.
Historical finished_at values retain their original meaning and bytes. Evaluate
`clock_timestamp()` after all gate locks and again at the success transition, so a wait
across midnight/cutoff cannot reuse stale wall-date authorization. Keep the transaction
bounded; authorization is ordered at that guarded success transition.

Mutation operations must not synchronously take a job lock while holding config,
entitlement or control locks. Their guarded updates commit first; the runner observes
the fence and settles the existing job. Avoid a reverse config→job lock edge against
publication's job→config order. On deadlock/serialization timeout, roll back and retry
the **same** reservation/job through a bounded classified retry, never reseal.

| Race | Required outcome |
| --- | --- |
| Two schedulers / restart after lost enqueue reply | Owner lock + PK + three-row transaction yield one reservation/job/run and one snapshot. |
| Config edit during scheduling | FOR SHARE orders edit vs snapshot; stale candidate returns retry/no insertion. |
| Edit after enqueue, even edit-and-restore | Epoch differs; old automatic run cannot publish. Daily dedup remains consumed. |
| OFF→ON after enqueue | Config epoch changes; old run stays ineligible; tomorrow can use new opt-in. |
| Global disable then enable | Control activation_epoch changes on either transition; sealed old epoch rejected. |
| Owner role removed or entitlement revoked | Locked authorization check blocks new reservation/publication; remove/revoke ordered against commit. Reads recheck authorization too. |
| Admission revoked/conflicting during compute | Control shared/exclusive fence prevents publication after event commit. No reread of latest input substitutes for sealed pins. |
| Lease loss / old worker returns | Existing attempt fencing rejects stale output; dedicated recovery retries the same job/run without payload mutation. |
| New session eligible during an old retry | Old unsucceeded daily run is canceled/superseded; next minute can reserve latest session, within capacity. No historical run scan/enqueue. |

## 7. Loader, factor, target and worker transition

Do not replace `OwnerBetaPriceRecommendationInput`. Add a decoder wrapper:

```rust
pub enum OwnerBetaExecutionInput {
    Historical(OwnerBetaPriceRecommendationInput),
    Daily(OwnerBetaDailyRecommendationInputV1),
}
// Explicit dispatch: absent schema => legacy exact five keys only;
// schema == "owner-beta-daily-recommendation-v1" => exact daily shape;
// unknown schema or mixed keys => reject. Never "try daily then fallback".
```

Daily JSON has exactly `schema,run_id,strategy_config_id,as_of,input,strategy,trigger`:

```json
{
  "schema": "owner-beta-daily-recommendation-v1",
  "run_id": "<uuid>", "strategy_config_id": "<uuid>", "as_of": "<date>",
  "input": {"kind": "owner_beta_daily_etf11_price_only_v1",
    "content_sha256": "<hash>", "manifest_sha256": "<hash>",
    "admission_sha256": "<hash>", "policy_sha256": "<hash>"},
  "strategy": {"strategy_id": "<id>", "strategy_version": "<version>",
    "config_json": {}, "config_sha256": "<existing canonical strategy hash>"},
  "trigger": {"kind": "DAILY", "version": "owner-beta-daily-trigger-v1",
    "config_epoch": 0, "activation_epoch": 0}
}
```

The common wrapper exposes run/config/as_of/strategy accessors plus typed provenance.
It serializes historical variants as the **original untagged JSON**, daily variants as
the exact tagged object. Extend 0050 binding function in 0060 with these two explicit
branches, retaining the historical comparison code/semantics. Preserve 0049 job
lineage trigger protection for the existing job type. Trigger kind/input kind/provenance,
epochs/snapshot become immutable and included in UPDATE trigger column lists.

Add `DailyPriceOnlyFactorSnapshot` and builder requiring `&AdmittedDailyPriceInputV1`;
new `PriceOnlyBars::from_admitted_daily` passes the verified bars through the same
ETF11/no-forward-fill/as_of checks. Extract only the pure row-factor computation from
`price_only.rs:181` behind a crate-private helper returning rows/versions/normalization.
Keep historical `canonical_bytes()` exact. Daily factor canonical preimage contains
`schema="owner-beta-daily-factor-snapshot-v1"`, input_kind/capability/as_of, all four
daily pins, versions/normalization/rows; historical five pins never stand in for daily.

Similarly add `OwnerBetaDailyTargetSnapshotV1` with
`schema="owner-beta-daily-target-snapshot-v1"`, daily pins, strategy hash, factor hash,
as_of, existing canonical items/cash/reasons. Reuse only the pure strategy arithmetic
and fixed-point/rounding helpers from `target.rs`, with a validated internal factor
view. Keep the historical target hash preimage at `target.rs:325` byte-identical.
No daily public builder accepts invented historical pins to reuse a constructor.

Runner branches after strict decode: historical calls the existing compute function;
daily rechecks DB gate, reopens **sealed admission** by hash, validates base/policy and
all immutable files, computes with the sealed strategy, then uses daily publication
gate inside the settlement transaction. A serializable daily factor/target does not
confer approval by itself. Retained artifacts/policies permit deterministic offline
replay; live publication additionally needs current state. Recovery is schema-agnostic
for basic status mirrors but must validate matching daily columns/reservation and must
never rewrite lineage, source kind or epochs when requeuing. Generic queue sweeping
continues excluding the protected existing job type.

## 8. Polling and no-backlog policy

Add `PRICE_BETA_DAILY_MODE=disabled|owner_only` default disabled; enabling requires
an enabled DB control row with the exact policy/Owner/entitlement/activation pins and
normal Owner per-strategy opt-in. The process mode is an upper bound, never authority
by itself. DB control OFF is the shared publication kill switch; setting a new process
environment alone cannot revoke an already-running old process.

Producer and independent verifier poll only committed local files/DB metadata, at
most once per 60 seconds with persistent poll checkpoints and one active cycle lock.
If a cycle lasts longer than a minute, the next begins at least 60 seconds after the
previous start and only after it ends; no timer catch-up burst. Hold a stage-specific
advisory session lock on a dedicated DB connection for the entire admit/schedule cycle,
release it on all exits before returning the connection; producer uses flock. Acquiring
the persistent poll slot alone is insufficient to exclude a still-running long cycle.
On checkpoint corruption
or clock rollback, fail closed. Use one repeatable bounded local manifest scan per cycle,
recording its source frontier. New source records arriving during verification wait for
the next cycle; record the verified frontier and process discovered conflicts before
new reservation. This is bounded detection, not a claim of omniscience over future Raw.

Scheduler uses its own committed DB minute slot and reuses the runner process. Keep
existing 2-second queue consumer polling, 10-second lease heartbeats and dedicated
recovery cadence: those are queue lifecycle, not daily input/resource polling
(`deploy/compose/compose.yml:1068`). Do not call provider APIs on any of these paths.

`expected_session` is the latest credentialed, continuously covered trading date at
the DB KST cutoff: before 16:30 use prior civil-day cutoff; at 16:30 today is eligible.
Weekends/closures retain the latest proven trading session. With an admitted older
input and a newer expected session, wait/show stale; do not enqueue the older input.
At activation select only that latest expected session and require it >= the G4/G5
first_auto_session. Cumulative **data** can be repaired in order; past **recommendations**
are never iterated. Retries before supersession belong to the original reservation.
U2's 18:30 display deadline remains separate from eligibility and is not extended by
retries or late admission. Missing one required daily execution resets the three-day
acceptance observation; neither manual results nor catch-up artifacts replace it.

## 9. API DTO, source scopes and display

Keep the manual POST and historical `supported-as-of` route behavior unchanged;
callers asking for manual history still get the exact v3 support set. Daily state is
a new Owner-only route under the existing admitted recommendation prefix:
`GET /api/v1/recommendations/owner-beta/price-only/daily-state?strategy_config_id=<uuid>`.
No HTTP endpoint accepts arbitrary admission/policy/source pins or schedules daily runs.

Expose `owner_beta_daily_enabled` and read-only `owner_beta_daily_epoch` in existing
StrategyConfigDto. Create body optional daily_enabled defaults false. Update body
optional daily_enabled: absence preserves old value; explicit false disables. Only
an authenticated Owner can enable; CSRF and existing idempotency/audit rules apply.
Other users cannot enable by sending that field to the generic strategy-config route.
Normal config edits still increment the DB epoch even if this field was omitted.
Do not change config_json/strategy schema merely to store a scheduling preference.

```rust
pub struct OwnerBetaDailyStateDto {
    pub enabled: bool,                    // per-config opt-in
    pub runtime_enabled: bool,            // release mode AND DB control
    pub expected_session: Option<NaiveDate>,
    pub data_support_end: Option<NaiveDate>, // latest currently eligible admitted daily input
    pub recommendation_as_of: Option<NaiveDate>,
    pub published_at: Option<DateTime<Utc>>,
    pub input_kind: Option<String>,
    pub trigger_kind: Option<String>,     // DAILY or MANUAL, actual displayed result
    pub current_reservation_run_id: Option<Uuid>,
    pub state: DailyStateCode,
    pub reason_code: Option<DailyReasonCode>,
    pub data_due_at: Option<DateTime<Utc>>, // expected date 18:30 KST
    pub observed_at: DateTime<Utc>,
}
```

Daily state codes: DISABLED, UNVERIFIABLE, INPUT_PENDING, INPUT_BLOCKED, QUEUED,
RUNNING, RETRYING, FAILED, CURRENT, STALE. State is not a replacement for the independent
four dates. A fresh input with an old recommendation remains old. Distinguish no calendar,
no admission, gap/conflict/revocation, unsupported action, capacity and job failure via
finite reason codes. A manual result may be displayed with MANUAL provenance but never
sets DAILY CURRENT or the daily-observation success metric. At/after the applicable
18:30 deadline an absent successful daily result is STALE unless a more specific
blocked/failed state applies; retain its as_of and failure reason together.

Existing run list/detail gain `trigger_kind` and a discriminated `input` projection.
For history keep all current historical pin fields/values, with `input.kind=historical`
and those same pins; for daily omit historical pin fields and return `input.kind=daily`
plus four daily pins and base five pins nested as `base`. Make the historical fields
optional in shared DTOs, required only for the history OpenAPI union branch. Preserve
old historical outputs and record hashes; do not serialize null historical impostors.
Add `published_at` only for SUCCEEDED, equal to finished_at; pending/error runs have
no output items. Keep current Owner-scoped pagination, detail authorization and
integrity failures when tags/row/payload disagree.

SQL source scopes are explicit: daily-state selects daily admission/control/reservations,
the requested Owner's config and recommendation runs, plus credentialed calendar and
entitlement evidence. It does not use generic dataset READY, intraday quotes, equity-V2
admissions, Paper or latest unrelated user jobs. All recommendation reads require
current Owner/entitlement; API returns no provider body, Raw path or filesystem access.
OpenAPI route inventory and owner-beta middleware classification must include the new
route (`crates/api-server/src/contract.rs:1`, `:446`; `http/mod.rs:302`). Daily state
does not silently redefine the historical support endpoint or generic ETF data freshness.

D2 supplies exact Web component/test allowlists. D1 does not edit those files. The
existing strategy screen gets the explicit default-OFF control; recommendation views
label expected trading date, daily data support end, result as_of and published time
separately, retaining previous results during failure and marking manual history.
If admission bytes become unavailable, worker fails closed and daily health reports
input unavailable; API metadata alone must not claim a filesystem verification it did
not perform. Publish a bounded verifier status receipt/DB observation for that health
fact, with timestamp and expiry, rather than read Raw from the API.

## 10. Migration, deployment and recovery acceptance

0060 UP is transactional with bounded lock/statement timeouts. Acquire locks for
modified run/config tables, add defaults/nullable daily fields, replace only the
named input-kind/binding/success guard functions/checks, add RLS/grants/functions and
immutable reservation checks. **Do not use 0050's empty-run-table guard.** Existing
PENDING, FAILED, CANCELED and SUCCEEDED historical rows with items must all remain
valid and byte-identical in payload/provenance/output. No automatic opt-in, new daily
control activation, timestamp rewrite, input reseal, or historical backfill occurs.
Preflight verifies roles and existing histories before applying; no migration file
through 0059 changes. Treat checksums as immutable.

0060 DOWN is deliberately guarded: refuse if any daily admission/event/reservation/run,
opted-in config, nonzero daily epoch or activated control history exists. This protects
permanent dedup and evidence; **do not delete daily history to make rollback pass**.
With an unused daily feature, lock tables, remove added objects/grants/columns, restore
exact 0050/0052 function/trigger definitions and 0049 input-kind/NOT NULL constraints,
preserving populated historical rows/items. Down leaves the externally provisioned
login inert with no feature privileges; root role/secrets retirement is a separate
reviewed operation. Most real rollback is a compatible release with daily mode OFF
on schema 0060, retaining evidence. An older reader that cannot decode daily rows must
not be deployed against populated daily histories; require a compatibility release,
not an undocumented destructive downgrade.

Add profile-gated `owner-beta-daily-producer` and `owner-beta-daily-admitter` Compose
services as aliases of the same commit-pinned Owner-beta runner image, no build graph
or new image target. Add `daily-produce`/`daily-admit` subcommands with role/UID/path
validation before file/DB access, plus bounded daemon loops and typed health. No other
command can accidentally enable admission. Original daemon invocation remains supported.
The official manifest verifier resolves both service aliases to the existing
owner-beta-runner image-ID entry and checks the installed revision. Do not allow an
unvalidated tag because a new service lacks a manifest entry. New filesystem/DB role
provision is a separate installer stage before default-OFF startup. No cron/static
unit or ad hoc host script is an alternate activation path.

G4 preparation must provide exact proposed commands/impact, role/grant verification,
protected base/policy pins, initial frontier/budget, resource checks, backups, read-only
mount checks, and disable/recovery procedure before any operational approval. A separate
daily DB control change is bound to the installed release; it must be synchronized with
runtime disable so in-flight workers see the shared kill switch. Existing daily KIS
activation remains solely the immutable installer path. No KIND timer or provider call
is added. Production builds retain one service at a time, COMPOSE_PARALLEL_LIMIT=1,
CARGO_BUILD_JOBS=2 and mandatory batch health/OOM checks; this is not a build-speed task.

Back up admission bytes, policy bytes, DB admissions/events/reservations together with
ordinary data. Restore requires reconciliation before enabling: missing leaf with DB
row => blocked; leaf without DB row => reverify and insert same identity; conflicting
date => blocked; lost reservation with surviving daily run => restore/reconcile the
original key under guarded maintenance, never enqueue a replacement. No startup cleanup
deletes histories. All stale claims stay with existing recovery and their exact IDs.

Structural implementation changes affect both architecture diagrams: new producer and
admitter services, mounts/UID roles, daily tables/functions and web/API surface. Update
`docs/diagrams/component_architecture.puml` and `runtime_deployment.puml` evidence
comments using actual implemented file:line citations, and locally render/commit PNGs
in the implementation integration package. D1 changes no code structure and leaves
diagrams untouched; Docker rendering is prohibited in this assignment.

## 11. Evidence gaps and design review gates

- **Closed-day action coverage:** daily collector's action request is start=end on the
  requested trading date (`providers/kis.rs:289`, `:1070`). Continuous calendar coverage
  alone does not prove absence of noncash actions whose record date falls on a closure.
  Require source action coverage for the whole interval between the base frontier and
  each addition, including closed dates. Existing saved range evidence may satisfy it;
  current host availability was not inspected. If absent, return ACTION_COVERAGE_GAP.
  G4 must settle exact existing-source/catch-up evidence and budget; this report does
  not widen requests or silently approve a daily acquisition change. This is a material
  operational prerequisite, not an excuse to guess zero actions.
- **Daily normalizer evidence:** generic normalization can fail on cash dividends; the
  proposed daily pure verifier must reconstruct and commit its separate normalization
  proof. It cannot cite a nonexistent generic normalized batch. D3 must confirm this
  explicit separation meets the adopted validation boundary before implementation.
- **Calendar discovery:** current service-status continuous-proof logic lives inside
  API service-status, and V2 scheduling checks only latest eligible row. Do not call
  V2 scheduling or treat it as the complete daily proof. Implement a bounded daily DB
  function using the three-way source binding and coverage algorithm; parity fixtures
  must show 16:30/18:30 agreement. Current host calendar coverage is unknown.
- **Policy/activation evidence:** no daily policy bytes/hash, activated control row,
  first_auto_session, verified current gap inventory, or execution budget exists in
  the inspected contract. They remain concrete release-gate artifacts. The design
  specifies schemas and default OFF; no missing value is filled from the clock.
- **Timestamps:** stored ordinary Raw has batch-level retrieval, not independent per-file
  request times. Preserve the recorded precision plus original copied-source times;
  do not claim request-level exactness or strict PIT.
- **Authority limits:** principal separation protects against the data producer, not a
  malicious root/migration authority. Independent admission is a trusted reviewed
  verifier, not an external signature attestation. Unknown future broker corrections
  cannot be detected until evidence arrives. No runtime assurances are inferred.
- **D2/D3 integration:** exact Web edits belong to D2/coordinator; D3 must check the
  tagged legacy dispatch, success-trigger/grant branches, lock order, permanent dedup,
  source completeness and no-snapshot-reseal rules. No unresolved architectural choice
  is delegated to a Luna implementer: amend this design/brief first if D3 changes one.

### D1 verification record

Only this report was written. No product source, plan, progress document, schema,
artifact or private output was created or modified by D1. Read-only Git comparison
against `619634db32526032aae07ee2024a10f69fc560c4` returned no changed paths under
`crates`, `migrations`, `deploy`, `scripts`, `apps/api-server`, `data-pipelines`,
`tests/integration/migration-contract`, `configs/evidence` or `docs/diagrams` while
coordinator documentation changes continued concurrently.

Source path/line-bound checks, three relative document-link checks, balanced code
fences and whitespace checks passed. Source citations were read in context; these
mechanical checks establish existence and line bounds, not runtime correctness.
No test/build/compiler, DB, Docker, root, provider, external network, lifecycle,
Git staging/commit/push or delegation action was performed. The supplied actual
memory hook was used only for relevant context and a no-durable-change finish;
architecture proposals are not stored as adopted decisions. Runtime/compilation/DB
success remains **unverified**. Missing source/operational evidence is itemized above;
unreported blockers: none. Background work: none. Permission waits: none.

## 12. Staged, disjoint implementation packages

These are proposed assignment briefs, **not executions** or modifications of the plan.
Each package begins Luna/max after coordinator freezes these interfaces and resolves
D3 findings. Never grant overlapping files concurrently. Only the listed files are
writable; a needed outside edit is a returned dependency, not implicit scope expansion.
Tests listed here are future acceptance commands and were **not run by D1**.

### I1 — immutable daily evidence and admission library

Allowlist: new `crates/market-data/src/daily_price_only/mod.rs`,
`crates/market-data/src/daily_price_only/contract.rs`,
`crates/market-data/src/daily_price_only/verify.rs`,
`crates/market-data/src/daily_price_only/artifact.rs`,
`crates/market-data/src/daily_price_only/admission.rs`; existing `crates/market-data/src/lib.rs`
and `crates/market-data/src/normalize.rs`; new `crates/market-data/tests/daily_price_only.rs`,
`crates/market-data/tests/daily_price_only_storage.rs`; new
`configs/evidence/owner-beta-daily-price-policy-v1.json` (reviewed policy, not activation).
Do not edit any historical file/registry, provider or collector. Expose only pure
existing normalization helpers needed by the new verifier. Implement sections 3–5,
including raw/source enumeration, whole-market seven-class handling, closed-day gaps,
copied timestamp precision, deterministic serialization, independent reopen/copy and
no-replace storage. Synthetic provider-free fixtures must include missing ETF/day,
extra/duplicate/future rows, mixed adjusted semantics, noncash/mixed dividend, malformed
empty pages, page loops, incomplete calendar/action coverage, changed prefix/hash/policy,
symlink/rename/hardlink mutation attempts, source conflict and crash before/after rename.
Acceptance: `cargo test -p market-data --test daily_price_only --test daily_price_only_storage`;
existing market-data historical approval/artifact unit tests must still pass unchanged.
Return exact exported signatures and immutable-byte fixture hashes.

### I2 — additive DB authority and transaction functions

Depends on frozen I1 schema/hash rules; can implement in parallel with I1 once frozen.
Allowlist: new `migrations/0060_owner_beta_daily_inputs.up.sql`,
`migrations/0060_owner_beta_daily_inputs.down.sql`;
`tests/integration/migration-contract/role-bootstrap.sql`,
`tests/integration/migration-contract/bootstrap.sql`,
`tests/integration/migration-contract/tests/migration_contract.rs`; new
`deploy/db/integration-validation/preflight-0060.sql`,
`deploy/db/integration-validation/actual-role-0060.sql`,
`deploy/db/integration-validation/rollback-0060-guard.sql`. Implement sections 5–6/10 exactly, SQL shape/lineage checks,
current Owner/entitlement/date proof, epochs, narrow definer functions and conditional
run columns. Do not change old migrations or provisioning scripts (I6 owns them).
Actual-role acceptance must populate v2/v3 PENDING and SUCCEEDED histories before UP;
verify unchanged after UP and allowed DOWN. Check denied producer/app/worker admission,
cross-owner access, privilege inheritance, direct daily run spoofing and success bypass;
test concurrent reservation, config edit/restore, OFF/ON, admission/revoke and role/rights
revocation in two real connections. Verify DOWN refusal after any daily evidence/epoch,
rollback atomicity and restored 0050/0052 guards. Command:
`cargo test -p migration-contract --test migration_contract` in coordinator-provided
isolated PostgreSQL harness with required actual-role URLs; a skipped DB test is not PASS.

### I3 — daily factor/target and strict loader transition

Depends I1. Allowlist: `crates/factor-engine/src/bars.rs`,
`crates/factor-engine/src/price_only.rs`, `crates/factor-engine/src/lib.rs`;
new `crates/factor-engine/src/daily_price_only.rs` and
`crates/factor-engine/tests/daily_price_only.rs`;
`crates/job-queue/src/owner_beta/input.rs`, `crates/job-queue/src/owner_beta/compute.rs`,
`crates/job-queue/src/owner_beta/target.rs`, `crates/job-queue/src/owner_beta/mod.rs`;
new `crates/job-queue/src/owner_beta/daily_input.rs`,
`crates/job-queue/src/owner_beta/daily_compute.rs`, `crates/job-queue/src/owner_beta/daily_target.rs`;
new `crates/job-queue/tests/owner_beta_daily_compute.rs`.
Implement section 7 without modifying historical serialization/constructors; centralize
only pure arithmetic. `mod.rs` is owned here and declares only I3's daily modules.
I4's runtime modules are declared under its owned `runner.rs`, so no missing-module
stub or simultaneous edit is required. Verify old
v2/v3 JSON and factor/target golden bytes, daily pins/hash sensitivity, exact snapshot
hash, null factors, all-cash, weights/rounding, future rows and no forward fill.
Acceptance: `cargo test -p factor-engine --test daily_price_only`;
`cargo test -p job-queue --test owner_beta_daily_compute`; existing owner_beta input,
compute/target unit tests and `owner_beta_publish` replay fixtures remain required.

### I4 — scheduler, isolated local producer/admitter and worker settlement

Depends I1–I3, SQL signatures frozen. Allowlist: new
`crates/job-queue/src/owner_beta/runner/daily_schedule.rs`,
`crates/job-queue/src/owner_beta/runner/daily_admission.rs`,
`crates/job-queue/src/owner_beta/runner/daily_runtime.rs`;
existing `crates/job-queue/src/owner_beta/runner.rs`,
`crates/job-queue/src/owner_beta/publish.rs`, `crates/job-queue/src/owner_beta/recovery.rs`;
`crates/job-queue/src/bin/owner-beta-runner.rs`;
new `crates/job-queue/tests/owner_beta_daily_schedule.rs`,
`crates/job-queue/tests/owner_beta_daily_publish.rs`,
`crates/job-queue/tests/owner_beta_daily_recovery.rs`,
`crates/job-queue/tests/owner_beta_daily_runtime.rs`.
Implement sections 4/6–8 using existing image/binary, distinct subcommands/principals,
no provider calls. File scanner is bounded and exact source selection is verified by
I1; no fallback to generic READY. Preserve historical worker invocation. No queue.rs
edits should be needed because the protected job type is unchanged; report otherwise.
Tests: two producers/admitters, restart minute gate, lost admission/SQL commit replies,
poll cadence, capacity competition with manual jobs, source tamper/revocation, stale
lease, kill-before-publication, config epoch, latest-only/superseded run, and preserved
reservation after terminal failure. Acceptance:
`cargo test -p job-queue --test owner_beta_daily_schedule --test owner_beta_daily_publish --test owner_beta_daily_recovery --test owner_beta_daily_runtime --test owner_beta_publish`.
DB tests require actual worker/admitter roles; fixture-only success is not DB evidence.

### I5 — API contract and read/config behavior

Depends I2/I3, can run parallel with I4 after interfaces freeze. Allowlist:
`crates/api-server/src/http/owner_beta.rs`, `crates/api-server/src/http/dto.rs`,
`crates/api-server/src/http/strategies.rs`, `crates/api-server/src/http/mod.rs`,
`crates/api-server/src/http/state.rs`;
`crates/api-server/src/repos/owner_beta.rs`, `crates/api-server/src/repos/strategy_configs.rs`;
`crates/api-server/src/contract.rs`, `crates/api-server/src/runtime.rs`,
`crates/api-server/src/bin/api-server.rs`;
new `crates/api-server/src/http/owner_beta_daily.rs`,
`crates/api-server/tests/http_owner_beta_daily.rs`;
`crates/api-server/tests/http_owner_beta_price_only.rs`,
`crates/api-server/tests/http_owner_beta_access.rs`;
`apps/api-server/scripts/openapi-spec.mjs`, `apps/api-server/openapi.json`.
Implement section 9 and release-mode config validation. Do not use an admin pool to
bypass tenant RLS or make the API a daily producer. Verify default OFF/missing-update
field semantics, normal Owner opt-in, Member/foreign/expired/CSRF rejection, manual
history compatibility, daily union DTO integrity, pending/stale/blocked times and
future dates, revoked input display, and manual-success exclusion from daily acceptance.
Acceptance: `cargo test -p api-server --test http_owner_beta_daily --test http_owner_beta_price_only --test http_owner_beta_access`;
`npm --workspace @lagrange/api-server run openapi:check` remains required
(`apps/api-server/package.json:7`). I5 does not
change service-status generic ETF freshness; a dedicated daily projection owns the new facts.

### I6 — controlled deployment, actual principal tests and diagrams

Depends I1–I5. Allowlist: `deploy/compose/compose.yml`, `deploy/compose/.env.example`,
`deploy/compose/owner-beta-static-check.sh`; `deploy/db/bootstrap-roles.sh`;
`scripts/ops/provision-db-secrets.sh`, `deploy/secrets/provision-runtime-secrets.sh`,
`deploy/secrets/runtime-static-check.sh`; `scripts/ops/deploy-production-release.sh`,
`scripts/ops/compose-release.sh`, `scripts/ops/lib/release-image-manifest.sh`;
`crates/job-queue/Dockerfile.owner-beta-runner` (copy policy source only; same binary/image);
new `scripts/ops/owner-beta-daily-static-check.sh`,
`docs/runbooks/owner-beta-daily-input.md`;
`docs/diagrams/component_architecture.puml`, `docs/diagrams/runtime_deployment.puml`,
`docs/diagrams/component_architecture.png`, `docs/diagrams/runtime_deployment.png`.
No build optimization or unrelated artifact-build pipeline edit. If adding a policy
include_bytes requires another existing Docker COPY boundary, report the exact file
before expanding this allowlist; prefer runtime policy bytes verified against the
compiled hash to avoid broad build-context coupling.
Implement section 10, restrictive filesystem parents/mounts, unique login/secret,
default-OFF profile and manifest aliases. Provide source checks plus **isolated** real
UID/role denial tests proving producer cannot write admitted leaves or call admission,
admitter cannot enqueue/configure, worker/API cannot admit, and no external egress.
Verify runtime readiness/health does not imply admission or a successful recommendation.
Run static checks and local diagram rendering only in the later authorized verification
environment. Prepare reviewed G4 commands; do not independently run deployment/root actions.

### I7 — Web and final integration (coordinator assigns after D2/D3)

Web writable allowlist is **not guessed here**: take exact paths from D2, freeze DTOs
from I5, and attach separate Web test acceptance. Integration alone may sequentially
touch the coordinator's ROADMAP/STATUS/plan/
execution files, and CI invocation inventory if necessary. Keep new source implementation
and historical runtime/Owner/three-day evidence separate. Independent D3/review findings
must be resolved before treating packages as assignable without architectural guessing.
This is the only remaining package-scope dependency; no native delegation was used by D1.
