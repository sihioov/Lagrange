# WP-4/I1b-S1-R — independent Raw manifest evidence source review

2026-10-14, Asia/Seoul. Direct review for R2-04/09 in
`/data/worktrees/3puw275b/excited-dolphin`. Only this report was written.
The supplied `$paseo-delegate` worker contract was applied with the explicit
direct-work/no-delegation restriction. Coordinator owns corrections, integration,
roadmap updates and fresh hosted verification; this report does not accept source.

**Five findings require bounded follow-up: three P1/HIGH and two P2/MEDIUM.**
The new malformed-record error can expose submitted metadata; the Unix implementation
has mutable-receiver/type-inference problems; new tests contain fixture-function
shadowing, a missing-tree setup panic and a directory-error classification mismatch.
These are source findings, not observed compiler, test or OS-probe diagnostics.
No source fix or test execution was performed.

## Fresh inputs, references and comparison

Read applicable repository and Codex provider AGENTS, ROADMAP R2-04/09, the final
wave-8/wave-9 plan sections and complete
[I1b-S1 brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1072).
Read [D4 Origin](../contracts/owner-beta-daily-etf11-input-v1.md#L156),
[D5 A6](2026-10-14-r2-daily-input-design-amendments.md#L509), the
[source handoff](2026-10-14-r2-role-and-raw-source-handoff.md), the private implementer
final and wave-8 input inventory. No prior source version was used as current input.

| Reviewed fixed input | Lines | SHA-256 |
| --- | ---: | --- |
| [storage.rs](../../crates/market-data/src/storage.rs) | 3654 | `a2c786c27d5031fbff0a413c1ccb8aa3e9e34fac287ae9a2dc8be1564bad6444` |
| [daily_price_only_storage.rs](../../crates/market-data/tests/daily_price_only_storage.rs) | 558 | `03a20807c06553848699169e4a6422ac8604108f0765550b281eb680a9f7e047` |
| [D4](../contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| [D5](2026-10-14-r2-daily-input-design-amendments.md) | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

Both owned sources matched the private inventory before review and at final.
Comparison baseline is `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`; its storage blob
is 2914 lines, SHA-256
`1ff177e3b41e4a1bcfc7a4caf1dd42d5230357ac605e21158d4a2b93ea91b70f`.
The new integration-test file is absent from that baseline. Reviewed the complete
storage delta and complete new test file, plus exact adjacent legacy/helper paths.

The private inventory is 94 lines, SHA-256
`e27c834dc9014804631029a9f77276f4133f9d0a96b8a71a03d4e6c65784152c`;
the implementer's final is 55 lines,
`5e97157b63407f68a6c8d2de7ddf481b2bfad0def2be502d7b2abf939924cc9e`.
The source handoff is 91 lines,
`701705ff7645cbcfc4f1d610f4ebd672a7f972e7f58101030bb1b5693586349f`.
These are input provenance, not executed evidence. Entry HEAD was
`da0499ee1b89f9696df45bf5a75a2c82be581996`; a later read was
`f1dfb91cdc0cad7d5ae96a310ff462f40cb738a3` while the pinned inputs stayed fixed.

All nine declared unchanged references matched inventory counts/hashes and were
byte-identical to their baseline Git blobs:

| Reference | Lines | SHA-256 |
| --- | ---: | --- |
| crates/job-queue/tests/common/mod.rs | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` |
| crates/api-server/tests/common/mod.rs | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| tests/integration/migration-contract/role-bootstrap.sql | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| tests/integration/migration-contract/bootstrap.sql | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |
| crates/market-data/src/daily_price_only/contract.rs | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| crates/market-data/tests/daily_price_only.rs | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` |
| nt/live-node/tests/test_runtime.py | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` |
| D4 (above) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| D5 (above) | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

The accepted baseline's hosted jobs/30 wire-test results cannot validate these new
source bytes. No hosted run was inspected. The concurrent nine I0b source files were
not reviewed; their inventory entries were not treated as this package's findings.

## S1-R01 — malformed-record errors retain submitted values

**P1/HIGH; violates the sibling's explicit error-redaction requirement.**

[Parser:2355](../../crates/market-data/src/storage.rs#L2355) returns the existing
CorruptManifest variant with the original serde_json::Error. StoreError derives Debug;
its [Display:242](../../crates/market-data/src/storage.rs#L242) includes `{source}`,
and Error::source also exposes that error. Serde errors can carry submitted strings,
not only a fixed syntax description.

Concrete trigger: a complete manifest row otherwise matching the fixture, with mode
set to an unrecognized sentinel string. [FetchMode](../../crates/market-data/src/contract.rs#L258)
is a serde-derived enum. The locked serde_core 1.0.229 unknown_variant implementation
formats the submitted variant into its error message; serde_json 1.0.151 preserves
that message in both Display and Debug. The new sibling therefore returns an error
whose formatted output contains the sentinel manifest value. Invalid typed hash/ID
strings can similarly enter custom deserializer messages. No secret or provider body
was read to establish this source path.

The test named without_echoing_metadata only checks that the rejected filename is
absent from InvalidCommittedManifestRecord Display. Its malformed case is the generic
`not-json`, which does not exercise a value-bearing serde diagnostic. It does not
cover Display, Debug and source-chain leakage on malformed typed records.

Bounded correction: after consulting is_eof for the torn final tail, discard the
serde error in this sibling and return a new or suitable redacted typed record error
with safe path/physical line/category only. Add sentinel-based assertions for Display,
Debug and any exposed source chain. Preserve the legacy reader/error behavior outside
the sibling and preserve the exact EOF-tail decision.

## S1-R02 — Unix reader/parser need local type-check corrections

**P1/HIGH; source compile blockers, no compiler invocation.**

[read_committed_manifest_bounded:2276](../../crates/market-data/src/storage.rs#L2276)
binds `file: &File` immutably, then [2291](../../crates/market-data/src/storage.rs#L2291)
calls `file.read(...)`. std::io::Read::read requires a mutable receiver; Read for &File
requires borrowing that reference binding mutably. The parameter is not mut, and
a shared &File cannot instead supply &mut File. This function is compiled on Unix
regardless of whether the new API is called by a test or product.

The parser also starts with an unconstrained Vec at
[2322](../../crates/market-data/src/storage.rs#L2322), then accesses
[records[first_index].entry:2390](../../crates/market-data/src/storage.rs#L2390)
before its first type-constraining push. Field access on the unknown element type
needs an explicit Vec<CommittedManifestEvidenceRecord> annotation; the later push
and final return do not make this early field lookup a reliable inference site.
Actual compiler diagnostics and diagnostic codes remain unobserved here.

Bounded correction: make the borrowed reference binding mutable, or create a local
mutable borrowed reader, and annotate the records vector's element type. Keep the
same opened descriptor, read-only access and max+1 buffer. Neither correction needs
a public API, storage permission or contract change. Fresh hosted compilation must
confirm these changes and detect any additional diagnostics.

## S1-R03 — new fixture bindings shadow the fixture function

**P1/HIGH; deterministic test-target compile failure.**

In rejects_symlinked_raw_root_and_manifest_redirect, the local `fixture` bound at
[382](../../crates/market-data/tests/daily_price_only_storage.rs#L382) remains in scope
when [392](../../crates/market-data/tests/daily_price_only_storage.rs#L392) calls
`fixture()` again. Name lookup finds a Fixture value, not the function, so that call
cannot type-check. The same pattern occurs in rejects_non_regular_controls...:
[418](../../crates/market-data/tests/daily_price_only_storage.rs#L418) shadows the
helper before calls at [465](../../crates/market-data/tests/daily_price_only_storage.rs#L465)
and [474](../../crates/market-data/tests/daily_price_only_storage.rs#L474).

The Unix integration target therefore cannot reach its assertions as written.
Bounded correction: use distinct local names or disjoint nested scopes for each
fixture and update only those references. Retain the fresh independent fixtures and
all negative assertions. No helper/public rename or weakened path check is necessary.

## S1-R04 — missing-tree regression panics before calling the sibling

**P2/MEDIUM; deterministic written-test failure and missing intended evidence.**

[Test:323](../../crates/market-data/tests/daily_price_only_storage.rs#L323) creates an
empty temp directory and calls the shared read helper. That helper first evaluates
trusted_uid(store), whose [56](../../crates/market-data/tests/daily_price_only_storage.rs#L56)
unwraps metadata for `store.root()/raw`. That directory deliberately does not exist.
The helper panics before read_committed_manifest_evidence is invoked; is_err and the
no-creation assertions are never reached, nor is the later missing-lock case.

Bounded correction: call the public API directly with an explicit expected UID for
the absent-tree case, or pass an explicit UID through a helper that does not inspect
the absent object. Do not create Raw merely to obtain its UID. Keep both missing-tree
and missing-lock/no-write assertions. Product source itself opens without creation;
this finding concerns the test's inability to verify that path.

## S1-R05 — directory-symlink assertions assume only the LOOP errno

**P2/MEDIUM; concrete error-classification/test mismatch, no no-follow bypass.**

The directory walk at [950](../../crates/market-data/src/storage.rs#L950) uses
RDONLY|DIRECTORY|NOFOLLOW. Its mapper at
[900](../../crates/market-data/src/storage.rs#L900) turns only Errno::LOOP into
UnsafePath; a NOTDIR result becomes StoreError::Io. No-follow directory opens can
report NOTDIR for a symlink, including the Linux O_DIRECTORY|O_NOFOLLOW case.
Locked rustix 1.1.4 forwards the flags/errno rather than converting NOTDIR to LOOP.

The alias-root, alias-ancestor and redirected control-directory fixtures at
[359](../../crates/market-data/tests/daily_price_only_storage.rs#L359),
[372](../../crates/market-data/tests/daily_price_only_storage.rs#L372) and
[410](../../crates/market-data/tests/daily_price_only_storage.rs#L410) each require
UnsafePath exclusively. On the NOTDIR outcome they fail despite the reader rejecting
the redirect. This platform-dependent errno condition was not probed or executed
here. Leaf symlink opens use different flags and are not this condition.

Bounded correction: map directory NOTDIR to an appropriate static unsafe-directory
reason alongside LOOP, or make the tests explicitly accept the documented typed
no-follow NOTDIR rejection. Keep permission-denied errors closed, do not retry with
different/following flags, and do not change the legacy opener unnecessarily.

## Reviewed behavior and guard-to-written-test mapping

The API supplies explicit positive byte/record limits, no Default or deployment scan
policy. Evidence fields are private; four immutable getters expose parsed entry,
original bytes/hash and 1-based line number. The record has no Debug implementation
and grants no approval. R01 concerns the separate malformed-record error route.

The intended parser slices up to but excluding LF, retaining CR/spaces/key order;
it hashes/copies that same slice. Complete unterminated JSON is included. Only a
nonblank final unterminated record whose serde error is_eof is ignored; terminated
EOF errors and other malformed rows fail. Blank ASCII-whitespace records are ignored.
Equal parsed duplicates retain the first bytes/line and order; conflicting metadata
fails. Complete nonblank terminated rows are counted before parsing/dedup; a successful
unterminated final record is counted before dedup. Torn EOF does not consume a record.

Limits use checked +1, a fallible max+1 allocation, bounded read slices and checked
read/record/physical-line counters. The input includes all bytes, even blank/torn tail
bytes. Returned line buffers use fallible reservation; the return-vector size estimate
uses checked multiplication. `start+newline_offset` is bounded by the remaining slice,
and increment past LF is checked. Serde and BTreeMap still allocate internally; the
caller byte bound limits their input, and this code does not claim globally fallible
allocation or a measured peak-memory bound. Maximum+1 is allocated up front even for
a short file; no unlimited read_to_end is added to this sibling.

| Guard / behavior | Production anchor | Written check and limitation |
| --- | --- | --- |
| Exact LF-only bytes/hash, CR and whitespace/key order | S2329–2334/S2417 | T116 checks bytes, hash, line2 and unchanged file; T139 checks complete unterminated bytes and ignored torn EOF. |
| Malformed/scope/name rejection | S2351/S2372/S2378 | T162 checks typed rejection; value-bearing error redaction is missing, R01. No terminated-torn versus non-EOF unterminated negative pair is written. |
| First equal duplicate/conflict and append order | S2389–2421 | T206/T239 check retained first physical bytes/line, conflicting metadata and order of two unique batches. |
| Byte bound exact/+1 | S2255/S2281–2310 | T264 accepts N bytes with limit N and rejects the same file at N-1; no counting-reader hook independently measures syscall bytes. Source slices cap total read at limit+1. |
| Complete nonblank count before dedup | S2342/S2363 | T264 accepts two duplicate rows at limit2 and rejects them at limit1. No separate over-limit complete unterminated final-row case is written. |
| Zero and usize +1 overflow | S2258–2271 | T294 uses an absent tree, checks exact error variants and no Raw creation. Allocation-failure/other counter-overflow branches have no injected regression. |
| Absolute/ParentDir-free path | S908/S916 | T515 requires typed rejection of relative and parent-component roots. |
| No-follow every directory from /; no-follow leaves | S939/S950/S1008 | T337/T404 redirects; directory errno mismatch R05. Missing-tree setup is broken, R04. |
| Raw/control UID and no group/other write; regular leaves | S540–558 | T417 checks wrong expected UID, market-dir and manifest group-write, Raw other-write and nonregular controls; repeated fixtures cannot compile, R03. Wrong expected UID is not actual cross-UID/ACL proof. |
| Outside Raw ancestors have no Raw UID/mode rule | S931/S965 | T503 makes the data-root ancestor group/other writable and expects success. No cross-owner ancestor fixture is written. |
| Existing lock, shared lock, pre/post revalidation | S1539–1595 | S3589 replaces lock before locking; asserts UnsafePath, replacement and writer handle. No sibling-specific shared-lock coexistence/writer-wait or post-lock/read replacement test is written. |
| Manifest opened descriptor and inode/path binding | S1008/S1575/S1587 | S3620 replaces opened manifest before reading; asserts UnsafePath, hook flag and replaced bytes. No post-read hook test is written. |
| No create/repair/write; missing lock and positive read | S1534–1608 | T320 intended missing tree/lock checks blocked by R04; T489 compares manifest bytes and lock length. This is not a full mode/inode/ACL snapshot assertion. |
| Non-Unix closed | S1510–1520 | T544 conditional UnsafePath assertion; no non-Unix compilation/execution evidence. |

Here S means storage.rs and T means daily_price_only_storage.rs, both linked above.
There are 14 Unix integration-test definitions, one alternative non-Unix definition
and two new private Unix tests: 17 written definitions, normally 16 selected on Unix.
None was executed. Generic is_err assertions are not counted as proof of a specific
guard when the fixture fails earlier. No existing 30 wire test was altered.

The path walk opens from /, rejects ParentDir, and applies owner/mode enforcement
starting at the appended raw component, not the data root or outside ancestors.
statat with SYMLINK_NOFOLLOW and fstat compare file types and device/inode binding.
Raw directories and both regular controls use the explicit trusted UID. There is no
new euid equality, root bypass or fallback. Directory revalidation reopens the lexical
walk and compares the final descriptor inode to the saved one.

The sibling validates lock before/after shared locking and after reading. It validates
manifest at open, after the test hook and after descriptor read, then validates the
directory and lock before parsing/exposure. Unlock is attempted on closure failure,
and success additionally requires unlock success. Cached fs2 0.4.3 shows shared flock
and unlock use the opened file descriptor; no write/create is hidden in those calls.
No path-based read replaces the opened manifest descriptor. No chmod/chown/mkdir,
ACL mutation, writer/orphan reconciliation or store/commit call is reachable from
the new method. Runtime race/lock guarantees still need execution evidence.

## Legacy preservation, checks and handoff limits

Source comparison found the delta additive except the necessary new StoreError
Display/source match arms. Function-body comparisons confirm unchanged store_batch
and its commit helper, append_manifest and its locked helpers, validate_copied_sources,
read_manifest, legacy committed-reader/open-control functions, reconciled-reader
functions, parse_manifest_records/merge, discover_orphan_batches/resync_orphan_batch
and filename validation. All existing private storage unit tests are preserved;
only the two new private tests are appended. Accepted legacy behavior was not weakened
to implement the trusted-UID sibling.

Actual checks: fresh source/document reads and full delta/new-test inspection;
own inputs and nine reference hashes/counts with baseline-byte comparisons; legacy
function/test preservation extraction; cached locked rustix/fs2/serde dependency reads;
edition-2024 rustfmt --check with skip_children=true on the two sources (exit0);
scoped Git diff whitespace check (exit0); report local-link/line-bound/whitespace
checks and final input stability verification. No compiler, Clippy, tests, builds,
shell validator/self-test, SQL/DB, root, Docker, network/provider/production operation,
secret/provisioning/lifecycle/dependency install, Git write or delegation was performed.
The strict all-targets lane will compile these paths; rustfmt does not establish
type correctness or warning cleanliness. No additional speculative lint finding is
asserted in place of actual diagnostics.

The own hook context call returned MCP_ERROR/no response. No memory result was used.
Own finish returned SKIPPED, saved=false, incomplete=false with no_durable_change.
Only the supplied helper performed loopback/operational-receipt work. No other memory
scope/session or Basic Memory was used. One optional serde_core version lookup missed;
Cargo.lock and the actual cached 1.0.229 source resolved it without installation.

Coordinator follow-up: correct S1-R01–05 within the two-file package, preserve the
frozen D4/D5 and legacy behavior, pin corrected hashes and obtain independent delta
review plus fresh exact-candidate compilation/strict lint/test evidence. Full
batch.json/provider bytes, copy lineage, normalization, admission and runtime/Owner
acceptance remain deliberately later; their absence is not a defect of this seam.

Deviations from assignment: none. Missing required inputs: none. Contradictory or
missing requirements needing invented policy or out-of-scope changes: none identified.
Unresolved: the five findings and the execution gaps mapped above. Unverified:
all new compiler/lint/test results, observed platform errno, allocation failure and
post-read races, actual cross-UID/ACL permissions, lock behavior and no-write runtime
proof, non-Unix behavior, fresh hosted CI and coordinator acceptance. Excluded DB/
provider/batch/admission/deployment/Owner behavior is unverified and not claimed.
Other unreported missing evidence: none. Further decomposition needed: none; the
bounded corrections remain local. Background work and pending permissions: none.
Report line range and SHA-256 are supplied in the final handoff.
