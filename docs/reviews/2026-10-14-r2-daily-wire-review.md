# R2 WP-4 I1a-R — independent pure-wire source review

2026-10-14, Asia/Seoul. Bounded intermediate/high-confidence review, assigned
codex/gpt-6.1-sol high. Checkout `/data/worktrees/3puw275b/excited-dolphin`, workspace
`wks_10fae7d550ab1704`. Scope: complete I1a plus D6-01 correction, R2-04/09.
Coordinator owns acceptance, source fixes, documentation/Git integration and hosted CI.
No delegation or execution verification was performed by this reviewer.

**Source acceptance requires follow-up: two P1/HIGH and four P2/MEDIUM findings.**
The bundle-count contradiction and missing timestamp ordering trait prevent accepting
the frozen implementation. Other findings concern typed proof/date parity, strict
warnings, missing required boundary regressions and the seal output byte bound.
These are source-derived conclusions and test gaps, not reported compiler/test runs.
D6-01's three present-empty code rules are consistent in the reviewed parse/typed
paths; its written bundle regression remains affected by I1a-R01.

## Frozen inputs and scope

Read applicable AGENTS, ROADMAP, [adopted U2](2026-10-13-r2-daily-input-decision.md),
the final [I1a-R brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L447)
and [CI ownership brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L462).
Reviewed the complete bounded implementation and test source, with relevant domain
types and local cached dependency source. I0a-R/D6 were neither redone nor edited.

