# WP-4/I1a-F2-R — independent integrated delta review

2026-10-14, Asia/Seoul. Direct review in
`/data/worktrees/3puw275b/excited-dolphin`; ROADMAP R2-04/09.
Only this report was written. Coordinator owns integration, roadmap updates and
fresh exact-candidate hosted CI. No delegation or further decomposition was needed.

**No concrete new defect was identified in the combined reviewed delta.**
CR-01–04 and the additional diagnosed lexical/pair condition and fixture-shadow
conditions are corrected at source level. Original R01/02/03/06 corrections and
the frozen public contract are preserved. This is source closure only: compiler,
strict all-targets Clippy and all 30 written Rust tests remain unverified.

## Fixed inputs and scope

Read AGENTS.md, ROADMAP R2-04/09 and the complete latest I1a-F2-R brief at the end
of the implementation plan. Used the frozen D4 and prior corrections review,
and read the complete new corrections CI report. Reviewed the combined two-file
delta against `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff`, including relevant
unchanged guard, validation and fixture paths. Concurrent Python work was excluded.

| Exact input | Lines | SHA-256 |
| --- | ---: | --- |
| [contract.rs](../../crates/market-data/src/daily_price_only/contract.rs) | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| [daily_price_only.rs](../../crates/market-data/tests/daily_price_only.rs) | 2386 | `11a8255d60d5c4ff1e16f136e3246adf046fc7743c8248b7eb7de755e1d3cd74` |
| [D4](../contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| [Frozen corrections review](2026-10-14-r2-daily-wire-corrections-review.md) | 309 | `72784303a8c92984b9d6e070838b4aaa5094318ee1b7594f8a90568796946546` |
| [Corrections CI report](2026-10-14-r2-daily-wire-corrections-ci.md) | 31 | `6fe75a87cb2b67d19544b933bc2e3b06d1cc1d7706e2b3325bb87730034357bd` |

All supplied counts/hashes matched at entry and final checks. Baseline source blobs
are the former 4163-line implementation with SHA-256
`79370bc25c927aa0d4b19e9646ab1bccfb845b8980a800217282a172901ccf2a`
and 2283-line tests with SHA-256
`ea08ac3ae55c7a4b667556cec100cd25ac32298d2d21a7c209b490c412f355e4`.
Entry HEAD was `883ed6f6181f1d91c14427752b92d459b64ab22c`; HEAD/progress documents
may advance while these source hashes remain frozen. This review is bound to the
hashes, not an assumption that HEAD or unrelated files stayed unchanged.

The read CI report attributes run38506113191's terminal failure at 03:36:21 UTC
to three collapsible-if lints, two table type-complexity lints, workspace E0618 and
a separately owned Python failure. Its chart six tests and PostgreSQL APPROVED/
59-migration checks passed for the old candidate. None of its 29 named daily-wire
tests produced execution results. Those old successes do not validate this delta,
and this review did not query the run or execute its commands.

## CR-01–04 disposition

| Finding | Source disposition | Condition/path checked |
| --- | --- | --- |
| CR-01 civil-date bound | Corrected; execution pending | [date_range:2514](../../crates/market-data/src/daily_price_only/contract.rs#L2514) checks `dates.len().checked_add(1)` before pushing. Proposed length 370 passes; 371 returns LimitExceeded before the terminal break. The unchanged private 370/371 assertions are at [4104](../../crates/market-data/src/daily_price_only/contract.rs#L4104). Public validate_session retains its returned-length check. |
| CR-02 shared 16-hop metadata | Corrected; execution pending | [1919](../../crates/market-data/tests/daily_price_only.rs#L1919) updates every shared calendar binding for both 16 and 17 hops. Same input metadata, first_batch and timestamps produce identical full FileEvidence for the three reused identities. The 16-hop call reaches public evidence validation without the former conflict; the 17-hop call is rejected in preflight. |
| CR-03 private lint patterns | Corrected at source; actual lint result pending | [Aliases:4002](../../crates/market-data/src/daily_price_only/contract.rs#L4002) replace the directly nested array/tuple/function-pointer type annotations. Singleton borrowed calls use from_ref, including the old copied-calendar/cash cases. No blanket warning allowance or strict-lane change was introduced. |
| CR-04 output-byte coverage | Corrected; execution pending | [Writer matrix:4157](../../crates/market-data/src/daily_price_only/contract.rs#L4157) binds all four names to their actual fixed values and tests exact/+1 canonical bytes. [Public proof pair:1123](../../crates/market-data/tests/daily_price_only.rs#L1123) reaches the proof encoder's final output writer with fresh commitments, legal shape and counts. Legality, formulas and cost are checked below. |

CR-02's top-level FileKeys do not change when copied_from/original timestamps change,
so the calendar normalized-source commitment need not be refreshed for this fixture.
All current/origin batch identities within each chain are distinct; request/wire/size
metadata remain equal and time decreases by single nanoseconds. The original current
batch ends in 1; origins begin at 3. Updating all bindings preserves the permitted
shared-file identity equality rule rather than weakening it. Ten-page/+1 evidence
and the two-hop nanosecond inversion test remain in place.

CR-04 distinguishes isolated writer evidence from public schema evidence. Legal
policy output remains the fixed 1827-byte shape; admission has bounded scalar fields
and fixed-length strings and cannot approach 1 MiB. The new comments identify those
unreachable nominal public boundaries. The isolated matrix covers policy 262144,
manifest 16777216, admission 1048576 and normalization 16777216 independently by
name/value. For each L, a plain ASCII string of length L-2 encodes to exactly L bytes;
length L-1 encodes to L+1 and must return LimitExceeded. No schema-invalid object is
counted as an output-byte test. Public policy/admission success and the earlier
four incoming-byte boundary pairs are retained.

## Additional diagnosed conditions

The two lexical changes at
[1334](../../crates/market-data/src/daily_price_only/contract.rs#L1334) and
[1339](../../crates/market-data/src/daily_price_only/contract.rs#L1339) preserve
short-circuit behavior. UUID parsing executes only for batch_id/source_batch_id
and only after valid_uuid succeeds; commit parsing executes only for
code_commit/verifier_code_commit and only after valid_commit succeeds. A matching
invalid value still returns its exact InvalidUuid/InvalidCommit; an unrelated key
does not run either parser. The order relative to other lexical checks is unchanged.

[pair_value:2007](../../crates/market-data/src/daily_price_only/contract.rs#L2007)
uses `name == key && found.replace(value.as_str()).is_some()`. Rust's left-to-right
short circuit means a nonmatching pair cannot replace found. The first matching
pair stores its value and continues; a second matching pair returns DuplicateEntry.
No match ends as InconsistentFields. This preserves both the lookup and its duplicate
error; moving replace outside the conditional would not have preserved that behavior.

The final test at
[2332](../../crates/market-data/tests/daily_price_only.rs#L2332) renames only its local
fixture value to candidate and updates its references. The final
[2380](../../crates/market-data/tests/daily_price_only.rs#L2380) `fixture(false)` now
resolves to the helper function, producing a fresh first-addition fixture. The
verifier-only hash checks, stale policy-pin/content-hash rejection and nonnull
first-addition-parent assertion retain their original meaning. No global helper/API
rename or weakened assertion was introduced. The E0618 source condition is removed;
that statement does not claim observed compiler success.

## Public proof fixture: legality, exact sizes and guard path

The new test creates one standalone unapproved bars document, dated 2026-08-31,
using the fixed schema/version, valid hashes and a nonempty source-key set.
Each key has provider kis, market kr, a nonnil canonical UUID and a unique 128-byte
ASCII filename. The six-digit index is below 1000000 and gives distinct identities
in lexicographic order. Reusing the same claimed wire hash across different filenames
is legal for FileKeys; no page-array uniqueness rule is inherited here.

This is a standalone proof encoder fixture. Its claimed size_bytes=1 and row_count=0
are legal local document claims: proof validation does not bind them to a particular
session's 11 bars or to actual normalized bytes. It is not asserted to form a valid
session/bundle, supply ETF bars or establish external document truth. Those separate
constraints are not invented to make a byte test appear successful.

The test refreshes source_evidence_sha256 through the public
[normalized-source preimage:3796](../../crates/market-data/src/daily_price_only/contract.rs#L3796)
after both key-count changes. That helper sorts/validates the keys and serializes
the inner commitment under the 67108864-byte guard. The encoder independently
reconstructs the same commitment in validate_proof, so its hash is fresh in both cases.

Independent in-memory Python compact-JSON/hash calculation confirmed the source
recipe, with no Rust code execution. Each FileKey is 313 bytes; comma stride is
314; one-key proof minus that key is overhead 533. Thus
`proof_bytes(n) = 533 + 314*n - 1`. The largest equal-length-key count below the
output limit is `floor((16777216 - 533 + 1)/314) = 53428`.
The inner commitment has `inner_bytes(n) = 110 + 314*n - 1`.

| Keys | Inner commitment bytes | Proof bytes | Relative to output limit |
| ---: | ---: | ---: | --- |
| 53428 | 16776501 | 16776924 | 292 bytes below |
| 53429 | 16776815 | 16777238 | 22 bytes above |

The calculation also verified unique identities, 128-byte names and matching fresh
commitment hashes. Sorting cannot alter these lengths. Both counts are below 262144;
one document is below the proof-document bound; size_bytes=1 is below 67108864;
valid date/schema/kind/version and shallow typed structure introduce no competing
shape/date/count/depth failure. No arbitrary large index or unbounded loop is used.

The exact rejection path is
[encode_daily_normalization_proof_v1:3865](../../crates/market-data/src/daily_price_only/contract.rs#L3865)
→ preflight_proof → canonicalize/validate_proof →
`canonical_bytes_limited(value, max_normalization_bytes)` at 3871 → bounded writer.
Preflight counts pass; the inner serialization and hash check pass structurally;
only the outer 16777216-byte writer exceeds its limit. BoundedVecWriter checks the
proposed byte length before extending and sets exceeded, which maps the serializer
error to LimitExceeded. Stale-hash, duplicate, shape and count failures do not masquerade
as the intended byte overflow.

Resource cost is bounded but nontrivial: approximately 16 MiB serialized proof,
53429 FileKeys and one document. Refreshing and encoding clone key sets/commitments,
sort and compare keys, compute hashes and allocate byte buffers. Positive encoded
bytes and oversized unrestricted size-evidence bytes are explicitly dropped before
the next stage. Peak RSS, duration and actual allocation behavior were not measured;
serialized size is not peak memory. No requirement to construct 262144 full files,
provider bodies or a valid large session/bundle is introduced. No concrete resource
illegality was found in this fixture; hosted execution must establish its practical cost.

## Preserved earlier corrections and frozen contract

Static function-body comparison against the baseline confirms derive_evidence_counts,
derive_counts, validate_file_evidence, validate_proof, seal and the bundle validator
are unchanged. In particular:

- R01 totals use the actual manifest.sessions length and checked multiplication by
  11; added/evidence counts stay separate. The more-than-two-base-dates test remains.
- R02 retains inner timestamp comparisons, all three concrete Option accumulator
  types and nanosecond serialization/order; the corrected shared-hop fixture adds
  coverage without changing production copy rules.
- R03 retains valid_domain_date before proof commitment checks and the fresh-hash
  years 0/1/9999/10000 typed/JSON parity test.
- R06 retains the final manifest byte guard before seal returns. The near-limit
  seal pair and its construction are unchanged; no new seal-size calculation or
  Rust execution is claimed in this turn.

All 25 wire struct macro declarations/field order, four literal enum declarations,
payload-free error enum, 17 public free-function signatures and two timestamp methods
are byte-identical to the baseline. No approved/verified type or external authority,
Raw/file/provider/registry/DB/runtime path was added. Historical and registration
code is outside this combined two-file delta.

The primary seven-golden test is unchanged. Its six sizes/hashes and literal `[]`
were independently reproduced from D4's normative pure Python construction:
1827/44176/8790/45616/45707/1353 and 2 bytes, all matching the existing assertions.
The present-empty cash literal remains 515 bytes with SHA-256
`0f87c41ba19ba820ee79a2a05f54c27b9a4af5f185a98b080bc429b56eaca864`.
The empty-cash bundle and exact-error test bodies are unchanged. The 32-byte-code
test changes only its cloned singleton borrow to from_ref; empty/32-byte acceptance
and MissingField/InvalidType/InvalidLiteral classifications remain intact.

Inventory: 25 integration tests plus five private unit tests, 30 written in total.
This is a source inventory, not a result count. The new proof pair is the added test.

## Actual checks, follow-up and limits

Actual checks performed: read-only source/document/Git blob/diff inspection; supplied
line/hash comparisons; API/function/test-body inventory and preservation comparisons;
cash-literal hash; bounded in-memory Python proof/golden calculations; rustfmt
`--edition 2024 --check` on the two fixed Rust files (exit 0); scoped
`git diff --check 02a400e0...` on those files (exit 0); report link/line/fence/whitespace
checks and final frozen-input stability checks. None is compilation or test execution.

The own-request memory context helper returned MCP_ERROR/no response; no context
result was inferred. The supplied finish helper returned SKIPPED, saved=false,
incomplete=false with no_durable_change. Only the explicitly supplied hook performed
loopback/operational-receipt work; no alternative memory scope or session was used.

Actionable new delta findings: none. Unresolved source findings in this bounded delta:
none identified. Follow-up: coordinator combines this source review with fresh
exact-candidate compiler, strict all-targets Clippy and all 30 named test results.
Existing old-candidate chart/PG success and the separate Python correction are not
substitutes. No current-candidate CI PASS or R2-04/09 completion is inferred.

Deviations from the fixed assignment: none. Missing/conflicting requirements: none
identified. Missing fixed inputs or unexplained input-hash mismatches: none.
Other unreported missing evidence: none. Only this report was written; prior reports,
D4, sources, roadmap, Python files and Git state were not written by this reviewer.

Unverified: current compilation/Clippy output; all 30 Rust tests; Rust fixture sizes,
run duration/peak memory and historical v2/v3 execution; fresh hosted CI and final
coordinator acceptance. I0/0060, DB/SQL/actual roles, external bytes/Raw/registry trust,
provider acquisition, deployment/runtime and Owner acceptance are excluded/unverified.
No Cargo/rustc/Clippy/test/build, DB/SQL/root/Docker, external network/provider,
operations/workflow action, Git write or delegation was performed.
Background work and permission waits: none. Further decomposition needed: none.
Exact report line range and SHA-256 are supplied in the final handoff.
