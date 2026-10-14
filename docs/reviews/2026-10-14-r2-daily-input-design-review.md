# R2 WP-4 D3 — independent source-bound design review

Date: 2026-10-14, Asia/Seoul. Scope: D3, R2-04/08/09. Classification:
intermediate, medium confidence. Reviewed checkout HEAD
`a83bd64d3dcd72087471b697d6e988342a0e0a83`; product sources match
`619634db32526032aae07ee2024a10f69fc560c4` in the compared product directories.
Owner adoption of U2 is resolved. This review requests no renewed adoption or rights approval.
At final checking HEAD was `b476cdaa438056f92b98170e3364c1c049e14243`; the product-directory comparison remained empty and D1's frozen hash remained identical.

Reviewed AGENTS.md, ROADMAP.md, the adopted U2 decision, the implementation plan
including its final coordinator integration section, D2 and the completed D2a
follow-up. D1 input is frozen at 926 lines, SHA-256
`e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670`.
No source/configuration/migration was implemented. All execution checks below are
**required future verification**, not tests performed by D3.

## Assessment

The separate daily tag, retained historical reader/five pins, isolated admission,
permanent reservation key and default OFF are sound review directions. The whole
implementation is **not yet frozen sufficiently for Luna/max**. Resolve the concrete
source/contract findings below before assigning their affected packages. A narrow
pure contract slice can proceed after its byte schema is frozen; absent operational
policy hashes, host evidence and G4 values do not stop that slice.

| ID | Priority | Affected boundary | Classification |
|---|---|---|---|
| D3-01 | P1 | Recurring closed-day action evidence | Structural supply/consumer contract gap; also separate host evidence unknown |
| D3-02 | P1 | Closed-day calendar DB projection | Existing target-only projection cannot manufacture surrounding-day bindings |
| D3-03 | P1 | Existing config grants and epochs | Mandatory exact ACL/trigger specification before I2; proposed rule alone is insufficient |
| D3-04 | P1 | Success guards and durable outputs | Concrete direct-worker DML path remains outside transition-only gate |
| D3-05 | P1 | Role bootstrap and official verification | Missing integration locations in proposed allowlists |
| D3-06 | P2 | I1/SQL interface freeze | Missing byte schema, normalization and conflict-result contract |

These are design findings against the proposed extension of existing code, not
claims that an unimplemented 0060 migration has been executed and failed.

## D3-01 — a finite catch-up does not solve the recurring action gap

