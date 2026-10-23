# R2 WP-4 D6 — independent amendments delta review

2026-10-14, Asia/Seoul. Assignment: intermediate/high-confidence source design review,
selected codex/gpt-6.1-sol high. Workspace `wks_10fae7d550ab1704`, checkout
`/data/worktrees/3puw275b/excited-dolphin`. Reviewed directly, without delegation.
Roadmap scope is R2-04/08/09, WP-4/D6. Coordinator owns acceptance, roadmap/status/plan,
Git integration and subsequent execution. U2 adoption and source authorization remain
resolved; no additional acquisition or operational permission is inferred here.

## Verdict and frozen inputs

**One P2 / MEDIUM interface finding remains, D6-01.** D3-01 through D3-05 have
source-design closure under D5's explicit overrides. D3-06's runtime outcome and
read-only reconstruction architecture is specified, but its D4/A6 dividend mapping
has the concrete contradiction below. Correct that narrow interface before treating
the affected I1a rules and I1b mapping as final. It does not require production
inventory, policy activation or G4 values, and does not stop unrelated pure wire work.
No P1 / HIGH finding was identified in this bounded delta review.

These are closures of the prior **design failure paths**, not implementation acceptance,
executed guards, effective production ACLs, runtime success or Owner acceptance.
The ongoing I1a implementation was neither read nor reviewed as a completed artifact.

| Input | Lines | Verified SHA-256 |
| --- | ---: | --- |
| [D1](2026-10-14-r2-daily-input-design.md) | 926 | `e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670` |
| [D3](2026-10-14-r2-daily-input-design-review.md) | 423 | `1ea75c9e1f8612475923775c5b2b49c2f3b7a4a2aa395e482d09cda1c4c9a487` |
| [D5](2026-10-14-r2-daily-input-design-amendments.md) | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |
| [Final coordinator-corrected D4](../contracts/owner-beta-daily-etf11-input-v1.md) | 904 | `308b9909932a323440557afc06ebd3c3a7fd26e44be38a502a02c25803ce377d` |

Product reference: `619634db32526032aae07ee2024a10f69fc560c4`. Inspection HEAD:
`16ce54beda057e1b0d13c3e95f7976d20197c066`; final documentation HEAD advanced to
`d266b25ff1187fa808c88738f1ad6db603fbdb48`. The compared committed product directories
have no differences from the product reference at either HEAD. The relevant existing source paths
also had no working-tree diff when checked. Concurrent coordinator documentation
and I1a work remain outside D6 ownership. Applicable repository/user AGENTS, ROADMAP,
[adopted U2](2026-10-13-r2-daily-input-decision.md) and the
[D6 brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L269)
were read. This report changes none of them.

## D6-01 — validated cash metadata cannot pass the frozen wire scalar

**Severity:** P2 / MEDIUM, high-confidence source/interface contradiction.
**Affected packages:** I1a's scalar validation and cash-row fixture specification;
I1b's independent dividend reconstruction. No DB/runtime/G4 prerequisite causes it.