| Exact input | Lines | SHA-256 |
| --- | ---: | --- |
| [D4 contract](../contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| [Module registration](../../crates/market-data/src/daily_price_only/mod.rs) | 8 | `e143131e3bc23628179d6f2fe2bff0f0539b960f86d311b5ad9627f1610b2135` |
| [Wire implementation](../../crates/market-data/src/daily_price_only/contract.rs) | 4057 | `4cf96f83b5a91e4891975e6fe577aa29a24fce92ce05b3c118cd9e4531285833` |
| [Library registration](../../crates/market-data/src/lib.rs) | 288 | `fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75` |
| [Written regressions](../../crates/market-data/tests/daily_price_only.rs) | 1788 | `860c728b9c76c838acbf95d4ab7a15967a0b8c7498d9cbc700ebeda9841973f1` |

Initial HEAD was `11c4829931909b102b409f09f561cb60d222b39e`; coordinator integration
advanced HEAD to `919d59880096af7b04b215851a07ba070536c8fd` during review. Hashes, rather
than an unchanged HEAD, identify the review inputs. They matched initially and on the
later stability check. lib.rs adds only `pub mod daily_price_only;` against `11c4829`.
The compared market-data source/test/Cargo and root Cargo.lock paths contain only the
four intended source/test changes. No historical implementation or dependency change
was identified in that comparison. No hosted CI was queried or adopted as evidence.

## I1a-R01 — bundle counts exclude the historical prefix

**Severity:** P1/HIGH, verified source logic defect. **Affected:** pure bundle validator.

D4 [436–438](../contracts/owner-beta-daily-etf11-input-v1.md#L436) distinguishes total
manifest session_count/bar_count from added_session_count. However,
[derive_counts:3241–3246](../../crates/market-data/src/daily_price_only/contract.rs#L3241)
sets session_count to the length of the DailySessionEvidence slice and bar_count to
eleven times that number. The bundle passes only canonical_manifest.daily_sources
and compares all derived fields to admission.counts at
[4041–4048](../../crates/market-data/src/daily_price_only/contract.rs#L4041).

**Reachable counterexample:** the normative first-addition fixture contains three
manifest sessions and one daily source. Its valid admission therefore has 3 sessions
and 33 bars, as the test helper explicitly constructs at
[daily_price_only.rs:615–621](../../crates/market-data/tests/daily_price_only.rs#L615).
The validator compares 1/11 against 3/33 and returns InconsistentFields. Any valid
manifest has at least the two retained historical dates, so the mismatch is not
limited to that fixture. The existing golden bundle assertion at
[922–929](../../crates/market-data/tests/daily_price_only.rs#L922), present-empty cash
bundle at [984–991](../../crates/market-data/tests/daily_price_only.rs#L984) and other
positive bundle cases cannot succeed on this source path once executable. They were
not run here; this conclusion follows from the compared fields.

**Bounded fix:** derive total session_count from canonical_manifest.sessions.len()
and bar_count from that total using checked arithmetic, while retaining separately
derived added-session/evidence counts. Do not add a hardcoded 2: real approved history
has 2,452 base sessions and the small fixture deliberately models only two. Preserve
the seven golden values and run the existing positive bundles plus an exact-count
rejection using a manifest with more than two base dates.

## I1a-R02 — timestamp comparison has no required ordering implementation

**Severity:** P1/HIGH, verified source compile defect; no compiler invocation.
**Affected:** wire timestamp/copy validation and unconditional module compilation.

[DailyWireTimestampV1:98–99](../../crates/market-data/src/daily_price_only/contract.rs#L98)
derives Clone, Copy, Debug, PartialEq and Eq, but no PartialOrd. Its only explicit
implementations are parse/accessors, Serialize and Deserialize. Nevertheless,
[validate_file_evidence:2359](../../crates/market-data/src/daily_price_only/contract.rs#L2359)
uses `prior_time < origin.retrieved_at`, where both operands have that wrapper type.
The inner [UtcTimestamp](../../crates/domain/src/time.rs#L81) has ordering, but a Rust
newtype does not inherit that trait automatically.

**Reachable failure:** the unconditionally registered module must type-check this
function even when a fixture has no copied files. Binary `<` requires PartialOrd,
which is absent. This is an unsatisfied source trait requirement, not a compiler
diagnostic observed in this review.

**Bounded fix:** compare the wrapped UtcTimestamp values through as_timestamp(), or
provide the wrapper's appropriate ordering implementation. Preserve nanosecond
comparison and nonincreasing copy-time rejection; no wire field/hash change is needed.
Retain copied-calendar tests and add a genuine ordered multi-hop/time-inversion pair.

Additional compile-risk review: unannotated Option accumulators used as `.cmp`
receivers before their first type-constraining assignment occur at
[2600/2623–2630](../../crates/market-data/src/daily_price_only/contract.rs#L2600),
[3050/3064–3071](../../crates/market-data/src/daily_price_only/contract.rs#L3050) and
[3645–3655](../../crates/market-data/src/daily_price_only/contract.rs#L3645). They may
require explicit concrete Option types for inference. Actual diagnostics are reserved
for coordinator CI; this risk is not counted as a separately executed compiler finding.

## I1a-R03 — typed proof encoder can emit a date its parser rejects

**Severity:** P2/MEDIUM, verified source validation mismatch.
**Affected:** encode_daily_normalization_proof_v1 and shared proof validation.

D4 [38/48](../contracts/owner-beta-daily-etf11-input-v1.md#L38) requires typed encoder
validation and D dates restricted to years 0001..9999. The JSON path enforces that at
[1315–1327](../../crates/market-data/src/daily_price_only/contract.rs#L1315). By contrast,
[validate_proof:3635–3683](../../crates/market-data/src/daily_price_only/contract.rs#L3635)
never checks document.session_date with valid_domain_date; it directly serializes
the reconstructed source commitment. The public encoder calls this validator at
[3927–3933](../../crates/market-data/src/daily_price_only/contract.rs#L3927).

**Reachable counterexample:** construct a one-document calendar proof with
`TradingDate::new(0,1,1)`, otherwise valid fields/FileKey and a matching
source_evidence_sha256 computed over the claimed source commitment's canonical bytes.
[TradingDate::new](../../crates/domain/src/time.rs#L257) permits chrono's wider year
range; the cached locked chrono 0.4.45 source explicitly documents year 0 serialization.
The typed encoder's local checks accept the proof and emit `0000-01-01`; parsing those
bytes returns InvalidDate. The same issue covers domain dates above 9999. External
document truth is intentionally opaque, but the wire date scalar is not an external
trust gate.

**Bounded fix:** apply the same valid-domain-date restriction to every proof document,
or reuse the already validating normalized-source preimage helper for this inner
commitment. Add typed/parser parity cases at years 0, 1, 9999 and 10000 with matching
commitments, so rejection cannot be attributed only to stale hashes.

## I1a-R04 — source warnings conflict with the strict CI lane

**Severity:** P2/MEDIUM, source-level compiler/Clippy concern; no Clippy run.
**Affected:** implementation and its test helper warnings.

The existing workflow uses `cargo clippy ... -- -D warnings` at
[ci.yml:79](../../.github/workflows/ci.yml#L79). The reviewed module/test has no applicable
warning allowance. The coordinator's suspected conditional is concrete:
[1424–1450](../../crates/market-data/src/daily_price_only/contract.rs#L1424) executes
the identical `visit(child, Some(child_key))?` in both branches. The entire condition
is redundant, matching Clippy's if_same_then_else lint, regardless of input.

There are also directly visible Rust warning conditions: private
[shape_document_commitment:960](../../crates/market-data/src/daily_price_only/contract.rs#L960)
and [canonical_instruments:1482](../../crates/market-data/src/daily_price_only/contract.rs#L1482)
each occur only at their definition; and
[canonical_admission:3988](../../crates/market-data/src/daily_price_only/contract.rs#L3988)
is declared mut but never changed. The enum macro's private literal method is unused
for DailySessionTypeV1. Further recognizable Clippy targets include
[1590](../../crates/market-data/src/daily_price_only/contract.rs#L1590)'s Vec-only sort
parameter (ptr_arg), [1771–1774](../../crates/market-data/src/daily_price_only/contract.rs#L1771)'s
ErrorKind::Other constructor (io_other_error), the nested tuple return at
[1594–1605](../../crates/market-data/src/daily_price_only/contract.rs#L1594)
(type_complexity), and oversized private helper argument lists at
[2245](../../crates/market-data/src/daily_price_only/contract.rs#L2245),
[3258](../../crates/market-data/src/daily_price_only/contract.rs#L3258),
[test158](../../crates/market-data/tests/daily_price_only.rs#L158) and
[test299](../../crates/market-data/tests/daily_price_only.rs#L299) (too_many_arguments).

**Reachable failure:** these new definitions are compiled by the all-targets lane;
the strict warning policy makes warning cleanup part of source readiness. Rustfmt
only checks formatting and cannot establish a clean compiler/Clippy result.

**Bounded fix:** collapse the redundant branch, remove unused helpers/mutability and
clean up the listed local warning patterns. Preserve frozen public APIs/bytes; use
only narrow, justified allowances for unavoidable private schema-helper complexity
rather than disabling the strict lane. Actual final diagnostics remain CI-owned.

## I1a-R05 — required boundary regression matrix is incomplete

**Severity:** P2/MEDIUM, verified test gap, not a demonstrated runtime guard bypass.
**Affected:** tests/daily_price_only.rs source acceptance.

D4 [864–866](../contracts/owner-beta-daily-etf11-input-v1.md#L864) explicitly requires
each fixed guard at its bound and bound+1, including canonical output bounds.
The complete source contains 16 test functions. Its resource-specific checks at
[1222–1270](../../crates/market-data/tests/daily_price_only.rs#L1222) and
[1475–1516](../../crates/market-data/tests/daily_price_only.rs#L1475) cover oversized
policy input, depth 33, u64 overflow/-0, pages 11, copies 17 and admission sessions 20001.
They do not provide paired boundary cases for the declared source/document/bar size
guards, civil dates 370, action chains 32768, top-level file occurrences 262144 or cash
rows 100000; nor all four input/output byte limits. The pages/copies examples repeat
identities and therefore also do not constitute valid positive 10-page/16-hop cases.

**Reachable validation gap:** changing/removing the max_source_file_bytes or
max_normalized_document_bytes comparison currently has no corresponding boundary
regression in these 16 tests. Typed public outputs also lack a boundary case that
detects I1a-R06. This is a concrete missing adopted acceptance condition, not an
assertion that the visible guards were executed and failed.

**Bounded fix:** add focused parameterized boundary pairs using unapproved claims and
declared sizes, including valid identity/continuation/copy metadata where a positive
case is expected. Exercise each public output guard without external files. Where
another schema/byte limit makes a nominal positive case impossible, isolate the guard
and document the expected earlier rejection; do not count duplicate/hash/shape errors
as proof of the intended resource limit. Add the specific I1a-R03/R06 parity cases.
No provider, Raw inventory, DB or operational fixture is required.

## I1a-R06 — seal checks draft bytes but omits final manifest byte bound

**Severity:** P2/MEDIUM, verified source output-bound gap.
**Affected:** seal_daily_price_manifest_v1.

D4 [145–147](../contracts/owner-beta-daily-etf11-input-v1.md#L145) bounds canonical
output as well as input. The public seal function at
[3782–3788](../../crates/market-data/src/daily_price_only/contract.rs#L3782) obtains a
bounded draft preimage, appends content_sha256 and calls validate_manifest. That
validator checks only the draft's serialized bytes and content hash at
[3487–3492](../../crates/market-data/src/daily_price_only/contract.rs#L3487), then returns.
It never serializes/checks the final manifest's byte size on this return path.
The separate manifest encoder does that later at
[3740–3744](../../crates/market-data/src/daily_price_only/contract.rs#L3740).

**Reachable counterexample:** a structurally valid draft whose canonical size lies
in `(max_manifest_bytes-91,max_manifest_bytes]` passes the draft limit. Adding the
fixed content_sha256 member adds 91 bytes (also visible in the normative 45707 minus
45616 lengths). Seal can therefore return Ok with a manifest that its canonical
encoder subsequently rejects as LimitExceeded. Valid extra action witness chains
with distinct file identities and corresponding source commitments can grow a draft
into this band within the count guards; bounded filename lengths allow local size
adjustment. The failure depends on final byte size, not external evidence/trust.

**Bounded fix:** check the final canonical manifest byte bound before returning from
seal, without weakening the existing limit or changing preimages. Add a near-limit
draft/final-manifest pair proving that seal cannot return an unencodable result.
This is source reasoning; no near-limit fixture was executed here.

## Reviewed behavior and written regression evidence

The source scanner [468–690](../../crates/market-data/src/daily_price_only/contract.rs#L468)
runs before constructing Value, decodes object keys before duplicate comparison,
rejects escaped collisions/lone surrogates, checks complete JSON, unsigned token
forms/u64 range, byte limit and container depth. The later Value intermediate therefore
does not erase duplicates on the named parse entry points. Manual exact-key shape
checks cover nested objects and require nullable keys before typed deserialization.
Public serde-derived structs remain unapproved mutable claims, not trusted constructors.

The full scalar/struct/API inventory matches D4: 25 wire structs, four literal enums,
payload-free errors, 17 public free functions and two timestamp methods. Struct field
order is retained by Serialize; set arrays use explicit literal/tuple sorts rather
than enum declaration order. Pages/copy chains retain their order. Timestamp parsing
and serialization retain nine fractional digits; the missing comparison trait is R02.

Request validators [2039–2205](../../crates/market-data/src/daily_price_only/contract.rs#L2039)
enforce redacted five-header shape, exact paths/TR IDs and endpoint queries. Source
validators retain SinglePage response markers without following, enforce KSD M/N
page order, copy metadata/time/identity consistency, full per-cell civil/action union
coverage, exact ETF11 file sets, unique normalized commitments and matching internal
source hashes. These are structural claims only; no body/file/registry/approval path
was introduced. The separate legacy calendar hash and witness-vs-selected action
count rules are preserved. Bounded serialization exists, with the seal exception R06.

D6-01 uses the same valid_code predicate at
[1180–1184](../../crates/market-data/src/daily_price_only/contract.rs#L1180),
[1304–1307](../../crates/market-data/src/daily_price_only/contract.rs#L1304) and
[1543–1547](../../crates/market-data/src/daily_price_only/contract.rs#L1543).
Empty strings remain unchanged; unsupported code characters reach InvalidLiteral
before generic control-character validation. Shape/type stages distinguish missing
and null/non-string. The three new written tests at
[933](../../crates/market-data/tests/daily_price_only.rs#L933),
[995](../../crates/market-data/tests/daily_price_only.rs#L995) and
[1040](../../crates/market-data/tests/daily_price_only.rs#L1040) cover the 515-byte golden,
empty/32-byte values, exact missing/null/type/33-byte/whitespace/control/non-ASCII/
unsupported-character errors in JSON and typed paths. Invalid lexical cases are
checked before stale hashes rather than accepted as HashMismatch-only rejections.

All seven normative golden hashes and their six serialized-size assertions plus
literal `[]` are present at
[878–929](../../crates/market-data/tests/daily_price_only.rs#L878), matching
[D4:780–786](../contracts/owner-beta-daily-etf11-input-v1.md#L780). Local hashing of the
test's literal present-empty row confirms 515 bytes and
`0f87c41ba19ba820ee79a2a05f54c27b9a4af5f185a98b080bc429b56eaca864`.
This hashes source literal bytes; it does not execute the Rust serializer/tests.
Existing tests also write meaningful reorder, duplicate/escaped key, shape/type,
nanosecond, request/civil gap, page/copy, legacy hash, overlapping witness,
opaque-external-hash and parent/policy/producer/verifier/source sensitivity assertions.
Those assertions are written evidence, not a reported PASS. R01 specifically conflicts
with their positive bundle expectations; R05 identifies missing boundary coverage.

## Actual checks, follow-up and handoff limits

Actual checks: bounded local source/document reads covering the full implementation
and tests; exact before/later hashes and line counts; read-only lib.rs/baseline changed
path comparison; public API/struct/test inventory and golden-literal comparisons;
local chrono 0.4.45 source read matching Cargo.lock; rustfmt check on only the four
reviewed Rust files and scoped git diff --check. The formatting/whitespace commands
returned exit 0. Final report link/line-bound/fence/whitespace checks passed for all
54 links; all five frozen inputs retained their exact line counts and hashes.
Rustfmt is not compilation or lint/test execution.

The focused Mem0 context helper returned MCP_ERROR/no response; no context result or
save was inferred from that failure. The current request's finish used its real hook
envelope with no_durable_change and returned SKIPPED, saved=false, incomplete=false.
No alternative scope/session or provider was used.

Coordinator follow-up: resolve R01–R06 in a bounded implementation follow-up, preserve
the frozen D4 meanings/primary goldens, then bind the new exact source hashes for
independent checking and coordinator-owned compiler/strict-Clippy/test CI. There is
no new product policy, Raw trust, production inventory or G4-value requirement from
this review. Full R2, later I1b/DB/storage/runtime, actual roles, deployment and Owner
acceptance remain separate. No hosted run status/log was queried by this reviewer.

Changed file: this new report only; final exact lines/hash are in the handoff.
Deviations: none. Missing/contradictory adopted requirements: the concrete code/test
mismatches above; no additional missing specification was identified. Required input
files/hashes not found: none. One initial optional cached-chrono path lookup missed;
discovery resolved the actual locked 0.4.45 path without installation/network access.
Unverified: compiler/type-inference diagnostics, Clippy output, every Rust test,
constructed near-limit runtime behavior, hosted CI, SQL/DB/actual roles/ACL/pools,
Raw/body/storage/registry/provider/production/runtime/Owner behavior and final acceptance.
Other unreported missing evidence: none. Further decomposition of this bounded review:
none; the fixes and validation gap are localized to the implementation/test package.
No source/contract/other-review/roadmap/Cargo/lock change, Git write, root/sudo, Docker,
network/provider/production action, compiler/build/test/SQL parser, package installation,
lifecycle, background process or native/Paseo delegation was performed. Background work
and permission waits: none. This unadopted review is not stored as durable progress.
