# WP-4/I1b-S2-F-R — Raw snapshot correction source review

Completed independent source review for R2-04/09, 2026-10-14.
Actual startup: 12:38:23.234 UTC; reporting deadline: 13:38:23.234 UTC.
Frozen handoff: `7545094404628974c1bd626c84f6c3f704555f5f`.
Comparison baseline: `411326865706ea9e7a1690aa49742cd0254d8450`.

Corrections are required. There are two P1 static unit-test compile blockers,
five P2 test/lint findings, two P2 production contract findings and one P2
evidence-inventory discrepancy. These are source conclusions, not executed
compiler, Clippy or test diagnostics. The production defects and T01 coverage gap
predate the correction; they violate the fixed reviewed contract.
Source acceptance remains with the coordinator.

All R01–R16, R12a–h, P1–P10, T01–T32 and C01 were inspected directly against
the current bytes. Required review rows left uninspected: none. All 45 definitions
remain WRITTEN_UNEXECUTED: 15 private Linux, 29 integration Linux, one non-Linux.
The original 34-definition inventory is historical and is not execution evidence.

**Prioritized findings**

**P1 S2-FR01 — the bounded-file test reuses a consumed ScanLimits.**

[source.rs:3143](../../crates/market-data/src/daily_price_only/source.rs#L3143)
passes `bounded_limits` by value into `scan_with_limits`; the same binding is
passed again inside the observation closure at
[3154](../../crates/market-data/src/daily_price_only/source.rs#L3154).
The helper takes an owned ScanLimits at
[2851](../../crates/market-data/src/daily_price_only/source.rs#L2851), and the
opaque type at [94](../../crates/market-data/src/daily_price_only/source.rs#L94)
implements neither Copy nor Clone. The first call consumes it regardless of
whether the returned snapshot is discarded. Trigger: compiling the Linux library
unit-test target. Consequence: use-after-move type checking blocks that target and
the required all-target gate. This is newly written code, separate from C01.

Bounded correction: construct a fresh ScanLimits from the Copy ScanLimitValues
for each call. Preserve the valid control, the same cap, and every observation;
do not add Clone/Copy to the public capability. Regression gap: T08's new private
oracle has never compiled or run.

**P1 S2-FR02 — the sequence-parity test requires Debug on an opaque parser error.**

[source.rs:3234](../../crates/market-data/src/daily_price_only/source.rs#L3234)
calls `unwrap()` on `Result<ManifestEntry, BoundedParseError>`.
[BoundedParseError:2202](../../crates/market-data/src/daily_price_only/source.rs#L2202)
has no Debug implementation; its Json variant contains a serde error.
Result::unwrap requires its error type to implement Debug. Trigger: compiling the
Linux library unit tests. Consequence: another independent type-check blocker,
even after S2-FR01 is corrected.

Bounded correction: match the result or map failure to a fixed fieldless error
before asserting success. Do not derive payload-formatting Debug on the parser
wrapper merely to satisfy unwrap. Preserve the legacy sequence comparison.
Regression gap: the newly added sequence-parity definition is uncompiled.

**P2 S2-FR03 — a new double-ended iterator is consumed with last().**

[source.rs:3396](../../crates/market-data/src/daily_price_only/source.rs#L3396)
uses slice `iter().filter_map(...).last()`; the call is at
[3402](../../crates/market-data/src/daily_price_only/source.rs#L3402).
Both this slice iterator and its FilterMap implement DoubleEndedIterator, and
the closure simply selects retained-byte totals. This exposes
`clippy::double_ended_iterator_last` under the required all-targets/all-features
`-D warnings` gate once the unit target type-checks. No lint command was run.

Bounded correction: take the last matching event with `next_back()` or a reverse
find_map, retaining the exact asserted total. Do not allow or suppress the lint.
Regression gap: rustfmt cannot establish strict-warning cleanliness. The original
R05 patterns were corrected, but the overall R05 gate remains partial.

**P2 S2-FR04 — the replacement FD observer still confuses reuse with ownership.**

[descriptor_is_open:3012](../../crates/market-data/src/daily_price_only/source.rs#L3012)
queries `/proc/self/fd/<raw_fd>` and compares only device/inode. The new post-close
assertions at [3364](../../crates/market-data/src/daily_price_only/source.rs#L3364),
[3434](../../crates/market-data/src/daily_price_only/source.rs#L3434),
[3872](../../crates/market-data/src/daily_price_only/source.rs#L3872),
[3908](../../crates/market-data/src/daily_price_only/source.rs#L3908), and
[3944](../../crates/market-data/src/daily_price_only/source.rs#L3944) require every
observed slot to stop naming that inode.

Concrete trigger: after this construction closes its root or another shared
ancestor descriptor, a parallel snapshot test opens the same `/` or shared
ancestor and receives that freed FD number before the assertion. Its device/inode
matches the old observation, despite the original owned handle having closed.
Consequence: correct normal Drop or failure cleanup can fail, including inside the
cleanup-time hook. Thread-local event selection does not isolate the process FD
table or distinguish open-file-description lifetimes. This is a remaining R10
defect, not an observed FD leak. Held-snapshot checks are stronger: those slots
cannot be reused while their owning handles remain alive.

Bounded correction: observe closure tied to each owning handle/lifetime, and use
fixture-unique native objects and the separate lock competitor for post-close
proof. Keep accounting for all opens, including common ancestors; do not add a
duplicate FD, global FD-count oracle or serialized-suite assumption. Regression
gap: the current observer has no distinguishing check for shared-inode FD reuse.

**P2 S2-FR05 — the resource-failing EOF-tail fixture is not an EOF fixture.**

[with_incomplete_over_limit_file:2824](../../crates/market-data/src/daily_price_only/source.rs#L2824)
inserts `,{"sentinel":` before the files-array closing bracket, then appends the
old closing suffix at [2833](../../crates/market-data/src/daily_price_only/source.rs#L2833).
Its tail therefore contains `,{"sentinel":]}` rather than ending after the colon.
T08 installs it at [3186](../../crates/market-data/src/daily_price_only/source.rs#L3186).
With a permissive file count, typed serde encounters `]` where a value is required:
a non-EOF syntax error. With the tested count=1, FileLimitSeed rejects the second
element before parsing it. The test does distinguish pre-materialization count
rejection, but it does not establish the requested genuine serde-EOF/resource
contrast. There is no assertion that the permissive parse has is_eof()==true.

Consequence: R12b's required torn-tail resource case is still missing despite its
fixture name and reported disposition. The production side channel at
[2475](../../crates/market-data/src/daily_price_only/source.rs#L2475) correctly
prioritizes Resource over Json; no current production EOF-swallow defect was found.

Bounded correction: end the added second object at the colon without restoring
closing bytes. Establish actual legacy/bounded serde EOF under a larger file cap,
then SourceScanLimit under the one-file cap, with the same no-materialization and
no-data-tree observations. Retain the existing malformed-element case separately.
For total-file/unique-batch limits, public T08 currently asserts final errors only;
add corresponding no-tree/open observations if claiming their pre-open test proof.

**P2 S2-FR06 — T26 loses its existing bounded completion failure.**

[test:1697](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1697)
uses unbounded `done_rx.recv()` after the worker enters the existing blocking
append_manifest path at
[1691](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1691).
The pinned [baseline:1485](../../target/r2-daily-beta/wp-4-integration/wave26-S2-baseline-snapshot.txt#L1485)
used a five-second recv_timeout; the correction removes it and Duration.

Concrete trigger: a lock-release regression leaves a shared lock held after
drop(snapshot). The worker remains blocked acquiring the writer lock and retains
the completion sender. The new recv neither succeeds nor disconnects, so the test
cannot report the regression within its former bound. Consequence: a guard intended
to detect Drop failure can stall the entire test binary until an external job kill.
The initial attempt_rx and worker continuation waits are also unbounded. This is
a test-liveness regression, not a claim that the current production Drop leaks.

Bounded correction: restore bounded communication, retain the real pre-Drop
WouldBlock handshake, and make the post-Drop writer path detect unsuccessful lock
release before entering an indefinite blocking append. Ensure failure handling
does not unconditionally join a blocked worker. No sleep or production retry policy
is needed. Regression gap: only the success-path synchronization is asserted now.

**P2 S2-FR07 — T01 does not check all returned multi-file views.**

[test:426](../../crates/market-data/tests/daily_price_only_snapshot.rs#L426)
selects only `batches()[0].files().next()` for the file-name/kind/size/bytes/hash
assertions. Its fixture has two batches and three distinct files; the aggregate
calculation at [447](../../crates/market-data/tests/daily_price_only_snapshot.rs#L447)
reads fixture lengths, not the other returned views. The accepted
[T01 contract:728](2026-10-14-r2-raw-snapshot-implementation-contract.md#L728)
requires all bytes/hashes/sizes and borrowed file identity.

Concrete coverage gap: an iterator/view regression returning index0 for later
files would preserve reader stats and every currently checked first-file value.
The other written multi-file limit tests do not inspect those later views either.
Consequence: T01's required aligned-file getter oracle is incomplete. This is a
pre-existing unreported test gap discovered while reviewing the corrected iterator,
not a new production defect: the current move closure preserves the actual index.

Bounded correction: iterate both batches and every expected file in order, assert
iterator length, complete file identity/bytes/hash/size and its batch association;
compare each exact metadata commitment as well. Keep opaque-provider semantics.
Regression gap: later file-view alignment is not distinguished by the present suite.

**P2 S2-FR08 — final membership reports a lost captured binding as initial absence.**

[source.rs:1826](../../crates/market-data/src/daily_price_only/source.rs#L1826)
returns MissingCommittedEvidence whenever metadata or an expected filename is
absent. The same helper is used for initial membership, post-capture membership
at [1744](../../crates/market-data/src/daily_price_only/source.rs#L1744), and final
membership at [1307](../../crates/market-data/src/daily_price_only/source.rs#L1307).
The accepted [error contract:656](2026-10-14-r2-raw-snapshot-implementation-contract.md#L656)
distinguishes initial absence from a previously captured leaf: the latter is
SourceChanged.

Concrete trigger: at BeforeFinalNames, after final byte/stat validation succeeded,
rename a captured listed body to a location outside the batch. The retained file
is still owned, no extra batch entry is introduced, and final enumeration finds
its basename absent. It returns MissingCommittedEvidence before the later binding
revalidation can report SourceChanged. Consequence: the snapshot still rejects,
but returns the wrong fixed diagnostic class for a construction-time change.
This is a pre-existing unreported helper defect; moving the final hook ahead of
membership exposes a deterministic regression seam for it. No current success or
permission bypass is claimed.

Bounded correction: distinguish initial membership's missing error from the
post-capture/final pass and return SourceChanged for lost captured leaves. Keep
initial MissingCommittedEvidence and extra-entry UnexpectedBatchEntry unchanged.
Add a valid-control private final-phase move-outside fixture asserting the precise
SourceChanged result. Existing T24 tests replace names or add extras; none removes
a captured expected name from the final set. No public error/API or other file is
needed.

**P2 S2-FR09 — the temporary parsed metadata survives all body captures.**

[source.rs:1708](../../crates/market-data/src/daily_price_only/source.rs#L1708)
binds the complete parsed batch.json entry as `stored`. The equality check borrows
it at1709, but its enclosing scope is the whole capture_batch function through
[1758](../../crates/market-data/src/daily_price_only/source.rs#L1758). There is no
move, explicit release or shorter lexical scope after equality. Its owned strings,
query/header vectors and file entries therefore remain live while metadata bytes
and every provider body are captured, although the original manifest entry is
already retained. Last-use borrow analysis does not eagerly run these owned-field
destructors.

The accepted [P3 rule:341](2026-10-14-r2-raw-snapshot-implementation-contract.md#L341)
requires temporary parsed metadata to be released immediately; the same section
specifies discarding the metadata copy after equality. Trigger: any successful
nonempty batch, particularly a large permitted metadata record followed by large
body captures. Consequence: avoidable parsed-metadata memory overlaps all body
allocations, contrary to the fixed lifetime rule. This is bounded overhead, not a
claimed unbounded allocation, wrong retained-evidence stat or hard-RSS cap bypass.
Failure still drops it before the sole lock; R07's cleanup correction remains valid.

Bounded correction: place parse/equality in a shorter lexical scope that ends
before constructing/capturing the body-bearing batch. Preserve full equality and
the same typed errors. This pre-existing source gap needs no public type, storage,
dependency or contract edit. Current evidence-byte/FD observations do not observe
temporary parsed-metadata release.

**P2 S2-E01 — 15 pinned expected result paths name the wrong private module.**

[test inventory:12](../../target/r2-daily-beta/wp-4-integration/source-wave-26-S2-test-inventory.json#L12)
and its other14 private rows use `daily_price_only::source::linux::tests::...`.
The actual declaration is
[source.rs:2638](../../crates/market-data/src/daily_price_only/source.rs#L2638),
`mod private_tests`. Thus those actual result names start with
`daily_price_only::source::linux::private_tests::`. All45 short names, source lines,
platform splits and written states match; the15 fully qualified private names do not.

Trigger: a later collector matches direct result rows against cargoResultName.
Consequence: valid private results would be reported missing or left unmatched.
This is an input/evidence metadata discrepancy, not a source or executed CI defect.
Bounded correction belongs to the coordinator: preserve this frozen artifact and
record correct module paths in the next candidate inventory before hosted extraction.
This reviewer cannot edit inventories; no additional source/API/contract file is needed.

**Original R01–R16 and C01 dispositions**

Resolved means resolved at source-review level only. It never means compiled,
executed, accepted or operationally verified. Thirteen original groups are resolved;
R05/R10/R12 are partial. New findings above remain independent follow-up work.

| Original finding | Disposition | Current source/test evidence and consequence |
| --- | --- | --- |
| S2-R01 | Resolved | S204 calls the now pub(super) entry at S1051; Linux internals remain private. Parent access is corrected without public expansion. |
| S2-R02 | Resolved | S2040 explicitly declares Vec<PendingBatch> before first.entry at S2107; later duplicate retention/index behavior is preserved. |
| S2-R03 | Resolved | S348–349 moves the shared self reference into the returned map closure and retains index alignment and the borrowed ExactSizeIterator signature. T01 coverage remains S2-FR07. |
| S2-R04 | Resolved | I16 imports std::fs::hard_link; I1731/1763/1793/1806 use evidence_error; I1739/1771/1801/1817 use source().is_none(). All sentinel/hardlink cases remain. S2-FR01/02 are new private-target blockers. |
| S2-R05 | Partial | Getter callers S284/313/368, named ScanContext/ParsedManifest/TreeState, helper slices, generic checked_u64, I440 direct slice, I730 local initialization, I1718 NLL Drop and Linux-only PendingBatch remove the original listed patterns. New S3402 lint is S2-FR03; no strict pass is inferred. |
| S2-R06 | Resolved | S2272–2307 checks each known slot before second-value parse; Option<Option<String>> retains Some(None), so optional null duplicates reject too. I566 tests all8 incomplete second values against legacy rejection. Completed invalid second values are rejected by the same pre-check by inspection, but no separate completed-invalid-value fixture is written. |
| S2-R07 | Resolved, test qualification | ConstructionResources S751/775 owns manifest/directories/names before the last lock field. capture_batch/open_provider_file temporaries unwind before the outer scan owner. S3914 observes a real separate competitor's WouldBlock during cleanup, then availability after return. Its non-lock FD checks still have S2-FR04. |
| S2-R08 | Resolved | LockFingerprint S621 contains only dev/inode/mode/UID/nlink; S1484/1535 use it without reading lock bytes. S3584 changes same-inode contents/length/mtime and requires success; replacement/busy guards remain. |
| S2-R09 | Resolved | I750–755 checks67108864 success/67108865 InvalidLimits. S3681/3689 saves replaced metadata outside the leaf set, preserving SourceChanged. S3900 asserts actual WouldBlock instead of mapping through io_error; post-Drop lock acquisition remains. |
| S2-R10 | Partial | Process-wide FD counts are removed. S2537/2575 records construction-local opens and live-snapshot identities, but S3012 post-close identity reuse is still nondeterministic: S2-FR04. |
| S2-R11 | Resolved | I934/944 restores valid depth32 bytes and calls valid_control before independent root-byte/component cap faults. The preceding depth33 row no longer masks them. |
| S2-R12 | Partial | All8 groups inspected separately below. New read/ordering/race assertions are substantial; EOF shape and post-close native resource proof remain deficient. |
| S2-R13 | Resolved | S1379–1387 maps negative/nonrepresentable actual stat sizes to SizeMismatch; cap exceedance stays SourceScanLimit. S3053 covers negative and cfg32/cfg64 conversion cases without allocating giant files. 32-bit execution remains unknown. |
| S2-R14 | Resolved | S986–1002 validates every lexical component before the first reserve/copy. S3439 observes no root/name copy, reservation or open for parent/relative faults. |
| S2-R15 | Resolved | S1244 validates every retained recorded size before descriptor/data-tree preflight/capture. S1390 checks positive/cap; r15 at S3471 asserts no tree phase or metadata/body descriptor on zero/over-cap rows. Actual size/hash checks remain later. |
| S2-R16 | Resolved | S2757 and I160 protect every newly created Raw-down fixture directory, with explicit fresh-fixture UID. Existing intentional fault modes and I1852 writable outside ancestry are preserved. No host-umask or production repair assumption is used. |
| C01 | Resolved | S3592 clones hook_path; S3598 moves/uses the clone; S3611 still borrows the original path for the post-hook inode/length check. No moved original binding remains in that case. Formatting is not borrow-check execution. |

S = current source.rs; I = current daily_price_only_snapshot.rs. The detailed
anchors above and the guard/name matrix below resolve these shorthand line citations.

| R12 group | Disposition | Distinguishing written assertion / remaining limit |
| --- | --- | --- |
| a — bounded read/allocation | Resolved at source level | S3071 observes exact3-byte capture plus one EOF probe; short read, growing read, cap-crossing probe and expected4/cap3 rejection with zero reads/events. I766 covers actual metadata/body exact/under caps; I665 charges torn-tail bytes. |
| b — seed/EOF resource guard | Partial | S3134 records exactly one materialized FileEntry, no BatchTreeOpen and no metadata/provider open for the invalid second element. S2-FR01 blocks compilation; S2-FR05's alleged EOF candidate is non-EOF malformed. I793's total-file/batch cases lack direct pre-open observations. |
| c — bytes/descriptors/duplicates | Partial | S3261 matches live owned opens, reserve-before-open events, bytes-reserve-before-buffer events, actual captured sums and rejected provider allocation/open; I610 plus S3239 checks exact/under duplicate budget and Arc count2. I860 covers exact/under FD formulas, shared date and deep root. Post-close evidence remains S2-FR04; new last() warning remains S2-FR03. |
| d — path levels | Resolved at source level | S3439 observes Linux relative/parent rejection before copying/opening. I1002 adds separate raw/manifests/provider/market/data-provider/data-market/date/batch/metadata/body/control/ancestor symlink controls, plus ordinary non-directory path. |
| e — metadata mode | Resolved at source level | I1372 changes only batch.json mode to Credentialed after valid_control and demands BatchMetadataMismatch; committed mode/request/body stay unchanged. |
| f — final membership/binding | Resolved at source level | S3710 adds a leaf at BeforeFinalNames and asserts invoked plus UnexpectedBatchEntry. S3725 replaces same-byte metadata at that phase with saved inode outside the set and demands SourceChanged. The hook now precedes final membership at S1299. |
| g — older partition/later interval | Resolved at source level | I1545 retains an earlier2020 partition carrying F_DT20261012/T_DT20261014, asserts both batches and exact captured query metadata. It does not assert provider interval truth. |
| h — actual writer attempt | Original gap resolved; new liveness defect | I1679 records real separately opened exclusive WouldBlock while snapshot is alive; parent waits for that observation before Drop/continue. Append and fresh capture follow Drop. No pre-Drop blocking append itself is claimed. S2-FR06 removes bounded completion failure. |

**P1–P10 preservation**

| Accepted contract | Source disposition | Evidence and practical qualification |
| --- | --- | --- |
| P1 API/private ownership | Inspected match | S39–403 retains every frozen public type/signature, private construction/fields and the two crate-only entry getters. No public FD/path/Read/Seek/mutable/into extractor, diagnostic/serialization/clone/default traits on capability representations or root re-export. Test-only TreeState diagnostics are distinct from production capability containers. New private contexts do not widen the capability. |
| P2 scope/full discovery | Inspected match | S45 exact kis/kis-calendar+kr and supplied UID!=MAX; UID0 has no bypass. S2035 scans the complete captured manifest; first unique order and sorted lookup index coexist. S262 matches all six SourceFileReference fields, rejects unsafe/reserved names first, and performs no copy/path/date fallback. |
| P3 finite bounds/resources | Bounds inspected; temporary lifetime defect | S99 validates all12 positive caps, checked+1/layouts,64MiB body/depth32 policy. S809/850 reserves descriptors/evidence; S971 validates roots; S1247 checks formula; S1979 bounds allocation/read. R13–R15 corrections are present. S2-FR09 delays parsed-metadata release; native cleanup evidence remains S2-FR04. Neither is a retained-byte cap bypass. |
| P4 Raw bytes/schema/parity | Inspected source match; tests partial | S2035 keeps LF-excluding ranges/physical lines, CR/spaces/key order/full manifest bytes, pre-dedup counts, first equal duplicate and full Eq conflict rejection. S2257 preserves optional/unknown fields and pre-value known-duplicate rejection; S1708 compares complete parsed metadata. Resource errors precede EOF handling. S2-FR02/05 qualify parity/tail tests. |
| P5 captured bytes/descriptors/path trust | Inspected with typed-error defect | All opens at S916/1091/1408/1484/1554 are descriptor-relative readonly no-follow, with type/UID/mode/link checks and binding comparisons. Outside ancestors are not assigned Raw UID. Evidence fingerprints include size and nanosecond mtime/ctime, excluding atime; directory and lock policies are separate. Final stream/path/membership checks reject changes; lost captured membership is misclassified in S2-FR08. |
| P6 one lock through Drop | Inspected source match; test defects | One S1172 nonblocking shared attempt on existing verified lock, locked ancestry/binding revalidation before manifest, no retry/dup/create. S751/775 and S795 transfer/drop lock last; returned views only borrow captured storage. S2-FR04/06 affect evidence quality/liveness, not an identified current unlock defect. |
| P7 finite redacted errors | Fieldless API match; one mapping defect | S405 gives29 fieldless variants; S438 fixed codes, S474 fixed Display and S480 source None. No StoreError growth, parser/path/body/errno diagnostics or logging. S2-FR08 must distinguish lost captured membership from initial absence. S2-FR02 must be fixed without adding sensitive parser Debug. |
| P8 scope/preservation | Match | Supplied exact delta changes only source.rs and snapshot test: +1796/-382. Nine-line registration and all19 frozen references remain exact. No dependency/workflow/storage/S1/wire/historical/normalizer/collector edit by this review or correction delta. |
| P9 meaningful regressions | Partial | All45 definitions and32 guards inspected. S2-FR01–07 obstruct compilation, strict lint, deterministic cleanup proof, EOF distinction, liveness or complete getter coverage; S2-FR08 lacks a precise late-absence regression. Written tests remain unexecuted. E01 also needs coordinator-owned evidence correction. |
| P10 Linux-only/no-I/O elsewhere | Inspected cfg match; unexecuted | S147/504/512 gate Linux-only layout/pending/machinery; S206–209 returns UnsupportedPlatform before I/O on every other target, including other Unix. Public signatures remain available. I1866 guards the one non-Linux test. No platform build/run occurred. |

The whole898-line accepted contract was read, including accepted P1–P9 and
section12's binding P10. Historical proposal wording is not an unresolved gate.
D4/D5 remain950/740 lines at their fixed hashes. The snapshot is opaque unapproved
storage evidence; it is not provider truth, reconstructed/copy evidence or admission.

**Resource, parser and lifetime reasoning**

Every production open uses DescriptorBudget::open before the actual open/openat.
There is no dup/try_clone and RawDir borrows the existing batch FD with a4KiB buffer
and rewind. The empty minimum is C+7. For nonempty scope it is
C+9+D+2B+F, with checked additions/multiplication, retained shared dates once and
no hidden transient FD. Successful current/peak counts accumulate actual owned
opens; live-snapshot observations match those opens on the written private fixture.
No claim is made about global process FD/RSS budgets or all recoverable OOM.

Aggregate evidence is the complete manifest once plus each distinct metadata/body
capture. It is charged before buffers are reserved/captured; provider charge also
precedes its open. Metadata stat/open is needed before its evidence length charge.
The manifest Vec moves into one Arc without copying, lines are ranges, temporary
duplicate entries are discarded, and no provider body is parsed. serde/domain
allocation overhead remains the accepted bounded-but-not-fully-fallible limitation.

Parsed batch.json's temporary copy is not retained in the returned snapshot, but
currently survives body capture until capture_batch returns; FR09 requires its
earlier release after equality. This qualification is separate from failure
unwind order and from the correctly charged evidence buffers.

ConstructionResources contains the sole HeldLock as its final field. On an error
inside capture_batch/open_provider_file, local body/metadata buffers and Files drop
before returning through the outer scan frame. Completed batches declared after
scan drop before scan's resource owner. That owner's Drop closes manifest,
directories and names before the lock field's automatic destruction. After final
checks, into_resource_guard takes the existing resources, sets transferred, and
performs no new descriptor open or lock transition. On success snapshot fields drop
batches before ResourceGuard; the latter drops manifest/directories/names then lock.
No new release gap or ownership extractor was found. Cleanup-time WouldBlock is a
real attempt on a separate open description, though its FD assertions remain FR04.

The separate lock fingerprint intentionally ignores size/content/mtime/ctime.
Evidence fingerprints retain size and nanosecond timestamps; directories exclude
child-sensitive size/link/time. Initial no-follow pre-stat/open/fstat/named checks,
final bounded byte comparisons bracketed by stat checks, final membership and
named directory/leaf/lock checks remain. The BeforeFinalNames hook now precedes
membership, so the new late extra-file fixture observes the final pass itself.
Lost/changed paths fail closed, with FR08's captured-membership diagnostic discrepancy;
no repair, permission fallback or writer
activation was added. Advisory locking and sequential checks do not prove immunity
to ABA, uncooperative trusted writers, hostile mounts or continuous post-return
attestation. These accepted limitations are not new defects.

Known duplicate top-level fields, including optional null, reject before the second
value is consumed. Unknown Raw fields remain IgnoredAny; complete schema equality
retains file order, request, mode, entitlement, scope, date/ID/time and optional
copy/continuation. Full manifest bytes include blank lines and ignored EOF tails.
Terminated nonblank rows count before parse; complete unterminated rows count before
dedup; only actual Json is_eof on the final unterminated row is ignored. Explicit
depth/file/allocation resource failures return typed errors instead. FR05 concerns
the written oracle for that distinction, not a contrary observed production path.

**Guard-to-written-test matrix**

All names below are definitions, not result rows. Match means the guard/oracle
was inspected; it does not mean an executed pass. The Linux private unit target
has FR01/02 blockers and FR03 warning exposure. Public integration/private tests
must both be included in later exact-candidate verification. Names and locations
cover all45 definitions, including r15_recorded_body_size_precedes_data_tree_and_leaf_io.

| Guard and current source | Written definition(s) | Control, isolated operation/fault and oracle disposition |
| --- | --- | --- |
| S2-T01; S225/301/348/1343 | [s2_t01_valid_multi_batch_snapshot_keeps_exact_evidence_and_stats](../../crates/market-data/tests/daily_price_only_snapshot.rs#L397) | Partial, FR07. Two-batch/three-file valid capture; first order, first file identity/bytes/hash/size, first metadata hash and retained/FD stats asserted. Only the first file view is inspected; later aligned views and every batch commitment lack the required complete oracle. |
| S2-T02; S2035/2057/2132 | [s2_t02_line_bytes_keep_whitespace_crlf_order_and_physical_line](../../crates/market-data/tests/daily_price_only_snapshot.rs#L475) | Match. Valid control and initial S1 entry/line/physical-line parity precede CRLF, spaces, blank lines and key-order rewrite. Exact LF-excluding first line/hash, second line at physical3, full-byte hash and count2 distinguish preservation. Modified-case S1 parity is not separately compared. |
| S2-T03; S2058–2085/2257/2475 | [s2_t03_bounded_entry_sequence_schema_matches_legacy_serde](../../crates/market-data/src/daily_price_only/source.rs#L3215); [s2_t03_final_record_eof_tail_and_terminated_errors_are_distinct](../../crates/market-data/tests/daily_price_only_snapshot.rs#L534); [s2_t03_known_duplicate_fields_reject_before_an_incomplete_second_value](../../crates/market-data/tests/daily_price_only_snapshot.rs#L566) | Match for public EOF/duplicate cases; private sequence comparison blocked by FR02. Valid complete final row/count1; torn tail remains byte-captured/count1; adding LF rejects it. All8 unterminated known-duplicate tails reject before second value, including first optional null, against legacy rejection. Genuine resource/EOF distinction remains FR05/T08. |
| S2-T04; S2102–2143 | [s2_t04_identical_rows_retain_one_batch_arc_and_one_evidence_charge](../../crates/market-data/src/daily_price_only/source.rs#L3239); [s2_t04_equal_duplicates_keep_first_line_and_conflicts_fail_closed](../../crates/market-data/tests/daily_price_only_snapshot.rs#L610) | Match. Valid one-batch control, whitespace-equivalent duplicate count2/first line/position/batch and file count1, then entitlement conflict -> ManifestConflict. Exact and one-under retained budgets and private strong_count2 distinguish one manifest Arc plus one distinct batch capture. |
| S2-T05; S1206/1984/2059/2079 | [s2_t05_manifest_byte_and_pre_dedup_record_caps_are_exact](../../crates/market-data/tests/daily_price_only_snapshot.rs#L665) | Match. One complete unterminated row succeeds at byte/record cap; one-under bytes fails. Duplicate complete final row cannot evade record cap. Ignored torn-tail bytes over manifest budget reject rather than producing a truncated success. T04 supplies equal-duplicate record controls. |
| S2-T06; S99/178/1379 | [s2_t06_stat_size_conversion_uses_size_mismatch_for_nonrepresentable_values](../../crates/market-data/src/daily_price_only/source.rs#L3053); [s2_t06_zero_overflow_and_policy_invalid_limits_fail_in_construction](../../crates/market-data/tests/daily_price_only_snapshot.rs#L714) | Match in written constructor/helper assertions. All12 individual zeros, byte max+1/layout overflow, body67108864/67108865 and depth33; actual stat-size helper negative and cfg32/cfg64 cases. Constructors contain no filesystem work. No huge allocation/file or platform execution is claimed. |
| S2-T07; S1390/1425/1979 | [s2_t07_bounded_reads_probe_once_and_reject_caps_before_allocation](../../crates/market-data/src/daily_price_only/source.rs#L3071); [s2_t07_batch_metadata_and_body_byte_caps_precede_capture](../../crates/market-data/tests/daily_price_only_snapshot.rs#L766); [r15_recorded_body_size_precedes_data_tree_and_leaf_io](../../crates/market-data/src/daily_price_only/source.rs#L3471) | Match for bounded read/capture order. Valid actual metadata/body capture at exact caps, one-under each; CountingReader exact3 bytes plus single EOF probe, short/growing input, cap-crossing probe and rejected expected4/cap3 with zero calls/events. r15 observes no tree/metadata/provider open for known recorded zero/over-cap faults. Torn bytes are charged in T05. |
| S2-T08; S2112/2391/2445 | [s2_t08_file_seed_rejects_before_materialization_and_data_tree_open](../../crates/market-data/src/daily_price_only/source.rs#L3134); [s2_t08_per_batch_and_total_file_counts_fail_before_body_open](../../crates/market-data/tests/daily_price_only_snapshot.rs#L793) | Partial, FR01/05. Public valid multi-file control -> per-batch/total-file/unique-batch errors; many small query fields meet exact metadata cap and fail one-under. Private exact-one-file control and invalid next object observe only one materialized entry and no data tree/body/metadata open. It consumes ScanLimits twice and the supposed EOF candidate is non-EOF malformed; total/batch pre-open timing lacks direct observations. |
| S2-T09; S849/1217/1591/1696 | [s2_t09_t10_owned_resource_observations_match_the_capture](../../crates/market-data/src/daily_price_only/source.rs#L3261); [s2_t09_aggregate_retained_bytes_accept_exact_and_reject_one_under](../../crates/market-data/tests/daily_price_only_snapshot.rs#L830) | Partial evidence, FR03/04. Valid two-file aggregate succeeds exactly and fails one-under although individual caps fit. Private event trace compares actual bytes, reserves before buffer allocation and provider open, and rejects a final body before allocation/open. Duplicate/Arc exact budget in T04 counts batch capture once. Post-error closure proof still uses reusable FD slots; the parsed-metadata release required by P3 is not observed (FR09). |
| S2-T10; S808/1061/1247/1761 | [s2_t10_descriptor_formula_is_measured_and_preflighted](../../crates/market-data/tests/daily_price_only_snapshot.rs#L860); [s2_t09_t10_owned_resource_observations_match_the_capture](../../crates/market-data/src/daily_price_only/source.rs#L3261) | Partial evidence, FR04. Valid two-batch exact FD formula, one-under failure, same-date sharing and deep-root counters; private actual live owned descriptors/reservations/attempts match peak/current, unique FD numbers, and no data-tree open when full preflight fails. Every production open site is charged; no dup exists. Post-close liveness is not deterministic ownership proof. |
| S2-T11; S971/2067/2487 | [s2_t11_root_path_and_unknown_json_depth_caps_are_exact](../../crates/market-data/tests/daily_price_only_snapshot.rs#L914) | Match. Valid unknown depth32 and exact root byte/component caps precede isolated depth33 rejection. Valid depth32 bytes are restored and control succeeds before each one-under root budget. Private T13 directly observes lexical root rejection before reserve/copy/open. The depth pass is byte-bounded and nonallocating; not a validity classifier. |
| S2-T12; S1131/1162/1196 | [s2_t12_missing_controls_do_not_repair_and_empty_manifest_is_valid](../../crates/market-data/tests/daily_price_only_snapshot.rs#L956) | Match. Separate valid controls before missing lock, manifest, manifest parent and Raw; precise MissingControl plus absent paths/no repair. Existing empty controls/manifest succeed with zero batches and C+7 descriptors. No creation or legacy reconciliation path exists in production. |
| S2-T13; S916/971/1408/1484/1554 | [s2_t13_parent_and_relative_roots_reject_before_component_copy_or_open](../../crates/market-data/src/daily_price_only/source.rs#L3439); [s2_t13_root_control_batch_and_leaf_symlinks_fail_closed](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1002) | Match. Valid controls precede distinct root/ancestor/Raw/control manifests/provider/market/data provider/market/date/batch/batch.json/body/manifest/lock symlinks and ordinary non-directory component. Private parent/relative roots return UnsafePath with no copy/reserve/open events. No canonicalize/retry/follow fallback. |
| S2-T14; S559/629/651/667 | [s2_t14_synthetic_raw_stat_owner_mode_and_link_guards_include_uid_zero](../../crates/market-data/src/daily_price_only/source.rs#L3030); [s2_t14_special_nodes_owner_and_mode_guards_do_not_block_or_bypass](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1197) | Match for specified fixture/synthetic observations. Valid control before body FIFO/directory, writable body/batch and wrong explicit UID; typed NonRegularFile/WritableNode/UntrustedOwner. Pure stat checks UID0 exact-match/wrong-owner/no bypass, outside policy and link1/2. No physical cross-principal access, actual special-device open or ACL proof is claimed. |
| S2-T15; S592/639/1424/1579 | [s2_t15_hardlinked_lock_manifest_metadata_and_body_are_rejected](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1263) | Match. Independent valid controls then hardlink body/manifest/lock/metadata outside the batch set, each requiring HardLinkedFile. Correct std::fs import. Production compares nlink, not hash uniqueness; distinct independent equal-byte files remain legal, though no dedicated equal-hash pair fixture is present. |
| S2-T16; S1708/2087 | [s2_t16_scope_and_complete_batch_metadata_equality_are_required](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1311) | Match. Fresh valid controls, isolated committed provider/market scope faults, and batch.json date/ID/request/time/mode/size/order mismatch faults. Exact ScopeMismatch/BatchMetadataMismatch assertions; I1372 mode case is newly added. Full ManifestEntry Eq includes every frozen field, not selected fields or byte reserialization. |
| S2-T17; S486/2090/1785/1300 | [s2_t17_duplicate_reserved_traversal_and_extra_entries_fail](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1408) | Initial/extra-entry guards match; late captured absence is partial, FR08. Valid controls before duplicate identical listed names, reserved name, traversal, extra temp and subdirectory; precise DuplicateFile/InvalidFileName/UnexpectedBatchEntry. Private T24 adds an extra leaf at BeforeFinalNames and observes the final pass. A captured leaf moved outside is untested and misclassified. That extra-leaf case has phase-reach proof rather than separate full pre-scan. |
| S2-T18; S1390/1579/1625/2487 | [s2_t18_missing_invalid_zero_size_and_same_size_corruption_reject](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1466); [r15_recorded_body_size_precedes_data_tree_and_leaf_io](../../crates/market-data/src/daily_price_only/source.rs#L3471) | Match. Valid controls before missing body/metadata, zero recorded size, truncation, same-size changed hash and torn metadata, each with precise error. r15 supplements pre-data-I/O observations for zero/over-cap records. No batch is skipped after an error. Its over-cap fixture also differs from stored metadata; the no-tree hook makes the asserted preflight boundary distinguishing. |
| S2-T19; S1276/1640 | [s2_t19_uncommitted_orphans_are_not_opened_or_reconciled](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1517) | Match. Valid committed control plus complete and incomplete uncommitted orphans; capture exposes only committed ID. Full fixture tree content, mode, UID, inode/device/link state compares equal after Drop, excluding atime; orphan paths remain. No date/provider directory enumeration or repair in source. |
| S2-T20; S1276/1664/352/387 | [s2_t20_batch_partition_date_does_not_limit_committed_discovery](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1545) | Match. Earlier2020 partition carries later20261012–20261014 F_DT/T_DT; known-valid scan preserves both batches/order/dates and exact captured query. No partition or caller civil-date filtering. This asserts metadata availability, not verified provider interval or chain choice. |
| S2-T21; S262 | [s2_t21_exact_lookup_matches_full_file_identity_without_copy_fallback](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1575) | Match for written public cases; other field checks inspected directly. Valid capture -> exact hit, absent batch/file, provider/hash/time mismatch and absent copied-source filename with precise errors. Source also checks market and unsafe/reserved reference name; those lookup-only branches lack isolated written cases. Lookup uses retained views only, never pathname/copy fallback. |
| S2-T22; S1172/751/795 | [s2_t22_two_readers_hold_shared_lock_until_both_drop](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1618) | Match. Two successful readers remain simultaneous; separately opened exclusive writer fails before Drop and after first Drop, succeeds after second Drop. No dup competitor. Public test uses is_err for contention; T23/private T28 additionally assert actual WouldBlock. |
| S2-T23; S1162/1172/1186/1535 | [s2_t23_lock_path_replacement_before_and_after_shared_acquisition_fails](../../crates/market-data/src/daily_price_only/source.rs#L3564); [s2_t23_same_lock_inode_ignores_contents_length_and_timestamps](../../crates/market-data/src/daily_price_only/source.rs#L3584); [s2_t23_exclusive_writer_is_reported_busy_without_retry](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1637) | Match. Valid control before existing exclusive writer -> real competitor WouldBlock and snapshot LockBusy; unlock restores success. Private replacement before/after shared acquisition is invoked and SourceChanged, never adopting replacement. New same-inode content/length/mtime mutation succeeds with dev/inode unchanged; C01 correctly retains post-hook path. |
| S2-T24; S1287/1299/1313/1336 | [s2_t24_replacements_at_capture_and_final_hooks_are_rejected](../../crates/market-data/src/daily_price_only/source.rs#L3618) | Partial, FR08; written replacement oracles are distinguishing. Valid pre-scans in replacement helper; changed Raw/control/manifest/batch/metadata/body bindings at named capture/final phases -> invoked and SourceChanged, including same-byte metadata saved outside set. BeforeFinalNames now precedes membership; late extra -> UnexpectedBatchEntry, metadata replacement -> SourceChanged. Moving a captured basename outside at that final phase is untested and returns MissingCommittedEvidence. |
| S2-T25; S559/904/1888/1905 | [s2_t25_in_place_bytes_mode_and_link_mutations_fail_revalidation](../../crates/market-data/src/daily_price_only/source.rs#L3743) | Match. Valid replacement-helper controls before in-place manifest/metadata/body and mode changes; invoked -> SourceChanged. External hardlink -> HardLinkedFile at actual AfterBodyCapture; synthetic owner guard tests wrong UID, without chown. No ABA/mutate-restore or continuous-attestation claim. Link case has successful capture-phase reach rather than separate full pre-scan. |
| S2-T26; S1172/795; I1662 | [s2_t26_cooperating_append_publishes_after_snapshot_drop](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1662) | Partial test readiness, FR06. Successful snapshot while next batch is still uncommitted. Separate worker performs actual nonblocking exclusive attempt and sends WouldBlock before parent drops snapshot; continue handshake starts append after Drop; fresh capture asserts second committed ID. This closes original R12h attempt ambiguity, but removes prior bounded completion and can hang on a release regression. |
| S2-T27; S383/1620 | [s2_t27_returned_bytes_remain_captured_after_disk_mutation](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1705) | Match. Successful snapshot and exact lookup are the control; post-return body mutation violates owner protocol deliberately. Borrowed captured bytes remain equal to prior bytes; Drop then fresh scan -> HashMismatch. Removed non-Drop view drop is compatible with last-use/NLL. No disk-validity promise after return. |
| S2-T28; S751/775/795 | [s2_t28_injected_failures_release_lock_descriptors_and_preserve_tree](../../crates/market-data/src/daily_price_only/source.rs#L3831); [s2_t28_failure_unwind_closes_temporary_leaves_before_lock_release](../../crates/market-data/src/daily_price_only/source.rs#L3914) | Partial native closure oracle, FR04. Valid controls before8 injected phases, invoked plus Io, scoped opened-descriptor observations, cleanup event, unchanged complete tree and post-return exclusive lock. Normal Drop also observes held descriptors and actual WouldBlock then post-Drop exclusive lock. Added AfterBodyCapture failure -> BeforeLockRelease sees sole lock still live/contended and temporary leaves gone; post-close common-ancestor identity assertions remain racy. |
| S2-T29; S405/438/474/480 | [s2_t29_fieldless_errors_never_format_raw_sentinels](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1723) | Match for finite diagnostic source and written sentinel cases. Valid controls before malformed mode/hash/UUID/name and request/copy/entitlement sentinel with bad hash, plus missing-path OS case. Rendering does not contain sentinel; all four source() checks are is_none; first Display equals code. No wrapped error/log in production. All remain unexecuted; no raw diagnostic was printed. |
| S2-T30; S2257/2391/2487/1620 | [s2_t30_unknown_fields_optional_raw_fields_and_invalid_provider_body_stay_opaque](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1821) | Match. Known-valid fixture then bounded unknown Raw field and omitted FileEntry continuation/copy fields; full metadata Eq succeeds and correct-hash non-JSON provider body remains opaque bytes. Parser parity is retained, not strict D4 wire validation or provider/schema admission. |
| S2-T31; S206/504/512; I1866 | [s2_t31_non_linux_returns_unsupported_without_path_io](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1868) | Static cfg match; excluded on Linux. Fixed portable signatures and valid explicit scope/limits with relative nonexistent root -> UnsupportedPlatform. Source branch contains no I/O; no non-Linux compilation or filesystem observation executed. |
| S2-T32; S55/578/651/916 | [s2_t32_explicit_fixture_uid_and_writable_outside_ancestor_policy_are_preserved](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1852) | Match for fixture policy; operational proof excluded. Explicit actual fresh Raw UID and writable outside data-root mode777 permit known-valid capture; T14 wrong expected UID rejects. Synthetic uid0 check does not bypass ownership. No actual other principal/ACL/mount/denied-access experiment, repair or fallback is written/claimed. |

**Input identity, checked files and preservation**

The186-line [Wave26 inventory](../../target/r2-daily-beta/wp-4-integration/source-wave-26-S2-review-inputs.json)
has SHA256 `9bffe79342b2a5a1b61691ff63b6a1b36a992ddb9357d763479ae21272cc83bf`.
Its sourceCommit and baselineSourceCommit equal the handoff/comparison above.
All38 pins matched SHA256 and line counts at startup and final verification:
3 targets,19 readonly references,16 evidence artifacts. E01 is a semantic metadata
discrepancy inside a correctly hashed artifact, not a byte-pin mismatch.

The supplied baseline text copies match the original target hashes/counts.
An inert line-by-line reconstruction of every supplied diff hunk matched both
current target texts exactly: source +1511/-328, integration +285/-54; combined
+1796/-382, two files only. This inspected diff text, not executable source, and
did not invoke Git or write a temporary artifact. mod.rs is unchanged. The19
reference hashes also match the original review's preserved inputs, including
existing storage/S1/writer/orphan behavior, historical sources, wire and Cargo.
No Git object/HEAD/branch query was made: source identity uses the pinned bytes
and supplied commit inventory, as required by the no-Git instruction.

| Pin group / checked file | Lines | SHA256 | Start/final |
| --- | ---: | --- | --- |
| targets: [crates/market-data/src/daily_price_only/source.rs](../../crates/market-data/src/daily_price_only/source.rs) | 3987 | `4577d5f10bbe4dda508c89b25ff04c638a274bc5050e5f9e9b7e03b5ba4d8bb5` | MATCH / MATCH |
| targets: [crates/market-data/tests/daily_price_only_snapshot.rs](../../crates/market-data/tests/daily_price_only_snapshot.rs) | 1896 | `754a32ec22ba32f7b6c57da9e3dfb335cdeb064926ecedd1591f4ff3c74d9dd7` | MATCH / MATCH |
| targets: [crates/market-data/src/daily_price_only/mod.rs](../../crates/market-data/src/daily_price_only/mod.rs) | 9 | `700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/storage.rs](../../crates/market-data/src/storage.rs) | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/contract.rs](../../crates/market-data/src/contract.rs) | 398 | `27b0a4d71f55ea2fea89eb54b6fe92483f859f2ae59c608885415a1eb680d3d4` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/normalize.rs](../../crates/market-data/src/normalize.rs) | 2019 | `be5b4d3444b355c40295ca33bdc9d40e3aa1e359146fcaca293b4a1f745cd6b9` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/historical_price_only.rs](../../crates/market-data/src/historical_price_only.rs) | 1052 | `d30e7a95bb695c180e47eedb6b11209dc7fceb598a65cc4b60a16456a48a4b52` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/historical_price_only_artifact.rs](../../crates/market-data/src/historical_price_only_artifact.rs) | 3924 | `60298769aa267a4cd32d7e0c3343b76f65fddd79a0b449bd1881d53408fb6065` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/daily_price_only/contract.rs](../../crates/market-data/src/daily_price_only/contract.rs) | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/lib.rs](../../crates/market-data/src/lib.rs) | 288 | `fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/Cargo.toml](../../crates/market-data/Cargo.toml) | 36 | `d2af9a324bf1e421c23f4c15b5768882f8b041502e351f7e1f1ac671c95a1997` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/tests/daily_price_only_storage.rs](../../crates/market-data/tests/daily_price_only_storage.rs) | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` | MATCH / MATCH |
| readOnlyReferences: [docs/contracts/owner-beta-daily-etf11-input-v1.md](../../docs/contracts/owner-beta-daily-etf11-input-v1.md) | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-daily-input-design.md](../../docs/reviews/2026-10-14-r2-daily-input-design.md) | 926 | `e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-daily-input-design-amendments.md](../../docs/reviews/2026-10-14-r2-daily-input-design-amendments.md) | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-raw-manifest-evidence-review.md](../../docs/reviews/2026-10-14-r2-raw-manifest-evidence-review.md) | 290 | `e0322d1774dbd0124d1619f7f69be983dc43a035df8d71ac8ec324d5e43c089b` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-raw-manifest-corrections-review.md](../../docs/reviews/2026-10-14-r2-raw-manifest-corrections-review.md) | 199 | `f65bb3e3a47f8cea39dfccd74656602639340249f56ddbc76881fc846eb5e88b` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-caller-role-raw-ci.md](../../docs/reviews/2026-10-14-r2-caller-role-raw-ci.md) | 26 | `989a3891493f3ece231de15050f6bb3d139f1034edf3c0fe87e4aef4b686a47c` | MATCH / MATCH |
| readOnlyReferences: [Cargo.lock](../../Cargo.lock) | 5609 | `d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-raw-snapshot-implementation-contract.md](../../docs/reviews/2026-10-14-r2-raw-snapshot-implementation-contract.md) | 898 | `c6f15291ba24eef23421f3e80ead194bb4e5adac6a5733271eb0aeb05cbe6056` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/src/historical_price_only_v3_artifact.rs](../../crates/market-data/src/historical_price_only_v3_artifact.rs) | 2487 | `9a604f95e9d5df92f1a54c3b3f23b8221513c19c1fc64f135c826947c6b02244` | MATCH / MATCH |
| readOnlyReferences: [crates/market-data/tests/daily_price_only.rs](../../crates/market-data/tests/daily_price_only.rs) | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` | MATCH / MATCH |
| evidenceArtifacts: [docs/reviews/2026-10-14-r2-snapshot-source-handoff.md](../../docs/reviews/2026-10-14-r2-snapshot-source-handoff.md) | 50 | `78b54aa391bcee322b3ac057db44b21c5bbbe9b1a9f9410565be863e398a90fd` | MATCH / MATCH |
| evidenceArtifacts: [docs/reviews/2026-10-14-r2-admitter-helper-ci-recovery.md](../../docs/reviews/2026-10-14-r2-admitter-helper-ci-recovery.md) | 77 | `1cceaae4932ee3671538fb77e7665ce7de2e0bc1e7c19cb7b81251df466ee156` | MATCH / MATCH |
| evidenceArtifacts: [docs/reviews/2026-10-14-r2-admitter-helper-ci.md](../../docs/reviews/2026-10-14-r2-admitter-helper-ci.md) | 118 | `2587d1804ee035d7e30a618bd79ed7802b48a73596b2aa7767a4b3a682df7200` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/ci/a888cd6/ci-evidence.json](../../target/r2-daily-beta/ci/a888cd6/ci-evidence.json) | 1455 | `77241980b864ac78fcfb1168b2ae49a34f9c1c5ec99dfd7a00aab4474c41d5fb` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave22-I0c-CI-coordinator-verification.json](../../target/r2-daily-beta/wp-4-integration/wave22-I0c-CI-coordinator-verification.json) | 444 | `63c31c8ccf7c38e505f6052e48ce4381544a3d4260a25efa52a5096f70858eed` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave23-I1b-S2-final.md](../../target/r2-daily-beta/wp-4-integration/wave23-I1b-S2-final.md) | 64 | `699a37b5a1e8e4ff4d55feba04e13bb53d694a1ec6583b9b162171b745993b6c` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-23-S2-test-inventory.json](../../target/r2-daily-beta/wp-4-integration/source-wave-23-S2-test-inventory.json) | 78 | `380b2c761202baa8291d22a1c84213f06fe2637bf77e7ea9aab906e45723aede` | MATCH / MATCH |
| evidenceArtifacts: [docs/reviews/2026-10-14-r2-raw-snapshot-source-review.md](../../docs/reviews/2026-10-14-r2-raw-snapshot-source-review.md) | 496 | `60970b3a03cd81899e74e2f8ad4fab6713cd1461ac8cf4f5afa4c97179ba5cf7` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave25-I1b-S2-F-final.md](../../target/r2-daily-beta/wp-4-integration/wave25-I1b-S2-F-final.md) | 78 | `2e3b32b5112d804b53b471ba7a01ecb8cc99274eab8873a5efa2aa8bc627a6ca` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave25-I1b-S2-F-coordinator-verification.json](../../target/r2-daily-beta/wp-4-integration/wave25-I1b-S2-F-coordinator-verification.json) | 61 | `5d71a98b56af2220dd0054794079d93cd559f0cde5d7e608c80f2fd9774e316c` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-26-S2-test-inventory.json](../../target/r2-daily-beta/wp-4-integration/source-wave-26-S2-test-inventory.json) | 414 | `2902947e9bccc7ad5acc4f2470ef8c9ffff04fe9c8bc7e1153c972bb71b6f441` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-S2-baseline-source.txt](../../target/r2-daily-beta/wp-4-integration/wave26-S2-baseline-source.txt) | 2804 | `41ae987cd9b50abebf44d1cb01f44311317d3c73f5ed62907afa43b86a2ba092` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-S2-baseline-snapshot.txt](../../target/r2-daily-beta/wp-4-integration/wave26-S2-baseline-snapshot.txt) | 1665 | `c375faad9fa7731354d850e5bd5d6b5b9cfebf3b13952117665524812c7b06d4` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-S2-correction.diff](../../target/r2-daily-beta/wp-4-integration/wave26-S2-correction.diff) | 3156 | `d69c90f2740b3af68b7957480a896b9b1c8ae3c8d124ed6219681210c88963f2` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-25-S2-correction-inputs.json](../../target/r2-daily-beta/wp-4-integration/source-wave-25-S2-correction-inputs.json) | 152 | `e03273e83e23948583fc8026622e7c83c5b255fb2db380cdde2f4f5391f006bf` | MATCH / MATCH |
| evidenceArtifacts: [docs/reviews/2026-10-14-r2-snapshot-corrections-handoff.md](../../docs/reviews/2026-10-14-r2-snapshot-corrections-handoff.md) | 88 | `ef676ee3a41282b1fb15dfe673e4356125a923e3f2f7ec1718ecbf78b560ecb8` | MATCH / MATCH |

Read in full: root AGENTS.md, relevant ROADMAP R2-04/09, Wave26, accepted contract,
original496-line review, correction handoff, private78-line worker final and61-line
coordinator verification. All target source/test text and relevant supplied delta
were inspected. Bounded instruction-path checks found no additional CLAUDE/AGENTS
file under the reviewed docs/crates/market-data ancestors/subdirectories.
No Paseo/delegation workflow was invoked; the latest direct-worker instruction
governs this review despite the plan's coordinator routing label.

Readonly supporting source checks used the pinned storage ManifestEntry/FileEntry,
S1 EOF/count/parser/name behavior and pinned contract RequestMetadata/FetchMode/
SourceFileReference. The other frozen reference/evidence files were hashed;
no general historical/provider/parser architecture re-review or raw log inspection
was performed. Historical CI receipts were hashed for preservation, not executed
or reinterpreted as current coverage.

Cached dependency text only: serde_json1.0.151 SeqAccess/value error handling,
serde_derive1.0.229 duplicate-field ordering, rustix1.1.4 RawDir Linux export,
fs2-0.4.3 nonblocking flock implementation under
`/home/l1nnx/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.
Cargo.lock pins those versions. No dependency installation or network fallback.

**Actual checks and results**

| Check actually performed | Result / limit |
| --- | --- |
| Read-only Python hashlib/count comparison of inventory and all38 pins, start/final | MATCH; zero missing files or hash/count mismatches. Source/test have final LF. |
| Inert supplied baseline/diff text reconstruction and insertion/deletion counting | Both texts match exactly; +1796/-382 across only the two targets. No Git action. |
| Definition/name/source-line/platform inspection versus414-line test inventory |45 short names and source lines match;44 Linux/1 non-Linux;15 fully qualified private result paths disagree, E01. |
| rustfmt --edition 2024 --config skip_children=true --check on source.rs and snapshot test | Exit0. Syntax/formatting check only; not name/type/borrow resolution, compilation or lint acceptance. |
| Inert added-line whitespace check on pinned correction diff |0 trailing-whitespace additions; target final LF present. |
| Inert report local-link/line-bound and whitespace checks | PASS after final draft; no broken local link/line anchor or trailing whitespace. |
| R01–R16/R12a–h/P1–P10/T01–T32/C01 review coverage | All inspected; no unchecked required row. |

No Cargo, rustc, Clippy, test, build, parser/helper/fixture/checker/validator/
provisioner/self-test, standalone probe, SQL/DB/root/Docker/provider/production/
workflow/CI action was performed. No source function was executed. No Git actions,
dependency writes, agent/delegation calls, source fixes or other-document writes.
Only this assigned report is the review work product.

Old a888cd66c032c8931a4979434168568387c83a12/run38545976070 attempt1 SUCCESS
covers prior I0c partial source and51 Rust plus6 separate chart rows, excluding
both S2 files. Original immutable evidence and recovery supplement preserve their
nested-result limitations. No new S2 CI was supplied or dispatched. This review
does not infer a compiler pass, current job success or runtime acceptance.

**Unresolved, unverified and handoff**

Unresolved: S2-FR01–09 and S2-E01. Original R05/R10/R12 remain partial as mapped.
Coordinator owns bounded corrections, successor evidence inventory, finding
disposition, acceptance/commits and the later single exact-candidate hosted run
after checking existing runs. Source/test remedies fit the existing two-file
package. E01 requires coordinator-owned evidence bookkeeping; this reviewer has
no write authority for it. No contract/API/Cargo/platform change is required.

Not verified: current compilation, actual compiler diagnostics, strict all-target
Clippy, any of45 definitions, Linux lock/path/race/FD/cleanup behavior, non-Linux
build/no-I/O execution,32-bit size behavior, actual cross-principal UID/ACL/mount
access, installer live UID/root/caps or consumer scheduling/lifetime. These are
UNEXECUTED/unknown evidence, not observed runtime failures. Separate completed
invalid-second-value duplicate fixtures are not written; inspected pre-checks
reject them, and all8 incomplete-second-value cases are written. No universal
ABA or continuous-attestation claim is made.

Missing necessary review inputs: none. Conflicting requirements: none.
Conflicting supplied metadata: E01's15 private result paths. Other concrete
production defects found beyond FR08/FR09: none. Required rows left uninspected: none.
Execution-scope deviations: none; FR07/FR08/FR09 are explicitly identified existing
fixed-contract gaps, not represented as newly introduced production behavior.
No source repair or scope expansion was made.

Negative bootstrap/rollback,0060, provider/copy reconstruction, normalization/
admission, I6 structural diagrams before main, runtime/Owner and final C_R2 remain
accepted later-package boundaries, not findings or missing source-review inputs.
P10's Linux-only scope is binding and was not reopened.

Own fresh memory-hook context failed MCP_ERROR/no response; no history was used
and no failed call was represented as saved. The same supplied request's finish
returned SKIPPED/no_durable_change, saved=false, skipped=true, incomplete=false.
Only that authorized helper's private operational receipts are the memory-hook
exception; no Basic Memory, alternate scope/envelope or durable save was used.

Further decomposition required for this completed direct review: none. Background
review work/processes: none. Permission/background waits: none. Pending permissions:
none. Auto-review denials: none. All frozen inputs remain unchanged. Final report
line count and SHA256 are returned after the last edit, rather than embedded as a
self-referential commitment.
