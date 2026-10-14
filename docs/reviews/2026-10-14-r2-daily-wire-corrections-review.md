# WP-4/I1a-F-R — independent corrections source review

2026-10-14, Asia/Seoul. Direct review in
`/data/worktrees/3puw275b/excited-dolphin`, for ROADMAP R2-04/09.
Only this report was written. Coordinator owns source corrections, roadmap/Git
integration and hosted CI. No delegation was used; no further decomposition is
needed for this bounded review.

**Source acceptance still requires follow-up.** R01, R02, R03 and R06 are corrected
at source level. R04 remains exposed to strict test-target lint patterns; R05 has
two deterministic written-test failures and incomplete output-bound coverage.
These conclusions are static review, not compiler, Clippy or test results.
All 24 integration tests and five private unit tests remain unexecuted in this review.

## Inputs and exact checks

Read repository AGENTS.md, ROADMAP R2-04/09, complete D4, the complete original
review, CI diagnostics and the complete final I1a-F-R brief in the implementation
plan. Reviewed both source deltas against
`779b3faa787922032c682b9cf3fe07675a3d976f`, including all new private tests,
and followed the relevant unchanged validation, fixture and domain code.

| Fixed input | Lines | SHA-256 checked against assignment |
| --- | ---: | --- |
| [contract.rs](../../crates/market-data/src/daily_price_only/contract.rs) | 4163 | `79370bc25c927aa0d4b19e9646ab1bccfb845b8980a800217282a172901ccf2a` |
| [daily_price_only.rs](../../crates/market-data/tests/daily_price_only.rs) | 2283 | `ea08ac3ae55c7a4b667556cec100cd25ac32298d2d21a7c209b490c412f355e4` |
| [D4](../contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| [Original review](2026-10-14-r2-daily-wire-review.md) | 316 | `92da4c046f8653075a4fb8631475eb37d49b7ee4e1eee357ffe856f1538b7540` |

All four matched at entry and final stability checks. Baseline Git blobs match the
original review: implementation 4057 lines,
`4cf96f83b5a91e4891975e6fe577aa29a24fce92ce05b3c118cd9e4531285833`;
tests 1788 lines,
`860c728b9c76c838acbf95d4ab7a15967a0b8c7498d9cbc700ebeda9841973f1`.
D4 is byte-identical to its baseline blob. The original review is a later working-tree
input, absent from that baseline commit; its supplied working-tree hash was verified.

Supporting checks: module registration remains 8 lines,
`e143131e3bc23628179d6f2fe2bff0f0539b960f86d311b5ad9627f1610b2135`;
lib.rs remains 288 lines,
`fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75`.
[CI diagnostics](2026-10-14-r2-daily-wire-ci-diagnostics.md) is 46 lines,
`a057968081b81e351ada955efc4881c9a21a3df6d09f2f3fc88e0d860092425d`.
Entry HEAD was `f85c3bb1aae40e94fc092f7a1e61bb4c1c957a51`; coordinator-owned HEAD
advanced to `cf433db342066053d40962d45c6e07bd3f6164f9` during review while the
fixed hashes stayed unchanged. Source hashes identify this review, not HEAD.

The diagnostics establish the old run's E0369 timestamp comparison and three E0282
Option-inference errors, plus strict unused-mutability failure. Chart and
api-tenancy-rls did not execute; the later workspace command was skipped.
No old or new hosted CI was queried here, and old failure evidence is not evidence
that any of the corrected 29 tests ran.

## Original finding dispositions

| Original finding | Source disposition | Concrete evidence and remaining limitation |
| --- | --- | --- |
| R01 cumulative counts | Corrected; execution pending | [derive_counts:3219](../../crates/market-data/src/daily_price_only/contract.rs#L3219) takes total session count, uses checked conversion and multiplication by 11; evidence counts stay in derive_evidence_counts. Manifest payload and [bundle:3977](../../crates/market-data/src/daily_price_only/contract.rs#L3977) pass manifest.sessions.len(). [Test:1123](../../crates/market-data/tests/daily_price_only.rs#L1123) inserts a third historical date, expects 5/55 total versus 2 added, and rejects consistent-but-wrong 4/44 counts before pins comparison. |
| R02 timestamp/types | Corrected; execution pending | [Comparison:2353](../../crates/market-data/src/daily_price_only/contract.rs#L2353) compares inner UtcTimestamp values. Explicit Option types occur at 2590, 3024 and 3585. [Test:1900](../../crates/market-data/tests/daily_price_only.rs#L1900) changes every shared calendar occurrence and tests a two-hop nanosecond inversion. The separate 16-hop positive test is defective, CR-02. |
| R03 proof date parity | Corrected; execution pending | [Proof:3596](../../crates/market-data/src/daily_price_only/contract.rs#L3596) validates each document date before its inner hash. [Helper:634](../../crates/market-data/tests/daily_price_only.rs#L634) recomputes the actual commitment using serde_json; [test:1617](../../crates/market-data/tests/daily_price_only.rs#L1617) covers years 0/1/9999/10000 in typed and JSON paths. Invalid dates are not hidden behind stale commitments. |
| R04 strict warnings | Partially corrected; remains open | Old dead helpers/redundant branch/mutability are removed; slice sorting, io::Error::other and private argument/tuple types replace the old patterns. The new private guard tables and cloned slice arguments introduce remaining lint exposure, CR-03. |
| R05 resource regressions | Partially corrected; remains open | All fixed guards have identifiable production call sites and substantially improved written pairs. CR-01/02 make two new positive/boundary assertions fail; CR-04 identifies remaining output coverage. The matrix below distinguishes public paths from isolated helpers. |
| R06 final seal bytes | Corrected; execution pending | [Seal:3729](../../crates/market-data/src/daily_price_only/contract.rs#L3729) serializes the final value with the unchanged manifest byte limit before returning. [Test:1201](../../crates/market-data/tests/daily_price_only.rs#L1201) has a legal near-limit rejection/success pair; independent byte calculations below support its construction. |

R01 has no hardcoded historical-prefix length in derivation. Admission's unchanged
`added <= total-2` rule is D4's local minimum-prefix bound, not an equality claiming
exactly two historical dates. Arbitrary valid historical prefixes contribute their
actual manifest length; verifying the real historical prefix remains outside I1a.

R02 retains equal-time acceptance and rejects a newer origin: the rule is
nonincreasing current-to-original time. Domain UtcTimestamp has PartialOrd/Ord over
chrono DateTime<Utc>; as_timestamp() preserves the nanoseconds. The new two-hop test
uses distinct batch identities and 123456789→123456788→123456787 ns; inversion changes
the first origin to 123456786 ns. Wrapper API and serialization are unchanged.

## CR-01 — civil-date helper permits the new test's bound+1

**P2/MEDIUM; deterministic private-test failure, no public limit bypass shown.**

[date_range:2523](../../crates/market-data/src/daily_price_only/contract.rs#L2523)
checks the existing vector length with an inclusive `actual <= 370` helper, then
pushes another date, then breaks when that date equals end. For start 2026-01-01
and end=start+370 days, the final iteration begins at length 370, passes the check,
pushes entry 371 and returns Ok. The new
[assertion:4101](../../crates/market-data/src/daily_price_only/contract.rs#L4101)
expects LimitExceeded for exactly this call. Its preceding 370-date case is legal.

The public [validate_session:3000](../../crates/market-data/src/daily_price_only/contract.rs#L3000)
checks the returned length again and rejects 371. The former implementation also
allowed this private helper's terminal 371st entry; the delta introduces a test that
now exposes it. This is not a demonstrated new externally accepted oversized session.

Bounded correction: validate the proposed next length before pushing, or check the
post-push length before the terminal break. Preserve the exact 370/371 test and public
LimitExceeded behavior. No contract change or large fixture is needed.

## CR-02 — 16-hop positive fixture conflicts with shared file metadata

**P2/MEDIUM; deterministic integration-test failure and missing valid 16-hop positive.**

[Test:1821](../../crates/market-data/tests/daily_price_only.rs#L1821) applies
set_copy_depth only to `calendar[0].source_file`, then asserts that the public evidence
preimage succeeds. make_session creates all three bindings by cloning the same
calendar file. Their current provider/market/batch/name identity is unchanged by
set_copy_depth, but the first now has 16 origins and a different original timestamp,
while the other two still have empty copied_from and the current original timestamp.

[compare_file_identity_metadata:2765](../../crates/market-data/src/daily_price_only/contract.rs#L2765)
requires complete FileEvidence equality for a reused identity. The second calendar
occurrence therefore returns InconsistentFields during validate_session, reached
from daily_price_evidence_preimage_v1 through derive_evidence_counts. The is_ok()
assertion cannot succeed, so the later 17-hop assertion is not reached in that test.
This failure is neither a copy-depth rejection nor a stale source-commitment error.

Bounded correction: replace all shared occurrences consistently, as the new two-hop
test already does. Top-level FileKeys remain unchanged here, so the normalized source
commitment need not change solely because origins were added. Retain exact 16/17
public-path assertions. The distinct 10-page fixture is correctly wired: unique names
and wire hashes, initial blank tr_cont, later N, M before the terminal F, and refreshed
corporate source keys/commitment; 11 pages is rejected in preflight.

## CR-03 — new test targets retain strict-lint exposure

**P2/MEDIUM; source lint finding, actual Clippy diagnostics unverified.**

The all-targets/all-features lane uses `-D warnings` at
[ci.yml:79](../../.github/workflows/ci.yml#L79). New cfg(test) code is in that scope.
Both [guard table:4003](../../crates/market-data/src/daily_price_only/contract.rs#L4003)
and [size table:4051](../../crates/market-data/src/daily_price_only/contract.rs#L4051)
spell nested array/tuple/function-pointer/Result types directly, retaining the
`clippy::type_complexity` pattern that the production aliases were intended to remove.
Neither definition has a narrow allowance or a private type alias.

New [1796](../../crates/market-data/tests/daily_price_only.rs#L1796),
[1822](../../crates/market-data/tests/daily_price_only.rs#L1822) and
[1907](../../crates/market-data/tests/daily_price_only.rs#L1907) pass
`&[owned_value.clone()]` to an API accepting a borrowed slice, the
`clippy::cloned_ref_to_slice_refs` pattern; std::slice::from_ref supplies the same
borrowed input without cloning. The older copied-calendar case has this pattern too.
No applicable crate-wide or local allowance/config override was found in the reviewed
files. These are concrete source patterns, not claimed output from a Clippy run.

Bounded correction: introduce private aliases or a small named table-entry type,
and use from_ref for singleton borrowed calls. Keep the strict lane and frozen public
API intact. The original unused mutability and missing comparison/inference source
conditions are removed, but this review cannot declare all-target strict lint PASS.

## CR-04 — output-byte coverage is still partial

**P2/MEDIUM; adopted regression gap, no output-guard bypass demonstrated.**

[D4:864](../contracts/owner-beta-daily-etf11-input-v1.md#L864) requires each fixed
guard at bound/bound+1, including canonical output. The new public input test at
[1098](../../crates/market-data/tests/daily_price_only.rs#L1098) pads valid JSON with
whitespace and proves the four incoming limits; canonicalization removes that padding,
so these are not oversized canonical-output tests.

The private [writer test:4137](../../crates/market-data/src/daily_price_only/contract.rs#L4137)
checks generic limits 8/7 and the actual max_manifest_bytes value at exact/+1.
The public near-limit pair checks manifest/seal output. There is no corresponding
fixed max_policy_bytes or max_admission_bytes writer pair, or public proof-output
overflow pair. max_normalization_bytes equals max_manifest_bytes numerically, so the
isolated 16 MiB writer check covers that numeric threshold, but cannot detect wrong
limit wiring at encode_daily_normalization_proof_v1. Existing public proof encoding
assertions are small successful values.

Valid policy canonical size is fixed at 1827 bytes; admission's bounded scalars and
fixed strings cannot approach 1 MiB. Do not fabricate invalid policies/admissions and
count their literal/hash rejection as byte evidence. Use explicitly mapped isolated
writer pairs for those unreachable public sizes, retain public entry-point tests,
and add a bounded valid proof overflow or clearly document the remaining public-path
gap. No larger product permission or external inventory is required.

## Guard-to-written-test matrix

All entries below are written/static evidence; none is a Rust PASS. `C` denotes
contract.rs; `T` denotes tests/daily_price_only.rs. Links above identify those files.
Private isolated helpers are permitted by the brief; they do not establish public
schema legality or execute the caller's wiring.

| Fixed guard | Production wiring | Bound / +1 written evidence and limits |
| --- | --- | --- |
| policy input 262144 | C3657→parse_typed→C670 | T1098/T894 public whitespace-padded legal JSON pair. |
| manifest input 16777216 | C3693→parse_typed→C670 | Same public input matrix. |
| admission input 1048576 | C3744→parse_typed→C670 | Same public input matrix. |
| proof input 16777216 | C3848→parse_typed→C670 | Same public input matrix. |
| policy/admission canonical output | C2201/C3758→bounded writer C1778 | Generic writer pair only; fixed-size isolated mapping absent, CR-04. |
| manifest canonical output | C3681/C3714/C3729→writer | C4137 exact 16 MiB/+1 string; T1201 public final near-limit pair. |
| proof canonical output 16777216 | C3854/C3869/C3912→writer | Shared numeric 16 MiB writer pair; proof public overflow untested, CR-04. |
| bars declared size 134217728 | C3334→check_bars_descriptor_size | C4049 isolated size pair; T1158 legal draft declared-size pair. No bar bytes allocated. |
| normalization descriptor size 16777216 | C3339→check_normalization_descriptor_size | Same C4049/T1158; descriptor claims are opaque outside a bundle. |
| sessions 20000 | C1895/1896/C3277; admission C3466 | C4001 isolated count pair; T1965 admission upper bound, T1920 admission +1. No 20000-session public array acceptance claimed. |
| civil dates 370 | C1854/C2523/C3000 | C4001 isolated pair; C4084 date_range 370/371 fails as CR-01. No legal public 370-date session fixture claimed. |
| action chains 32768 | C1855/C3023 | C4001 isolated pair; T1965 tests checked admission aggregate upper/+1, not a per-session array. |
| file occurrences 262144 | C1882 checked sum; C3051 | C4001 isolated pair; C4109 scanner array exact/+1; T1965 admission aggregate pair. Numeric JSON array is not legal FileEvidence. |
| source declared bytes 67108864 | C2265 for current/origin | C4049 isolated pair; T1158 legal draft current-source size pair. No provider bytes read. |
| normalized document size 67108864 | C2943/C3605 | C4049 isolated pair; T1158 legal draft document size pair; proof-specific bound pair not separately written. |
| copied depth 16 | C1841/C2322 | C4001 isolated pair; T1774 intended public 16/17 pair fails at positive case, CR-02. T1900 valid two-hop nanosecond case. |
| action pages 10 | C1873/C2646 | C4001 isolated pair; T1774 legal 10-page/+1 public evidence pair. |
| cash rows 100000 | C1874/C2589/C3825 | C4001 isolated pair; T1965 aggregate upper/+1. No 100000-row public success claimed; serialized rows can encounter the manifest byte limit first. |
| JSON container depth 32 | C577/C608→check_container_depth | C4084 parses 32/33 nested arrays before schema checks; old T1566 depth overflow remains. Scalars add no depth. |

Other preflight calls bound query/header lists, instruments, proof documents and
source-key lists through the file-count helper; session normalized-document count
uses checked coverage and a `3+370` preflight bound. Call-site inspection confirms
the extracted helpers are used; isolated function-pointer invocation alone would
not prove that. No duplicate/hash/shape error is counted as a resource-limit PASS.

## R06 near-limit fixture legality and resource calculation

Independently evaluated D4's pure Python golden construction in memory, then
calculated the exact JSON byte-size effect of the new fixture recipe at
[991](../../crates/market-data/tests/daily_price_only.rs#L991) and
[1201](../../crates/market-data/tests/daily_price_only.rs#L1201). This did not execute
the Rust fixture, parser, validator or serializer.

The loop produces 937 extra dividend chains: 944 total actions, 969 top-level file
occurrences, 967 unique top-level files and 14992 copied origins. Each extra chain
has one terminal page, 16 distinct older origins and a unique current UUID; UUID
blocks stride by 17, preventing cross-chain origin/current identity collisions.
All use valid ETF symbol 069500, dividend request extras and the required civil
interval. The seven original whole-market chains continue to provide full coverage.
Distinct first-file identities permit the overlapping extra witnesses under D4.
The refreshed corporate document has 944 unique source keys and a matching source
commitment; its selected row_count remains zero, consistent with empty cash rows.

Estimated pre-trim bytes: 16786995. Removing 548 filename characters across 20
chains changes 18 filename occurrences per character (current + 16 origins + key).
Names remain 100..128 ASCII bytes; declared wire sizes remain three-digit values,
so the 18-byte adjustment is exact. Calculation yields:

| Pair | Draft bytes | Final bytes (+91) | Relation to 16777216 |
| --- | ---: | ---: | --- |
| Rejected final manifest | 16777131 | 16777222 | Draft legal; final exceeds by 6. |
| After one more filename character removal | 16777113 | 16777204 | Final legal; 12 bytes below limit. |

The append-content-hash construction is valid because that member is last and adds
91 bytes. Canonical sorting changes order, not these lengths. Source/document size
claims remain small; all count/depth guards are below or at their limits. This is
a legal isolated draft/seal test, not a matching external normalization bundle or
trusted data fixture. Its unchanged normalization descriptor need not describe the
enlarged document until a bundle is constructed.

Resource size is finite: roughly 16 MiB canonical draft and 14992 allocated origin
objects, rather than 32768 expanded chains. Each seal/validation call can clone the
full draft and build metadata maps containing further copies; several byte vectors
also coexist. Extending from extra_actions consumes that vector.
Peak RSS and duration are unmeasured and must come from hosted execution; serialized
size must not be reported as peak memory. No concrete resource-limit illegality or
unbounded construction was identified in this fixture.

## Preserved contract and actual checks

Static extraction/comparison against the baseline confirms the 25 wire struct macro
declarations and field order, four literal enums, payload-free error enum, 17 public
free-function signatures and two timestamp-method signatures are unchanged.
No approved type, Raw/file/registry/DB/provider/runtime capability was added.
Calendar legacy hashes, SinglePage markers and witness-versus-selected cash counts
retain their original meanings. Historical/module-registration paths are unchanged
in the reviewed delta; historical tests were not executed.

The primary golden test and all three cash-code tests are byte-identical functions
to the baseline, apart from their moved line positions. All seven golden lengths/
hashes were independently reproduced from D4's pure Python construction:
1827/44176/8790/45616/45707/1353 bytes and literal `[]` at 2 bytes, matching the
assertions at [T1043](../../crates/market-data/tests/daily_price_only.rs#L1043).
Hashing the source cash literal at T26 confirms 515 bytes and
`0f87c41ba19ba820ee79a2a05f54c27b9a4af5f185a98b080bc429b56eaca864`.
The three cash tests at T1277/T1339/T1384 retain exact MissingField, InvalidType
and InvalidLiteral assertions, empty/32-byte success and the resealed bundle claim.
Those are written assertions and literal calculations, not Rust round-trip PASS.

Actual commands/checks: read-only Git blob/diff/hash inspection; source/document and
cached chrono 0.4.45 reads matching Cargo.lock; static API/test/golden comparisons;
in-memory Python JSON/hash/size calculations; `rustfmt --edition 2024 --check` on
only the two fixed Rust files (exit 0); scoped `git diff --check 779b3faa...` on
those files (exit 0); report line/link/fence/whitespace and final hash stability checks.
No Cargo/rustc/Clippy/test/build, DB/SQL, root/sudo, Docker, external network/provider,
operations/workflow action or Git write was performed. No background process or
permission wait occurred. The supplied memory hook alone contacted loopback and
wrote its operational receipts, as explicitly allowed by the request.

The focused context helper returned MCP_ERROR/no response, so no prior-memory result
was used. The same request's finish returned SKIPPED, saved=false, incomplete=false,
with no_durable_change. No findings were stored as adopted durable project decisions.
One optional cached chrono serde.rs lookup missed; the serializer was found in
naive/date/mod.rs. The baseline review-blob lookup failed because the report postdates
that commit; the required working-tree report was present and hash-correct.

## Follow-up, deviations and unverified work

Coordinator follow-up: fix CR-01/02, clean up CR-03 and complete or explicitly account
for CR-04's fixed output-guard coverage. Preserve frozen D4/API/goldens, bind the new
source hashes, then combine a fresh source review with exact-candidate hosted
compiler/strict-all-targets-Clippy and named test results. R01/02/03/06 source closure
does not establish those execution results or full R2-04/09 acceptance.

Deviations from the assignment: none. Roadmap/other-document writes remain coordinator
owned under the explicit report-only scope. Missing fixed inputs: none. Missing or
conflicting specification requiring an invented rule: none; the identified code/test
mismatches and coverage gaps are stated above. Other unreported missing evidence: none.

Unverified: all 29 Rust tests, corrected compiler diagnostics, actual Clippy warnings,
runtime near-limit fixture behavior, peak RSS/duration, historical v2/v3 execution,
new hosted CI results and final coordinator acceptance. I0/0060/runtime, DB/roles/SQL,
Raw/external bytes/registry trust, deployment, provider acquisition and Owner acceptance
are excluded and unverified. No permission request or background wait is outstanding.
Further decomposition needed: none; the remaining changes are local and bounded.
Report line range and SHA-256 are supplied in the final handoff.