**Exact evidence:** D4 [line 55](../contracts/owner-beta-daily-etf11-input-v1.md#L55)
requires `code` to match ASCII `[A-Za-z0-9_-]{1,32}`. Its cash-row table applies that
rule to `dividend_kind`, `stock_kind` and `high_dividend_flag` at
[lines 286–295](../contracts/owner-beta-daily-etf11-input-v1.md#L286).
D5 [lines 542–547](2026-10-14-r2-daily-input-design-amendments.md#L542) require extracting
the existing pure validators, and [line 565](2026-10-14-r2-daily-input-design-amendments.md#L565)
requires D4 CashDividendRow objects after equivalent-chain selection.
The shared validator uses `required_present_string` for all three source fields at
[normalize.rs:1398](../../crates/market-data/src/normalize.rs#L1398),
[1409–1410](../../crates/market-data/src/normalize.rs#L1409). That helper accepts a present
string and returns its trimmed value, including an empty string, at
[1890–1900](../../crates/market-data/src/normalize.rs#L1890).
The existing [valid cash fixture at kis_normalization.rs:85–99](../../crates/market-data/tests/kis_normalization.rs#L85)
has all three strings empty, a positive cash amount, zero stock rate and no stock/odd-lot
payment date. The historical cash classifier carries these validated values into its
commitment at [range_to_canonical_v3.rs:499–519](../../crates/market-data/src/range_to_canonical_v3.rs#L499).

**Reachable contradiction:** a complete dividend chain with that validated row shape
and a record date inside the required daily extension satisfies the shared field and
cash-only checks. Faithfully mapping its three strings produces D4 rows that I1a
must reject. I1b therefore cannot both preserve the source-validated fields and emit
the required D4/A6 rows. Dropping the row fabricates empty action evidence; substituting
an invented code or null fabricates metadata. Selection/count corrections do not fix
the lexical conflict. This is a source-derived counterexample, not a claim that such
a row has been received in production or that a test was executed here.

**Minimal resolution requirement:** coordinator must freeze a bounded string rule for
these three recorded fields that preserves the shared validator's present-empty
outcome, distinguishes it from a missing/non-string field and explicitly defines any
remaining character restriction and mapping. Do not infer codes or alter the legacy
validator to accommodate the new wire. Bind the corrected D4/launch brief, then require
a nonempty cash fixture with these empty values to round-trip through I1a and match
I1b reconstruction; absent/non-string source fields must still fail. Existing fields,
function signatures and seven primary empty-cash goldens need no change for that
correction. D6 has not edited D4 or any implementation file.

## Prior finding closure matrix

| Prior finding | Source-design disposition | Delta evidence and limit |
| --- | --- | --- |
| D3-01 | Closed at source-design level | D5 A1 indexes committed requests by inclusive interval independently of partition date, requires complete7/77 bundles/pages and civil-day/class/ETF coverage, validates all rows before filtering and compares deterministic multisets. Recurring acquisition is a concrete OFF proposal, separately gated at G4. Actual covering files and acquisition semantics remain unverified. |
| D3-02 | Closed at source-design level | D5 A2 defines independent original-Raw calendar projection, target inclusion, copy lineage, dedicated source/day tables and SQL proof query. Projection precedes admission/expected-session scheduling and creates no generic data_batches identity. Original bytes, projected rows and actual DB predicates require later verification. |
| D3-03 | Closed at source-design level | D5 A3 replaces table INSERT/UPDATE with exact column grants, freezes identity and DB-owned epochs, checks locked actual Owner membership and actor equality, and permits OFF after membership loss. A4 DOWN explicitly removes the new column ACLs and restores old table grants. Effective roles/ACLs are unexecuted. |
| D3-04 | Closed at source-design level | D5 A4 adds daily terminal run/item/queue/attempt guards, a trigger-derived current-claim publication receipt, immediate and deferred consistency, specified failure/retry/cancel/orphan states and shared source-event authority. All-tenant unused-only DOWN and compatible OFF rollback are specified. This closure depends on implementing the combined guards and runtime settlement; existing source alone does not provide them. |
| D3-05 | Closed at source-design level | D5 A5/A8-I0 moves bootstrap/secret/preflight/harness readiness before first0060 execution, owns both tests/common copies and official validator inventory, and assigns generated OpenAPI outputs sequentially. No bootstrap, secret, actual-role or harness execution occurred. |
| D3-06 runtime/reconstruction | Architecture condition addressed; D6-01 remains | D5 A6 separates committed read-only snapshots/original resolution/reconstruction from persistence, freezes internal document bytes, ADMITTED/REPLAY/BLOCKED and committed poll/event outcomes. Final D4 specifies the pure interface; the dividend scalar contradicts A6's source mapping. None of these future seams was run. |

The following checks explain those dispositions and retain the precise implementation
conditions rather than treating a proposed source guard as execution evidence.

### Action discovery and recurring supply

D5 [lines 42–87](2026-10-14-r2-daily-input-design-amendments.md#L42) close the earlier
partition filter failure: an interval beginning before the addition window is still
eligible for discovery. That matches range ingestion's actual start-date partition at
[ingest.rs:867](../../crates/market-data/src/ingest.rs#L867). WholeMarket7 and FixedEtf11
are explicit, rather than demanding77 files from the existing whole-market daily
bundle; [providers/kis.rs:345–365](../../crates/market-data/src/providers/kis.rs#L345)
shows the existing scope matrix. D5 additionally requires ordered terminal page chains,
unchanged queries, no repeated bytes, and every day/class/symbol cell. Equivalent
overlaps retain witness evidence while one deterministic chain contributes semantic
rows; unequal/duplicate/invalid evidence blocks. The old source matrix helper at
[kis.rs:1198](../../crates/market-data/src/providers/kis.rs#L1198) is a reuse reference,
not proof that its existing checks alone implement every new daily requirement.

D5 [lines 89–139](2026-10-14-r2-daily-input-design-amendments.md#L89) specifies one
additional Raw-only WholeMarket7 range, maximum 14 civil days, one persisted attempt
reservation/day, 70 logical pages and 280 wire GETs including four attempts/page.
[retry.rs:33–38](../../crates/kis-client/src/retry.rs#L33) supports that retry arithmetic.
The EOD bundle remains separate; aggregate requests, current official range semantics,
activation/frontier and runtime wiring remain G4. Offline planning is source work,
not an approved execute path. This addresses recurring weekends structurally without
claiming that one historical catch-up solves future closures.

### Calendar lineage and dependency direction

D5 [lines 149–204](2026-10-14-r2-daily-input-design-amendments.md#L149) require the
original requested target exactly once, unique validated surrounding rows and committed
copy resolution without resetting retrieval times. Source columns describe the terminal
original while evidence_json retains delivery/copy lineage. Future schedule dates are
facts; future retrieval instants fail. The old target-only filtering at
[normalize.rs:1144–1168](../../crates/market-data/src/normalize.rs#L1144) is preserved.

The dedicated SQL source/day join and continuous cutoff proof at
[D5:206–246](2026-10-14-r2-daily-input-design-amendments.md#L206) remove the generic
batch-date join and admission/scheduler cycle. The projection can run while daily mode
is OFF; calendar failure returns a typed gap, never another chk-holiday request or
a weekday inference. Original Raw availability is a later inventory question, not
an absent source-design mechanism. SQL trusts only the isolated verifier's submitted
proof; it does not claim to reopen provider files itself.

### Grants, publication, recovery and DOWN

[0009:27–33](../../migrations/0009_grants.up.sql#L27) grants app table-level config
writes; [0010:49–58](../../migrations/0010_rls.up.sql#L49) uses permissive actor RLS.
D5 [lines 254–283](2026-10-14-r2-daily-input-design-amendments.md#L254) explicitly removes
the grant bypass and adds actual locked user_roles checks, rather than another
permissive policy. Its retained columns match the existing config INSERT/UPDATE at
[strategy_configs.rs:55](../../crates/api-server/src/repos/strategy_configs.rs#L55)
and [117](../../crates/api-server/src/repos/strategy_configs.rs#L117). Epoch changes,
identity freezing and OFF-after-revocation are defined; release control/lease epochs
remain serving-role-inaccessible.

D5 [lines 307–378](2026-10-14-r2-daily-input-design-amendments.md#L307) cover the output
DML hole left by [0049:139–148](../../migrations/0049_owner_beta_price_recommendations.up.sql#L139)
and [0052:18–21/47–50](../../migrations/0052_owner_beta_strategy_config_lock.up.sql#L18).
The new receipt must derive from RUNNING job/current RUNNING attempt with count,
worker and unexpired DB-clock lease equality, before queue success. That respects
the existing item→run-success→queue-success order at
[publish.rs:380–442](../../crates/job-queue/src/owner_beta/publish.rs#L380), including
DB stamping that avoids transaction-start `now()` mismatches. Terminal outputs/items
are separately immutable; final checks cannot accept an arbitrary older success.

The common shared/exclusive transaction advisory source lock at
[D5:350–358](2026-10-14-r2-daily-input-design-amendments.md#L350) orders absent-event
checks against event insertion; control/user/rights/config precede it and ancestor
locks have stable order. D1's no mutation-authority→job locking requirement remains
at [D1:472–476](2026-10-14-r2-daily-input-design.md#L472). Actual two-connection revoke,
source-event and publication behavior is still required evidence.

D5 [lines 380–414](2026-10-14-r2-daily-input-design-amendments.md#L380) explicitly allow
failure's job-then-attempt order ([queue.rs:725–763](../../crates/job-queue/src/queue.rs#L725))
and orphan recovery's attempt-then-job order
([recovery.rs:344–394](../../crates/job-queue/src/owner_beta/recovery.rs#L344)). Cancellation
may commit before drain, with only one-way cleanup later; failed terminal mirrors
must commit together. Implement the daily immediate/deferred checks together; applying
only the final-state table or only the success trigger is insufficient.
An evidence qualification: D5:411–412's attempt-count/claimed_by recovery check is a
**new required condition**, not an existing verified check at those citations.
The baseline [RecoveryAttempt type](../../crates/job-queue/src/owner_beta/recovery.rs#L124)
and [latest_attempt query](../../crates/job-queue/src/owner_beta/recovery.rs#L464) do not
even load claimed_by. I2's daily guards and I4's owned recovery changes must enforce
the stated condition; no existing runtime success is inferred from that sentence.

D5 [lines 418–443](2026-10-14-r2-daily-input-design-amendments.md#L418) require locked,
all-tenant inspection with owned-table NO FORCE RLS, refusal after any durable daily
use/epoch, exact column-ACL removal/table-ACL restoration, and FORCE RLS restoration.
A second Owner must not be hidden by actor RLS. Populated daily histories require a
compatible schema0060 reader, including OFF rollback. None of UP/DOWN, effective ACL
restoration or deployment compatibility was executed.

### First-migration readiness and durable verification outcomes

D5 [lines 447–507](2026-10-14-r2-daily-input-design-amendments.md#L447) and
[669–680](2026-10-14-r2-daily-input-design-amendments.md#L669) own the previously missing
integration surfaces: bootstrap's fixed role/grant map
([bootstrap-roles.sh:66](../../deploy/db/bootstrap-roles.sh#L66)), official runner role
mapping/secrets/login lists/test inventory
([validate.sh:633](../../deploy/db/integration-validation/validate.sh#L633),
[1029](../../deploy/db/integration-validation/validate.sh#L1029),
[1315](../../deploy/db/integration-validation/validate.sh#L1315)), both
[job-queue](../../crates/job-queue/tests/common/mod.rs#L33) and
[API](../../crates/api-server/tests/common/mod.rs#L46) independent bootstrap copies,
secret provisioning and [production runtime_specs](../../scripts/ops/validate-production-config.sh#L828).
Membership/SET-role/attributes/PUBLIC checks, direct LOGIN CONNECT/USAGE without CREATE,
UID/mount isolation and no-SKIP lanes are mandatory later checks. Generated OpenAPI
ownership is correctly sequential because [openapi-check.mjs:41](../../apps/api-server/scripts/openapi-check.mjs#L41)
and [86](../../apps/api-server/scripts/openapi-check.mjs#L86) write both outputs.

D5 [lines 528–572](2026-10-14-r2-daily-input-design-amendments.md#L528) specify a new
read-only exact-line snapshot seam under the existing shared commit lock, trusted Raw
owner/descriptor checks and pure reconstruction without random BatchId or persistence.
Existing [storage.rs:1113–1180](../../crates/market-data/src/storage.rs#L1113) returns
parsed entries rather than exact lines and has an euid owner restriction at
[418–421](../../crates/market-data/src/storage.rs#L418); the new seam/installer permissions
must be implemented, not assumed already available. A6's recursively sorted internal
document encoding is distinct from D4's struct-key wire encoding. D6-01 is the remaining
concrete row mapping conflict found at that interface.

[D5:579–629](2026-10-14-r2-daily-input-design-amendments.md#L579) replaces inserted=false
ambiguity with three typed outcomes, nullable SQL result shape and committed conflict
events. Receipt allocation precedes scanning; completion/event/admission/poll-state
share a transaction, completion is immutable and lost replies reread durable identity.
BLOCKED cannot become VERIFIED, and failure receipts claim no admission. Stage locks
and 60-second persistent slots address overlap/restarts; 180-second freshness does not
claim detection of future/unobserved Raw appends. These are specified future semantics.

## Final D4 integration checks

1. **SinglePage marker correction is consistent.** D4
   [239–243](../contracts/owner-beta-daily-etf11-input-v1.md#L239) and D5
   [154–155](2026-10-14-r2-daily-input-design-amendments.md#L154) retain response markers,
   while blank request continuation and no following remain mandatory. Actual provider
   [643–674](../../crates/market-data/src/providers/kis.rs#L643) records the marker and
   follows only KsdGithubSample plus exact M. KSD last M still requires the next page.
2. **Witness/selected counts are separate.** D4
   [370–374](../contracts/owner-beta-daily-etf11-input-v1.md#L370) bounds the opaque
   normalized action count by 0..sum of witness cash rows; admission counts retain all
   witness occurrences. D5 [60–76](2026-10-14-r2-daily-input-design-amendments.md#L60)
   and [565–571](2026-10-14-r2-daily-input-design-amendments.md#L565) select one semantic
   contribution while retaining compared source keys. I1a checks the bound; I1b checks
   exact semantic count/bytes. D6-01 is independent of this corrected count rule.
3. **Legacy calendar hash is independent.** D4
   [327–334](../contracts/owner-beta-daily-etf11-input-v1.md#L327) separates legacy
   normalized hash from the new reconstructed commitment. The old sessions/holidays
   document at [normalize.rs:1171–1211](../../crates/market-data/src/normalize.rs#L1171)
   differs from D5 [564](2026-10-14-r2-daily-input-design-amendments.md#L564)'s daily
   date/type/evidence-kind row. I1a treats each opaque hash structurally; later legacy
   verification matches its original tuple/bytes. No hash equality is demanded.

Final D4's public inventory is 25 table/draft structs, four domain enums plus its error
enum, 17 public functions and two timestamp methods. Local in-memory Python execution
of the inspected normative JSON/hash construction recomputed all seven table entries:
policy 1827, evidence 44176, normalization 8790, content-preimage 45616, manifest 45707,
admission 1353 and empty cash 2 bytes; each hash matched
[D4:772–778](../contracts/owner-beta-daily-etf11-input-v1.md#L772).
This is document/preimage arithmetic, not parser, Rust or provider validation.
The original 876-line D4 was not supplied as a separately available review artifact;
no independent pre-correction field-by-field comparison is claimed. The supplied final
hash and reported final inventory/goldens were verified, with the three correction
rules assessed directly against source above.

## Checks, follow-up and handoff limits

Actual checks: bounded rg/nl/sed source and document reads; read-only Git HEAD/status
and baseline/path diff checks; SHA-256/line counts; final D4 normative preimage
sizes/hashes and declaration inventory; relative-link/source-line bounds, balanced
fences and whitespace checks on this report. All four frozen line counts/hashes match;
all seven final D4 sizes/hashes match; all 66 relative file/line links resolve and the
report has balanced fences and no trailing whitespace. The read-only Git diff checks
and scoped git diff --check exited 0 (the report is untracked, so its own whitespace
was checked separately). These results establish source/document consistency only.

Coordinator follow-up: resolve D6-01's narrow D4 rule and bind it into the affected
I1a/I1b specifications; retain D5's new recovery identity requirement as future
implementation, not existing source evidence; integrate these source-design closures
and later package ownership. Later isolated actual-role/UID, no-write reconstruction,
queue settlement/race/DOWN/API/historical regression and exact-candidate CI verification
remain necessary. G4 inventory, actual role/rights/pins, acquisition/date semantics,
aggregate budget/frontier/deadline, release and operational acceptance remain separate.
This review neither authorizes those actions nor renews already-adopted U2 permissions.

Changed file: this report only; complete line range is in the final handoff. Deviations
from the assignment: none. Missing architecture found within the bounded delta: the
exact dividend string mapping in D6-01; no other material gap identified. Not verified:
ongoing I1a source, compiler/tests/build, DB/ACL/RLS/lock/receipt behavior, real filesystem
permissions/UIDs, provider/production inventory or calls, deployment/runtime/Owner
behavior and original D4 before/after comparison. Other not-found/unreported items: none.
Further decomposition of D6: none; the correction is bounded and implementation
packages are already distinguished. No background work, permission waits, native
subagent/Task/Agent/team/delegation, Git writes or product/document edits outside this
owned report. Memory uses the real hook envelope only; this unadopted review is not
stored as an adopted decision or progress memory.
