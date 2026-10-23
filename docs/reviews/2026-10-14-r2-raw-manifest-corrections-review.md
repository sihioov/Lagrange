# WP-4/I1b-S1-F-R — independent Raw corrections delta review

2026-10-14, Asia/Seoul. Direct bounded review in
`/data/worktrees/3puw275b/excited-dolphin`, R2-04/09.
Only this report was written. No delegation was used. Coordinator owns source
acceptance, integration and fresh exact-candidate hosted verification.

**No concrete new defect was identified in this two-file correction delta.**
All five S1-R01–05 source conditions are corrected and the manifest-only trust/API
boundary is preserved. This report does not establish source acceptance, successful
compilation, strict lint cleanliness, test PASS or runtime verification.

## Fresh input checks and comparison

Read repository/provider AGENTS, checked applicable ancestor/subdirectory instruction
paths, ROADMAP R2-04/09 and the complete Wave10 recovery/Wave11 Raw review brief in
the implementation plan. Read the complete frozen prior review, private Raw input
freeze, implementer final and package recovery receipt. D4 Origin and D5 A6 remain
the exact-line and read-only trusted-UID references. The disjoint I0b-F shell sources
were not inspected.

| Fixed input | Lines | SHA-256 |
| --- | ---: | --- |
| [storage.rs](../../crates/market-data/src/storage.rs) | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` |
| [daily_price_only_storage.rs](../../crates/market-data/tests/daily_price_only_storage.rs) | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` |
| [Prior review](2026-10-14-r2-raw-manifest-evidence-review.md) | 290 | `e0322d1774dbd0124d1619f7f69be983dc43a035df8d71ac8ec324d5e43c089b` |
| [D4](../contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| [D5](2026-10-14-r2-daily-input-design-amendments.md) | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

All supplied pins matched before review and at final. Compared the complete scoped
delta against `f1dfb91cdc0cad7d5ae96a310ff462f40cb738a3`. Baseline storage is 3654 lines,
`a2c786c27d5031fbff0a413c1ccb8aa3e9e34fac287ae9a2dc8be1564bad6444`;
baseline tests are 558 lines,
`03a20807c06553848699169e4a6422ac8604108f0765550b281eb680a9f7e047`.
Entry HEAD was the coordinator integration commit
`de1889be8a0f3efa9bac2c1ab1a272c9c1609af0`. The review is bound to source hashes;
a source-preserving HEAD/progress advance does not supply execution evidence.

Private evidence read under target/r2-daily-beta/wp-4-integration:

| Artifact | Lines | SHA-256 |
| --- | ---: | --- |
| source-wave-11-raw-inputs.json | 66 | `1516b9dc497aa4c0888428c57dd20ab09ad4cbd788104ade21c3f4873bb6fb62` |
| wave10-I1b-S1-F-final.md | 44 | `611759ab6896311f1773708fc364ec2bf39c68397e39b7bbb66f3dba12f2f3ec` |
| wave10-I1b-S1-F-recovery.json | 43 | `c1ab5153185c1f2d3e8732064866f0c925bc00d1059a12a6fc92bf8ca840356c` |

The generic recovery.json lookup was absent; the plan's package-specific recovery
name resolved the requested evidence. Its recorded idle/no active turn/pending0/full
final recovery is coordinator handoff evidence, not an independently queried agent
status or successful compiler/test result. No other session's memory envelope was used.

All nine reference files matched the private freeze's line/hash pairs and their
baseline Git bytes: the job-queue/API common modules, both migration-contract
bootstrap files, wire implementation/test, Python readiness test, D4 and D5.
Besides D4/D5 above, the exact reference checks were:

| Reference | Lines | SHA-256 |
| --- | ---: | --- |
| crates/job-queue/tests/common/mod.rs | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` |
| crates/api-server/tests/common/mod.rs | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| tests/integration/migration-contract/role-bootstrap.sql | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| tests/integration/migration-contract/bootstrap.sql | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |
| crates/market-data/src/daily_price_only/contract.rs | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| crates/market-data/tests/daily_price_only.rs | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` |
| nt/live-node/tests/test_runtime.py | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` |

No reference or private receipt was written. The old 929fcd6 CI is baseline only;
no hosted run was queried or treated as proof for these corrections.

## Five finding dispositions

| Prior finding | Source disposition | Concrete correction and written evidence |
| --- | --- | --- |
| S1-R01 serde error disclosure | Corrected; execution pending | [S2357](../../crates/market-data/src/storage.rs#L2357) discards non-torn serde errors and returns InvalidCommittedManifestRecord with path/physical line only. [T124](../../crates/market-data/tests/daily_price_only_storage.rs#L124) checks exact variant, Display/Debug sentinel absence and Error::source=None; [T229](../../crates/market-data/tests/daily_price_only_storage.rs#L229) covers invalid mode/hash/batch ID. |
| S1-R02 receiver/vector typing | Corrected; compilation pending | [S2283](../../crates/market-data/src/storage.rs#L2283) creates a mutable local borrowed reader; [S2325](../../crates/market-data/src/storage.rs#L2325) gives the vector its concrete evidence-record type. The opened descriptor, readonly flags, API and max+1 buffer are unchanged. |
| S1-R03 fixture shadowing | Corrected; compilation pending | [T448](../../crates/market-data/tests/daily_price_only_storage.rs#L448) uses manifest_fixture/lock_fixture; [T503](../../crates/market-data/tests/daily_price_only_storage.rs#L503) uses uid_fixture/non_regular_manifest_fixture/raw_mode_fixture. Every later fixture() still denotes the helper, with independent fresh fixtures and retained negative assertions. |
| S1-R04 absent-tree setup panic | Corrected; execution pending | [T379](../../crates/market-data/tests/daily_price_only_storage.rs#L379) obtains UID from the existing temp directory, then calls the public sibling directly. No helper reads missing Raw metadata. Raw remains absent; the subsequent missing-lock and unchanged-manifest assertions remain. |
| S1-R05 directory errno mismatch | Corrected; OS execution pending | [S900](../../crates/market-data/src/storage.rs#L900) maps both LOOP and NOTDIR to a static UnsafePath reason only in the sibling directory opener. [T483](../../crates/market-data/tests/daily_price_only_storage.rs#L483) adds an ordinary-file directory-component rejection; all strict directory/leaf redirect assertions remain. |

S1-R01 uses is_eof only in the unchanged guard for a final unterminated torn record.
The remaining serde error is dropped; it is not retained in a field, diagnostic or
error source. InvalidCommittedManifestRecord's fields are still only path and line;
its Display uses a static invalid-metadata description and its unchanged Error::source
arm returns None. Derived Debug has no submitted value to expose. The API's evidence
record still has private fields, immutable getters and no Debug implementation.

The sentinel fixture mutates one actual serialized field per case: FetchMode mode,
FileEntry content_hash, or ManifestEntry batch_id. Adjacent type inspection confirms
the mode is a serde enum, ContentHash requires 64 hex digits after sha256:, and
BatchId uses UUID parsing. All sentinels are invalid for their targeted type; all
three rows are LF-terminated, so none can be silently classified as the permitted
final torn EOF. Each reaches the sibling with exact input-byte and one-record limits.
The generic malformed row now receives the same redacted checks; wrong-scope and
invalid-filename assertions remain. No test reads real Raw/provider data or secrets.

The borrowed reader remains &File; only its local binding becomes mutable to supply
Read's receiver. It neither clones/reopens a pathname nor grants mutable filesystem
access. The vector annotation supplies the exact field-access type. These remove
the cited source conditions; no compiler diagnostic or strict-lint PASS is inferred.

The directory mapper changes only the error category/reason for LOOP/NOTDIR.
Permission denial and other errors retain the failing Io path. Open flags, descriptor
walk, UID/mode/type checks and no-follow/no-retry behavior are unchanged. No root
bypass, permission fallback or repair was added, and the legacy mapper was not changed.

## Preservation and guard-to-written-test map

The current parser still copies/hashes the exact record slice excluding only LF.
CR, spaces and key order remain in both bytes and hash. Complete unterminated JSON
is included; only final unterminated serde EOF is ignored. Blank whitespace is not
a record. Complete nonblank terminated rows count before parse/dedup; complete final
unterminated rows count before dedup. Equal parsed duplicates keep first physical
bytes/line and append order, while changed metadata rejects. The error-redaction
change does not modify those branches or their checked counters.

Positive explicit limits, checked max+1, fallible bounded allocation, slice-capped
reads and checked byte/line/record/return-vector arithmetic remain. The reader consumes
at most the max+1 sentinel buffer, including blank/torn-tail bytes. Internal serde/map
allocations remain subject to input size, not a new globally fallible-allocation or
peak-memory promise. No deployment scan policy or Default was introduced.

| Guard / behavior | Production path | Written test/check and execution limit |
| --- | --- | --- |
| Redacted malformed typed values | S2354–2362; static Display and Error::source | T199/T229 via helpers T115/T124 check typed rejection, Display/Debug and no source. All unexecuted. |
| Exact bytes/hash, CR/whitespace/key order | S2332–2337/S2419 | T153 checks original bytes/hash/physical line2 and unchanged manifest. |
| Complete unterminated and torn EOF | S2354–2356 | T176 checks successful unterminated row and ignored final torn tail. No added terminated-torn/non-EOF negative pair. |
| Equal duplicate/conflict and first order | S2391–2423 | T262/T295 retain first bytes/line, reject changed metadata, check order of two unique batches. |
| Byte bound exact/+1 | S2257/S2277–2313 | T320 accepts N bytes with limit N and rejects with N-1; actual syscall byte count is not instrumented. |
| Pre-dedup record limit | S2345/S2365 | T320 accepts two equal rows at limit2 and rejects limit1. No separate over-limit unterminated-final-row test. |
| Zero/checked +1 overflow | S2257–2274 | T350 exact typed failures before Raw creation. Allocation and other counter-overflow branches remain source-only. |
| Missing tree/lock and no repair | S1490→descriptor opens | T376 now reaches the public reader; checks Raw/lock absence and unchanged manifest. |
| Absolute path/no ParentDir and no-follow | S910/S918/S941; readonly no-follow leaf open | T601 rejects relative/parent paths; T403/T470 reject root/ancestor/control redirects; T483 adds ordinary non-directory component. |
| Trusted UID, no group/other write, regular controls | S521→fstat/statat/type/dev/inode/UID/mode | T503 retains wrong expected UID, directory/manifest/Raw mode and nonregular controls. This is not actual cross-UID/ACL proof. |
| Outside ancestors not Raw owner policy | Directory walk's appended-raw threshold | T589 permits writable data-root ancestor; no cross-owner ancestor execution evidence. |
| Existing readonly shared lock/path binding | S1527–1610 | S3591 hook replaces lock before locking; asserts UnsafePath, replacement and simulated writer handle. No new post-lock/read race or writer-wait proof. |
| Manifest inode/path binding before/after read | S1527→opened manifest/revalidation/bounded descriptor read | S3622 hook replaces manifest before read; asserts UnsafePath, hook flag and replacement bytes. No post-read hook test. |
| Successful no-write | Same readonly sibling path | T575 compares manifest bytes and lock length. Not a complete mode/inode/ACL snapshot. |
| Non-Unix closed | Public API's cfg(not(unix)) UnsafePath | T630 alternate conditional test; no non-Unix compilation/execution. |

S denotes storage.rs; T denotes daily_price_only_storage.rs, linked above.
There are 16 Unix integration definitions plus one alternative non-Unix definition,
and two private Unix replacement tests: 19 written definitions, normally 18 selected
on Unix. Two integration definitions were added; no test was executed in this review.
Missing runtime coverage noted here is preserved evidence limitation, not a new
concrete delta defect or an invented requirement for this bounded correction.

The public method, private orchestration, node validator, leaf opener and directory
revalidation bodies are byte-identical to the pinned base. Absolute walking from /,
NOFOLLOW/DIRECTORY/CLOEXEC, Raw-only explicit UID/no-group-other-write enforcement,
regular leaf checks and saved descriptor/path inode binding remain intact. Lock
validation still brackets shared locking and descriptor read; manifest and directory
are revalidated before records can escape. No euid=owner restriction, root bypass,
create/chmod/chown/ACL mutation, reconciliation/store call or unsafe fallback is added.

Legacy committed reader/opener/parser, writer commit helper and orphan discovery/
resync bodies are unchanged. All private storage tests, including both replacement
tests, are byte-identical. Legacy CorruptManifest construction, Display and source
behavior remain intact; only the sibling stopped returning it for malformed records.
Eleven unchanged integration-test bodies were compared directly; the remaining old
tests retain their intended assertions through the narrowly reviewed fixture/helper
changes. Public limits/evidence/getter and Error::source blocks are unchanged.

## Actual checks and remaining work

Actual checks: complete scoped source delta and relevant current parser/descriptor/
test-body reads; fresh source/prior-review/contract pins; nine reference hash/count
and baseline-byte comparisons; function/API/private-test preservation extraction;
adjacent ContentHash/BatchId/FetchMode type checks; test inventory; scoped
`rustfmt --edition 2024 --check --config skip_children=true` on both Rust files
(exit0); `git diff --check f1dfb91...` on the two paths (exit0); report local-link/
line-bound/whitespace checks and final frozen-input verification. Mechanical checks
do not establish compilation, warnings or Rust test behavior.

Own-request memory context returned MCP_ERROR/no response; no context result was
inferred. Own finish returned SKIPPED, saved=false, incomplete=false with
no_durable_change. Only the supplied hook used loopback/operational receipts;
no alternate scope, another session envelope or Basic Memory was used.

Actionable new findings: none. Unresolved source findings within this reviewed delta:
none identified. Follow-up: coordinator combines the report with fresh exact-candidate
compiler, strict all-targets lint and named regression results, then decides acceptance.
No hosted CI status or acceptance was inferred from the prior 929fcd6 baseline.

Deviations from assignment: none. Required inputs not found: none after resolving
the package recovery filename. Missing/conflicting requirements requiring invented
policy or out-of-scope changes: none identified. Other unreported missing evidence:
none. Only the assigned report was written; sources, private receipts, prior reports,
roadmap/plan/ledger/diagrams and Git were unchanged by this reviewer.

Not verified: all current compilation/Clippy/test results; actual platform errno,
lock/race/permission/no-write behavior, cross-UID/ACL access, allocation failure,
peak memory and non-Unix behavior; fresh hosted CI and coordinator acceptance.
Full snapshot/batch/provider truth, copy/normalization/admission, migration0060,
production/runtime and Owner acceptance remain excluded/unverified.
No Cargo/rustc/Clippy/build/tests/self-tests/DB/root/Docker, production/environment/
secret read, provider/network/workflow/lifecycle action, dependency install,
Git stage/commit/push or delegation was performed. Background work and pending
permissions: none. Further decomposition needed: none. Report line range/hash
are supplied in the final handoff.