**Design:** [D1 lines 211–229](2026-10-14-r2-daily-input-design.md#L211),
[720–728](2026-10-14-r2-daily-input-design.md#L720).
**Source:** [daily action request](../../crates/market-data/src/providers/kis.rs#L289)
passes the same date twice; [query construction](../../crates/market-data/src/providers/kis.rs#L1070)
records F_DT/T_DT. [Daily scheduling](../../scripts/ops/kis-daily-production.sh#L503)
selects missing trading sessions and [delegates those exact dates](../../scripts/ops/kis-daily-production.sh#L616).

**Failure:** after an admitted Friday, Monday's seven logical action classes cover
Monday only. D1 requires Saturday/Sunday action coverage before Monday admission.
Calendar CLOSED rows cannot prove no corporate action. In the absence of independently
committed covering action evidence, ACTION_COVERAGE_GAP is correct; every later
one-session extension remains behind the missing interval. A one-time G4 catch-up
only moves the frontier to the next uncovered closure. This is a recurring structural
block under the current trading-date-only supply, not merely missing runtime proof.

There is a concrete existing range capability: [fetch_corporate_actions_range](../../crates/market-data/src/providers/kis.rs#L316)
supports WholeMarket and FixedEtf11 with sequential seven/77 initial class groups
and bounded continuation; [ingest_kis_action_range_with_batch_id](../../crates/market-data/src/ingest.rs#L835)
commits all pages as one Raw batch. The existing [range CLI](../../data-pipelines/collectors/src/bin/kis-action-range-raw.rs#L303)
is operator-confirmed, not an automatic daily supplier. It explicitly reports
coverage_claim=not_asserted; a successful capture still needs independent validation.

**Correction condition:** freeze offline consumption of already-committed range
evidence: for every class and ETF11 identity, validated request intervals must cover
every civil date in `(previous frontier, target]`. Accept the existing whole-market
seven-class shape or complete ETF11-scoped range shape; do not demand 77 files from
daily whole-market evidence. Validate every response row before filtering; classify
actions relevant to the required interval without inventing ex-dates or factors.
Discovery must inspect request intervals, not only ManifestEntry.date: range ingestion
sets the batch date to **start** at [ingest.rs:867](../../crates/market-data/src/ingest.rs#L867).
A covering batch beginning before the verifier's target window must remain discoverable.
Do not modify old historical range verifiers/pins to implement this daily contract.

The coordinator must separately identify a recurring supplier or record that
activation remains blocked. Reusing existing saved bytes requires no new provider
call. Obtaining additional ranges, adding closed-day calls, or changing daily F_DT/T_DT
is **not authorized by this review** and needs a concrete scope/budget/request-contract
decision under existing boundaries. Do not silently broaden the adopted daily budget.

**Required verification:** provider-free Friday→Monday and holiday-week fixtures
where all seven classes cover only the trading date must fail. The same additions
with complete committed covering range chains must pass; deleting one civil date,
class, symbol group or terminal page must fail. Include a range whose batch date lies
outside the queried addition window, a closed-date noncash ETF action, malformed
non-ETF rows, repeated pages and contradictory overlapping evidence. G4 then separately
verifies actual files and the explicitly authorized continuing supply.

## D3-02 — surrounding calendar rows are not existing DB projections

**Design:** [D1 lines 221–229](2026-10-14-r2-daily-input-design.md#L221),
[370–379](2026-10-14-r2-daily-input-design.md#L370),
[733–737](2026-10-14-r2-daily-input-design.md#L733).
**Source:** [normalize_calendar_with_contract](../../crates/market-data/src/normalize.rs#L1144)
validates surrounding broker rows but discards dates different from source.date.
The [standalone calendar sink](../../data-pipelines/collectors/src/sink.rs#L566)
requires exactly one fact for bundle.target_date. The current
[three-way DB join](../../crates/api-server/src/repos/service_status.rs#L288)
requires matching calendar/version/source and batch.batch_date=calendar.session_date.

**Failure:** a Friday Raw response containing Saturday/Sunday closure rows can prove
those dates to a new daily Raw verifier, but existing normalization/publication creates
only Friday's canonical fact. Reconstructing surrounding rows inside admission does
not create Saturday/Sunday's required DB bindings. D1 explicitly forbids creating
that projection there. Monday may therefore admit on Raw proof but remain unschedulable;
weekend expected-session discovery also fails if the closed-date projections are absent.
An XKRX dates artifact supplies neither missing binding nor credentialed evidence.

**Correction condition:** identify the exact existing standalone per-civil-date
calendar supply as a prerequisite, or specify a separate reviewed **offline** projection
contract for already-acquired first-page bytes. The latter must honestly bind original
source date/retrieval/hash and any derived normalized document; it cannot claim that the
existing target-only normalizer emitted the surrounding dates. It also needs an explicit
SQL join contract and owned source integration locations. Do not change the generic
calendar normalizer or synthesize data_batches identities implicitly in I1/I2.
Host availability of per-day sources is a G4 unknown; the target-only source behavior
and missing recurring supply specification are already knowable without host access.

**Required verification:** Friday Raw containing both closed days with only Friday's
DB projection must not yield a verified weekend/Monday DB gate. Exact independently
bound civil-day projections must permit discovery with 16:30/18:30 parity to
[service-status coverage logic](../../crates/api-server/src/service_status.rs#L3204).
Missing/duplicate/conflicting proof rows, future retrieval instants, normalized-vs-wire hash confusion and
copied-source timestamp changes must fail. No second chk-holiday call is introduced.

## D3-03 — revoke table grants before claiming epoch column isolation

**Design:** [D1 lines 298–331](2026-10-14-r2-daily-input-design.md#L298).
**Source:** [0009 grants](../../migrations/0009_grants.up.sql#L27) give app table-level
INSERT/UPDATE/DELETE on user_strategy_configs. [0010 policies](../../migrations/0010_rls.up.sql#L49)
authorize app writes by owner GUC, with no Owner-role predicate or column restriction.
[Existing repository DML](../../crates/api-server/src/repos/strategy_configs.rs#L55)
and [updates](../../crates/api-server/src/repos/strategy_configs.rs#L117) use this authority.

**Failure:** merely adding UPDATE(daily_enabled), or revoking UPDATE(epoch), leaves
the table-level grant effective for newly added columns. App can supply an epoch on
INSERT or UPDATE unless a trigger always rejects/overwrites it. A partially implemented
epoch trigger can allow edit-and-restore to recover the sealed epoch. Adding another
permissive Owner RLS policy does not narrow the old FOR ALL policy. Ownership/identity
mutation through existing broad config privileges is another bypass unless frozen.
D1 correctly warns about this, but does not yet freeze the replacement privilege set
and INSERT behavior needed to make its claim executable.

**Correction condition:** in 0060 revoke table-level INSERT/UPDATE on this table
from app, then explicitly regrant existing API-required columns plus daily_enabled,
excluding epoch. Keep needed SELECT/DELETE; preserve current actor RLS. Freeze IDs and
owner on UPDATE, set epoch=0 under DB authority on INSERT, and increment from OLD on
every relevant actual change (including OFF→ON, ON→OFF and edit-and-restore); caller
supplied epoch is rejected. If a different ACL/trigger arrangement is chosen, specify
its equivalent effective checks explicitly, including INSERT. The opt-in guard must
check real current Owner membership plus actor/config-owner equality inside SQL;
an extra permissive policy alone does not implement it. Control epoch and its Owner,
policy, entitlement, activation and capacity fields remain release-authority-only.
DOWN must restore the exact prior table grants after its unused-feature guard passes.

**Required verification:** real app login with Owner/Member/foreign/unset actor:
INSERT explicit epoch, UPDATE only epoch, UPDATE config+epoch, ID/owner substitution,
Member direct opt-in, and Owner legitimate create/update/disable. Assert effective
table/column privileges, not just migration text. No-op updates preserve epoch;
edit→restore and disable→reenable monotonically change it and reject old sealed jobs.
Check control toggles similarly and restored legacy CRUD after allowed DOWN.

## D3-04 — transition-only success gates leave published outputs writable

**Design:** [D1 lines 391–394](2026-10-14-r2-daily-input-design.md#L391),
[442–447](2026-10-14-r2-daily-input-design.md#L442),
[456–470](2026-10-14-r2-daily-input-design.md#L456).
**Source:** [0049 worker grants](../../migrations/0049_owner_beta_price_recommendations.up.sql#L139)
permit updates to status/factor hash/finished_at;
[0051](../../migrations/0051_owner_beta_target_publication.up.sql#L55) adds target hash
and cash. [0052 success trigger](../../migrations/0052_owner_beta_strategy_config_lock.up.sql#L18)
returns early if OLD is already SUCCEEDED. Its trigger fires only UPDATE OF status.
The [immutable lineage guard](../../migrations/0050_owner_beta_strategy_snapshots.up.sql#L63)
does not freeze output hashes or publication time.

**Failure:** after valid daily publication, a worker-role UPDATE of factor hash,
target hash, cash_weight or finished_at, without changing status, bypasses a daily
branch added only to the existing success trigger. Even `SET status='SUCCEEDED'`
returns early under the old condition. A shape/link consistency check can still pass:
the job/attempt are successful, ETF11 items exist and hashes are nonnull. Thus it
does not by itself prevent rewriting retained output/provenance/publication time
after admission revocation or config fencing. This is an available DML path, not
a claim that the current Rust publisher uses it.

**Correction condition:** freeze daily terminal output immutability explicitly in
0060, covering all output/status/time UPDATE columns and item mutations. Reject changes
to already published daily hashes/items/finished_at and terminal status resurrection;
permit only specified PENDING retry/recovery transitions. The initial daily success
transition must enforce authoritative gates even for direct worker DML, with deferred
job/run/reservation/item consistency at commit. State precisely which current attempt
must be SUCCEEDED and how it binds job.attempt_count; an arbitrary old successful
attempt is insufficient. Preserve historical behavior/bytes rather than silently
applying new daily restrictions to legacy replay.

**Required verification:** actual worker login attempts post-success output/time
rewrites, status round trips, extra/non-ETF items, stale-attempt success, direct success
without queue settlement, and publication after each epoch/role/entitlement/admission
fence. All fail atomically with unchanged result/reservation bytes. Legitimate first
publication and exact replay pass. Race tests must use two connections and barriers;
SQL shape checks alone do not prove the fence.

## D3-05 — proposed files are not wired into existing bootstrap/harness

**Design:** [D1 I2](2026-10-14-r2-daily-input-design.md#L803),
[I4/I5](2026-10-14-r2-daily-input-design.md#L846),
[I6](2026-10-14-r2-daily-input-design.md#L894).

**Failure and existing integration locations:**

1. I2 reserves SQL files but not [deploy/db/integration-validation/validate.sh](../../deploy/db/integration-validation/validate.sh#L633).
   That official runner has fixed role/password mappings at 633–669, secret generation
   at 1029–1041, fixed service-login lists at 936/1099, and fixed DB test inventory at
   1315–1324. CI invokes it at [.github/workflows/ci.yml:155](../../.github/workflows/ci.yml#L155).
   New `actual-role-0060.sql`/preflight/rollback files would otherwise never execute;
   new daily DB test targets are not in its no-SKIP lane. Extending production
   bootstrap alone also makes that runner need the new secret before migrations.
2. Updating migration-contract role-bootstrap.sql does not update the independent
   bootstrap copies in [job-queue tests/common](../../crates/job-queue/tests/common/mod.rs#L33)
   and [api-server tests/common](../../crates/api-server/tests/common/mod.rs#L48).
   Each runs the full migrator; a clean standalone I4/I5 cluster lacks the new role
   required by 0060. Previously created cluster-global roles can conceal this omission.
3. I6 omits [scripts/ops/validate-production-config.sh](../../scripts/ops/validate-production-config.sh#L828).
   It owns source-secret classification and per-service UID/GID/mode runtime_specs;
   provisioner/static-check changes alone do not validate the new admitter secret.

**Correction condition:** coordinator assigns these exact existing integration
locations, sequentially where shared, and couples bootstrap readiness to the first
migration/DB test execution rather than deferring it until I6. Include CONNECT/USAGE,
direct-login checks, safe role attributes and no membership in any runtime role.
[Production bootstrap](../../deploy/db/bootstrap-roles.sh#L96) currently sets role
attributes but does not reset memberships/NOINHERIT; require a fail-closed membership
preflight or explicitly reviewed revocation, not an assumption that an existing
role name is clean. No credentials or bootstrap authority reach serving processes.

**Required verification:** clean isolated cluster, full 0059→0060 upgrade with populated
legacy histories, direct daily_input_admitter login and effective role/ACL/RLS checks,
and new SQL/Rust test lanes recorded by the official runner with zero SKIP. Also run
I4/I5 test commands on independently fresh clusters. Add secret ownership/missing-secret
negative checks to the existing config validator. Existing bootstrap and inactive
legacy mode must retain their documented behavior. D3 ran none of these commands.

## D3-06 — freeze byte contracts and conflict outcomes before broad I1

**Design:** [D1 interfaces/manifest](2026-10-14-r2-daily-input-design.md#L83),
[evidence fields](2026-10-14-r2-daily-input-design.md#L172),
[SQL signatures](2026-10-14-r2-daily-input-design.md#L338),
[I1 allowlist](2026-10-14-r2-daily-input-design.md#L781).

**Failure:** DailySourceEvidenceV1, OpenedDailyCandidateV1, ReadOnlyDailyEvidenceStore,
VerifiedDailyAdmissionV1 and TrustedDailyPolicyRegistry have no complete type/byte
contracts. DailySessionEvidenceV1 and admission.json list concepts rather than exact
keys/types/order. Two workers can hash different valid-looking evidence or implement
different interval selection. Generic pure normalization entry points also use random
BatchId at [normalize.rs:565](../../crates/market-data/src/normalize.rs#L565) and
[381](../../crates/market-data/src/normalize.rs#L381); copied-calendar normalization
calls a persisting path at [1046](../../crates/market-data/src/normalize.rs#L1046).
Calling these wrappers is not deterministic, read-only daily reconstruction.

Separately, `admit_owner_beta_daily_input` returns `(admission_sha256, inserted)` but
D1 says same-date different content returns BLOCKED. That interface cannot distinguish
an exact replay from conflict unless a result/exception contract is specified. Treating
both as inserted=false can accidentally record a VERIFIED observation for a conflict.

**Correction condition / narrow first slice:** freeze I1a as pure contract/encoding
only, with owned new `daily_price_only/{mod,contract}.rs`, the `lib.rs` declaration
and a provider-free `tests/daily_price_only.rs`. No storage/DB/admitted constructor,
normalizer extraction or provider changes in that first slice. Coordinator supplies
an exact reviewed schema containing:

- Policy: schema/version, exact base five-pin object, ordered ETF11 identities,
  request/price/action semantics, source-kind allowlist, interval/page/count/byte bounds,
  normalizer/materializer versions; operational activation fields are excluded.
- Manifest: the D1 fixed metadata, full ordered sessions, parent admission Option,
  bars/normalization descriptors with explicit hashes/sizes/counts, daily_sources,
  policy hash and producer commit; exact content-hash exclusion rule and final byte hash.
- Per session: session date and prior frontier, ordered 11 identities, typed bars,
  reference, calendar and action evidence lists. Actions explicitly carry class,
  symbol scope, requested start/end, page index and request/response continuation;
  calendar evidence identifies civil dates and original source bindings.
- Per file: provider/market, batch ID/date, batch.json and exact manifest-line hashes,
  filename/kind, wire hash/size, redacted RequestMetadata, response continuation,
  original retrieval time/precision and ordered copied-source chain. Never infer a
  per-request time from batch retrieval. Fix sorting and duplicate-key rejection.
- Admission: exact content/manifest/policy/base/parent/evidence hashes, verifier commit,
  normalizer version and explicit counts. No self-hash, generated UUID or wall-clock time.

Freeze parse/encode/hash methods returning **unapproved** values and finite errors;
approved types remain privately constructed by later independent verification.
Golden fixtures must include exact canonical bytes plus content/manifest/admission
hash preimages, duplicate/unknown fields, reorder convergence, parent/policy/lineage
sensitivity, and explicit historical v2/v3 wire/hash preservation. Synthetic fixture
pins must never become production policy pins.

Before I1b, freeze a read-only source reader that exposes exact committed manifest-line
bytes and bounded file reads, interval-aware discovery and copied-source resolution.
Freeze deterministic normalization outputs and caller-independent batch identities
from original lineage/version; no call to a persistence/recovery helper. Add explicit
SQL ADMITTED/REPLAY/BLOCKED disposition (or equivalent typed exception/empty-result
contract with committed conflict event), and the corresponding poll receipt semantics.
This is a boundary refinement of I1, not a replacement implementation plan.

**Required verification:** first slice's deterministic byte/rejection tests; later
I1b independently rebuilds identical evidence from original Raw, rejects a forged
producer normalization document and proves no filesystem writes during verification.
Exact replay and different same-date admission must produce distinct SQL outcomes;
the conflict event must survive and VERIFIED must be impossible on that outcome.

## Accepted boundaries and targeted acceptance conditions

**Normalization:** [U2 lines 31–33](2026-10-13-r2-daily-input-decision.md#L31)
requires existing validation, not generic dataset READY. D1's separate daily cash-only
normalization is compatible if it reuses the field validator and reconstructs all
bars/reference/calendar/action evidence independently. The
[shared action validator](../../crates/market-data/src/normalize.rs#L1396) checks cash
amounts, stock rate and contradictory stock/odd-lot payment dates; the historical
[cash classifier](../../crates/market-data/src/range_to_canonical_v3.rs#L485) already
uses it. Generic [unsupported normalization](../../crates/market-data/src/normalize.rs#L1495)
must remain unsupported. Require a valid cash dividend to pass only the daily
price-return path; mixed/contradictory cash, bonus and other noncash ETF rows must fail
daily admission even when generic bonus normalization succeeds. Missing/malformed
classes and non-target malformed rows must never become empty proof. No inferred
available_at/ex-date or dividend-adjusted close is accepted. No generic behavior
weakening is necessary.

**Historical replay/storage:** retain the [five-pin v2/v3 reader](../../crates/market-data/src/historical_price_only_approval.rs#L268)
and historical canonical
factor/target bytes, including original untagged JSON dispatch. D1's no-hardlink copy,
fsync/no-replace/reopen algorithm meets the producer descriptor-mutation threat in
principle. Require two-process mutation/rename/symlink tests, same-content convergence,
changed-content no-clobber, orphan/lost-commit recovery, original retrieval precision,
and retained policy/artifact bytes after revocation. Producer/admitter/worker UID/GID
and mount denial remain real isolated/G4 verification, not established by this review.

**Reservation/lock order:** D1 uses the actual capacity lock at
[repos/mod.rs:37](../../crates/api-server/src/repos/mod.rs#L37), permanent Owner/config/
day/version PK and atomic job→run→reservation insertion. It correctly avoids resealing
on changes and job locks during config/control mutation. Publication/recovery must
preserve [job-before-run order](../../crates/job-queue/src/owner_beta/publish.rs#L321)
and then control→user/Owner-role→exact entitlement→config→admission. Admission/events
take the control exclusive lock; publication holds it shared. Existing
[0035 authorization](../../migrations/0035_recommendation_entitlement_lock.up.sql#L25)
alone cannot prove current Owner role/current rights/full cumulative range. Specify
the new gate's actual locked predicates and deterministic entitlement-row order;
freeze safe definer ownership, qualified objects, PUBLIC/runtime EXECUTE revocations,
FORCE-RLS owner policies and exception-safe actor-GUC restoration. Do not use
session_user as an implicit actor or grant worker config UPDATE to acquire row locks.

Required race outcomes are exactly D1's table: edit-and-restore, per-config and global
disable/reenable invalidate old epochs; role/rights/event revocation before guarded
success prevents publication; commit before event retains result with revoked status.
Manual/daily producers share capacity. Lost replies return identical reservation/job/
run/snapshot; terminal failure permanently consumes the key. Test lease loss against
[queue attempt fencing](../../crates/job-queue/src/queue.rs#L398), then recovery of the
same IDs with no source/epoch rewrite. A config/admission fence failure must settle
the existing job, not strand it as capacity-consuming PENDING forever.

**Rollback:** guarded DOWN is appropriate only before any durable daily use. Its
all-tenant inspection must be effective under FORCE RLS; a migration_owner session
without actor must not miss another Owner's daily row/epoch. Include a second-owner
rollback refusal fixture and verify ledger/rows unchanged after failure. Real rollback
retains 0060/history with a daily-capable release OFF. Existing
[release rollback](../../scripts/ops/deploy-production-release.sh#L272) only validates
the installed release and switches links: it is not an existing schema-reader
compatibility gate. I6 must concretely enforce the new compatibility prerequisite in
the controlled launch/rollback procedure and test refusal to start a historical-only
reader on populated daily history, rather than assume the present helper already does.

**API/Web:** the new daily-state route and input union preserve honest distinction
between expected session, eligible input support end, result as_of and actual successful
finished_at. Never relabel created_at or manual success as daily publication. Specify
an expired/failed verifier observation as UNVERIFIABLE/blocked while retaining the
last result and dates; absence of admission bytes cannot yield DAILY CURRENT from DB
metadata alone. Default false, omitted update preserves value, Owner-only explicit
opt-in and separate Member/foreign/expired/CSRF paths need real HTTP checks. D2's
existing form creates configs; existing-config edit/disable control must retain the
same config ID so toggling does not defeat permanent reservation semantics.

## Commands, scopes and evidence limits

The named Rust packages and existing `owner_beta_publish`, `http_owner_beta_price_only`,
`http_owner_beta_access`, `migration_contract` targets exist. D1's new daily test targets
are intended new files; their absence today is not a wrong command. Add `--locked`
to future acceptance commands and run DB targets through a supplied isolated harness
with no-SKIP evidence; an unset DATABASE_URL invocation is not verification.

D2's npm test scripts exist. D2a's corrected installed docs are at the root hoisted
`node_modules/next/dist/docs/`, not an app-local tree. Future Web checks should use the
existing [official E2E wrapper](../../scripts/qa/candidate-web-e2e.sh#L1) for full
acceptance; direct Playwright fixture invocations are narrower checks. The
[OpenAPI checker](../../apps/api-server/scripts/openapi-check.mjs#L33) regenerates a
drifting spec and writes generated TypeScript; its command is not read-only and its
expected generated outputs need a stated implementation allowance. D3 did not run it.
No new workspace member or runtime-module ownership overlap is needed: current
`owner_beta::runner` is public, so I4 runtime exports can stay under runner.rs.

G4/G5 unknowns remain: actual saved range/civil-date calendar bytes and projections,
UID availability and filesystem parents/mounts, actual role/rights/catalog state,
reviewed policy/base release hash, initial expected session/frontier/capacity/budget,
deployment/backup/reconciliation and installed image aliases. These unknowns are not
permission to guess values or a prohibition on pure source work. Real Owner opt-in,
provider receipt, runtime recovery and three consecutive unattended sessions remain
separate acceptance; none is claimed here.

## D3 completion record

- Changed file: this report only; final line range is reported with the handoff.
- Deviations: none. No D1, plan/progress, source/config/test/migration/diagram edit;
  no staging/commit/push, root/production/DB/Docker/provider/external-network action,
  build/test/compiler/runtime or delegation. Coordinator owns integration and roadmap.
- Actual checks: bounded rg/nl/sed source reads, HEAD/status/product diff (no changed
  product paths), D1 926-line count and exact SHA-256 verification; report relative-link/
  source-line bounds (57 relative links), code-fence and whitespace checks passed. These establish source/
  document consistency, not execution. Memory finish skips saving this unadopted review.
- Unresolved/follow-up: coordinator resolves D3-01 through D3-06 for the affected
  packages; freezes the narrow first schema slice; assigns missing integration paths;
  later independent source review and actual-role/storage/API/CI/G4 verification.
- Not found: a recurring closed-day action supplier in the inspected daily path;
  automatic surrounding-date canonical calendar projection in the inspected normalizer/
  sink; complete daily byte types and SQL conflict outcome in D1; harness integration
  for new role/test SQL in the proposed allowlists. These are the findings above.
- Unverified: compilation, tests, DB grants/RLS/locks, host evidence/permissions,
  deployment/runtime/provider/Owner behavior. None claimed. Other unreported missing
  evidence: none. Further implementation decomposition needed: I1a/I1b boundary only
  as specified; D3 itself is complete without delegation.
- Background work: none. Permission waits: none.
