# WP-4/I1b-S2-R — registered Raw snapshot source review

Completed independent source-only review, R2-04/09, 2026-10-14.
Actual startup:10:46:22.467UTC; reporting deadline:11:46:22.467UTC.
Baseline:`a888cd66c032c8931a4979434168568387c83a12`.
Frozen source handoff:`411326865706ea9e7a1690aa49742cd0254d8450`.

Corrections are required: four P1 finding groups identify library/test compile
blockers; twelve P2 groups identify strict-warning exposure, parser/lifetime/error
contract mismatches and defective or insufficient regression assertions.
All findings below are static reasoning. No fresh compiler, Clippy or test diagnostic
was executed or observed. Rustfmt success does not establish source acceptance.

The full three-file implementation and all required P1–P10/S2-T01–32 rows were
inspected directly. Unchecked required review rows:none. New tests remain UNEXECUTED:
34 definitions,5 private plus29 integration;33 expected on Linux and1 excluded
non-Linux definition. A name-to-row mapping is not proof that its assertions work.

## Prioritized findings

**P1 S2-R01 — parent cannot call the private Linux entry point.**

[Parent call, source.rs:202](../../crates/market-data/src/daily_price_only/source.rs#L202)
invokes `linux::snapshot`, but [child definition:914](../../crates/market-data/src/daily_price_only/source.rs#L914)
is an unqualified private `fn`. A child module's private item is accessible within
that child and its descendants, not its parent. Registering the module makes this
call part of the normal Linux library build. Impact: the new module blocks compilation
of market-data and consumers, before any snapshot regression can run.
Bounded correction: make this entry point `pub(super)`, like `validate_limit_layout`,
without adding a public API or widening the module. Gap: formatting cannot test privacy;
fresh exact-candidate compilation must include the registered library.

**P1 S2-R02 — annotate the pending vector before field access.**

[Declaration:1754](../../crates/market-data/src/daily_price_only/source.rs#L1754)
creates an unconstrained `Vec::new()`. The duplicate branch obtains its element at1818
and accesses `first.entry` at1821 before the later `push(PendingBatch { ... })` at1846.
Field resolution needs the element type at that point; later pushes/return constraints
do not supply that earlier field-resolution information. Impact: Linux parser type
checking fails. Bounded correction: `let mut pending: Vec<PendingBatch> = Vec::new();`.
Gap: duplicate tests are written but never compiled or run.

**P1 S2-R03 — the returned iterator must capture the self reference by value.**

[files():346–347](../../crates/market-data/src/daily_price_only/source.rs#L346)
returns `.map(|index| CommittedRawFile { batch: self, index })`. The non-move closure
borrows the local `self` reference binding, while the returned iterator outlives that
binding. The batch itself is borrowed for the caller's lifetime; borrowing the local
reference binding is different. Impact: the iterator-returning public getter fails
borrow checking. Bounded correction: `map(move |index| ...)`, moving the shared batch
reference into the closure, preserving the borrowed view and exact-size iterator.
Gap: all file-iteration tests depend on this getter, but formatting is not borrow checking.

**P1 S2-R04 — the integration target has three independent compile blockers.**

| Anchor | Concrete trigger/impact | Bounded correction |
| --- | --- | --- |
| [test:14](../../crates/market-data/tests/daily_price_only_snapshot.rs#L14) | Imports `hard_link` from `std::os::unix::fs`; that function belongs to `std::fs`. The Linux test module cannot resolve the import. | Import it from `std::fs` or call `fs::hard_link` at its four uses. |
| [test:1520](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1520), calls1547/1572 | `let error = error(...)` shadows the helper function. Later calls in the same test resolve to the `DailyEvidenceError` local, not a function. | Rename result locals, retaining the helper and every sentinel case. |
| [test:1523](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1523),1550/1575/1586 | `assert_eq!(error.source(), None)` requires `PartialEq` for `Option<&dyn Error>`; `dyn Error` has no such equality contract. | Assert `error.source().is_none()` at all four sites. |

These block compilation of the entire integration binary, not only T15/T29 execution.
Regression gap: no new target was compiled; retain all redaction and hardlink assertions
when fixing the types/names. The production fieldless error API itself does return None.

**P2 S2-R05 — the registered targets are not strict-warning clean.**

The following concrete source patterns are exposed to the required all-target
`-D warnings` gate; this is not a claim that a lint command ran.

| Anchor | Exposure | Bounded direction |
| --- | --- | --- |
| [source:350](../../crates/market-data/src/daily_price_only/source.rs#L350),385 | Both `pub(crate) entry` getters have no callers. Crate-only visibility does not exempt unused methods from `dead_code`. | Route appropriate existing getters/lookup code through them; retain the required crate-only signatures. Do not make them public just to suppress warnings. |
| [source:1201](../../crates/market-data/src/daily_price_only/source.rs#L1201),1251/1345 | Helpers take8,10,11 arguments, exceeding the default Clippy `too_many_arguments` threshold7. | Group existing private scan context/reference arguments; keep the fixed public API, ownership and caps. |
| S1204/1256/1347 | Read-only directory/batch parameters are `&mut Vec` without Vec mutation, exposing `ptr_arg`. | Use the appropriate borrowed slices/shared access alongside the private context fix. |
| [source:1753](../../crates/market-data/src/daily_price_only/source.rs#L1753); [test:247](../../crates/market-data/tests/daily_price_only_snapshot.rs#L247),251 | Nested parser-result and tree-state tuple types expose `type_complexity`. | Private aliases or a named test-only state representation, without changing payload/error visibility. |
| [source:585](../../crates/market-data/src/daily_price_only/source.rs#L585),594/596/597 | On x86_64 Linux, rustix Stat nlink/nanosecond fields are `c_ulong`/u64; `u64::from` is a same-type conversion, exposing `useless_conversion`. Other Linux widths differ. | Preserve checked/width-correct conversion with target-appropriate private handling; no narrowing or blanket allowance. |
| [test:412](../../crates/market-data/tests/daily_price_only_snapshot.rs#L412) | Getter already returns `&[u8]`; adding `&` before the from_bytes call exposes `needless_borrow`. | Pass the returned slice directly. |
| [test:620](../../crates/market-data/tests/daily_price_only_snapshot.rs#L620) | Initial `each = values()` is overwritten before any read, both in the loop and after it; `unused_assignments`. | Initialize at each actual case/use. |
| [test:1506](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1506) | Explicitly dropping a view with no Drop implementation exposes `drop_non_drop`. | Use a lexical borrow scope or let its last use end before dropping the snapshot. |

Non-Linux also leaves PendingBatch's fields used only by the excluded Linux machinery;
its unconditional size-of use does not read those fields. This is additional dead-code
exposure to handle without removing the portable public signatures.
No blanket lint allowance exists in the new source, and none is proposed. Further
compiler/lint diagnostics remain unverified after the P1 fixes.

**P2 S2-R06 — duplicate-field EOF classification differs from legacy Raw serde.**

[EntryVisitor:1980](../../crates/market-data/src/daily_price_only/source.rs#L1980)
passes `map.next_value()?` into `set_once`; the duplicate check only occurs inside
[set_once:2063](../../crates/market-data/src/daily_price_only/source.rs#L2063), after
that value has been deserialized. For an unterminated tail such as
`{"mode":"synthetic","mode":`, parsing the second value reaches serde EOF before
the duplicate is rejected. [Tail handling:1788](../../crates/market-data/src/daily_price_only/source.rs#L1788)
then discards the row and can return a successful empty/prefix snapshot.

The frozen ManifestEntry derives Deserialize. Cached serde_derive1.0.229
`de/struct_.rs:267` checks whether a field is already present before reading its next
value, so the same tail has a non-EOF duplicate-field error in the legacy/S1 path.
Impact: a known-invalid duplicate-field tail is treated as an ignorable torn tail,
violating P4 parity and fail-closed malformed-row handling. The special files arm
already checks duplicates before its seed; the other known fields need the same order.
Bounded correction: check slot presence before `next_value`, retaining typed bounds,
unknown-field handling and EOF-only tail behavior. Gap: T03/T30 do not compare this
case to S1. Add valid-control parity cases with an incomplete or invalid second value
for duplicated known fields; resource failures must still never become EOF.

**P2 S2-R07 — failure unwinding releases the lock before directory resources.**

[directories:924](../../crates/market-data/src/daily_price_only/source.rs#L924)
exists before [lock_file:1016](../../crates/market-data/src/daily_price_only/source.rs#L1016).
On `?`/early Err after shared acquisition, locals drop in reverse declaration order:
partial batches/manifest drop, then lock_file, then directories. Thus a cooperating
writer can reacquire the lock while failed construction still holds directory FDs.
This violates the explicit P5/P6 failure rule that the sole lock closes last.
Normal successful Drop is correctly ordered by ResourceGuard fields695–699 and
snapshot field order; the issue is the construction failure path.

Bounded correction: own partial construction resources in a private guard/state whose
lock is dropped last on every failure, then transfer that same ownership on success.
Do not add another lock descriptor or change the descriptor formula. Gap: T28 tests
post-return FD count/lock availability, which cannot observe relative close order;
add deterministic private cleanup-order/lock-held observation.

**P2 S2-R08 — commit.lock is incorrectly fingerprinted as evidence content.**

[Regular fingerprint:584](../../crates/market-data/src/daily_price_only/source.rs#L584)
always retains size, mtime and ctime; [lock open:1016](../../crates/market-data/src/daily_price_only/source.rs#L1016)
uses that fingerprint, and [lock recheck:1036](../../crates/market-data/src/daily_price_only/source.rs#L1036)
compares all fields. Trigger: the same regular, singly linked, trusted-UID lock inode
keeps its name/mode but its contents/length or timestamp changes during construction.
The call rejects SourceChanged, although P5 explicitly excludes lock content/length/time
from evidence and lists only identity/type/UID/mode/nlink as required lock properties.
Impact: an extra, unapproved lock-content stability condition can reject an otherwise
valid snapshot. Bound correction: use the frozen lock-specific identity policy; retain
strict full fingerprints for manifest/metadata/provider evidence. Gap: T23 covers lock
replacement/contention, not harmless same-inode lock-content/time variation.

**P2 S2-R09 — three written assertion cases would fail after compilation is fixed.**

| Anchor/case | Trigger and actual source result | Bounded correction and gap |
| --- | --- | --- |
| [T06:655](../../crates/market-data/tests/daily_price_only_snapshot.rs#L655) | Default max_source_file_bytes is8192; incrementing it yields8193, which is below the permitted67,108,864 maximum. ScanLimits::new returns Ok, but the test expects InvalidLimits. | Set the invalid case to67,108,865, and retain a valid maximum control. This currently does not test the D4 policy bound. |
| [T24 mutation helper:2375](../../crates/market-data/src/daily_price_only/source.rs#L2375), metadata case2609 | Renaming batch.json to batch.saved leaves an unexpected extra leaf in the same batch. After the metadata hook, membership at1468 rejects UnexpectedBatchEntry before final metadata binding checks can return SourceChanged; the test demands SourceChanged. | Keep the old inode outside the exact batch leaf set while replacing the metadata path. Establish the control and assert the intended replacement branch, rather than weakening the expected error. |
| [T28:2779](../../crates/market-data/src/daily_price_only/source.rs#L2779) | Expected snapshot lock contention produces WouldBlock. The test maps it through io_error at776, which only recognizes PermissionDenied and otherwise returns Io; it expects LockBusy. | Assert the typed WouldBlock kind, or use a contention-aware test mapper. Retain successful exclusive acquisition after Drop. |

These are test/oracle defects, not observed failures and not reasons to weaken the
production owner/type/membership/lock checks.

**P2 S2-R10 — process-global FD counts make T28 nondeterministic.**

[open_fd_count:2490](../../crates/market-data/src/daily_price_only/source.rs#L2490)
counts all `/proc/self/fd` entries. T28 compares samples at2746/2763 and2768/2783
without isolation from parallel Rust unit tests. Other S2 and existing library tests
can open/close unrelated fixture descriptors between these samples. Impact: normal
parallel hosted execution can fail despite correct cleanup, or unrelated closes can
mask a leak. A local lock used only by T28 would not protect it from other tests.
Bounded correction: make the observer identify this construction's owned resources
under a controlled lifetime rather than treating process-global equality as ownership
proof. Preserve actual FD/lock cleanup evidence; do not ignore the test or relax its
assertions. Gap: no parallel run or isolated owned-descriptor observation exists.

**P2 S2-R11 — T11's root-limit cases are masked by a prior depth fault.**

[T11:830](../../crates/market-data/tests/daily_price_only_snapshot.rs#L830)
installs a depth33 manifest and confirms SourceScanLimit. It never restores valid
manifest bytes before the root-byte and component cases at838/846. Both assert only
SourceScanLimit; an implementation that ignored those root caps would still reject
the invalid-depth manifest and pass the assertions. Impact: these purported root
boundary regressions do not distinguish the intended guards.
Bounded correction: restore the valid depth32 line and run the valid control before
each root-only cap fault, or use independent valid fixtures. Keep exact and one-under
caps and verify the relevant pre-traversal observation point.

**P2 S2-R12 — required pre-operation and late-race coverage is incomplete.**

The source contains many correct inspected guards, but the frozen P9 acceptance rows
require more than matching final error enums. Concrete omissions are:

| Existing anchor/row | Missing distinguishing assertion | Bounded follow-up |
| --- | --- | --- |
| [T07:669](../../crates/market-data/tests/daily_price_only_snapshot.rs#L669) | No read instrumentation verifies expected length plus at most one probe, or rejection before capture allocation/read. | Private bounded-read/capture observations, exact/over caps and growth/short-read cases. |
| [T08:696](../../crates/market-data/tests/daily_price_only_snapshot.rs#L696) | No observation distinguishes file-count rejection before the over-limit FileEntry is materialized/body opened from a post-parse length check. | A bounded-seed sentinel/malformed over-limit element plus operation observations; include a resource-failing unterminated candidate. |
| [T09:733](../../crates/market-data/tests/daily_price_only_snapshot.rs#L733), [T10:763](../../crates/market-data/tests/daily_price_only_snapshot.rs#L763) | Error/stats assertions do not demonstrate reserve-before-allocation/open, native retained/high-water ownership, or duplicate/Arc charging under the aggregate cap. | Controlled private resource observations, exact/under limits and duplicate-record aggregate accounting. Resolve R10 rather than using unisolated process totals. |
| [T13:901](../../crates/market-data/tests/daily_price_only_snapshot.rs#L901) | Linux relative-root, upper manifest/data-scope directory levels and batch.json symlink cases are absent; the non-Linux relative path is not Linux traversal evidence. | Add the required distinct no-follow path-level faults with valid controls. |
| [T16:1145](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1145) | No isolated batch.json FetchMode mismatch case, despite the explicit complete-metadata row. | Change only the parsed metadata mode, retain the committed entry, assert BatchMetadataMismatch. |
| [T17:1232](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1232), [private T24:2547](../../crates/market-data/src/daily_price_only/source.rs#L2547) | Extra entries are installed before construction; no explicit extra-leaf mutation tests the final membership pass. No binding mutation is tested at BeforeFinalNames (T28 only injects Io there). | Deterministic private late extra-entry and named-binding fault cases at the actual final observation points. |
| [T20:1369](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1369) | Old partition ordering is tested, but its request query carries only its own date, not the required later F_DT/T_DT overlap discovery fixture. | Supply that Raw request metadata and assert it remains available; do not validate provider interval truth here. |
| [T26:1477](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1477) | Barrier precedes append_manifest. Immediate try_recv only proves the worker has not sent completion, possibly because it has not attempted the lock. | Synchronize/assert a real separate-writer contention observation while the snapshot is held, then test publication/fresh capture after Drop. |

Impact: regressions that remove/reorder these guards could still pass the written
assertions. These are missing required written checks, separate from the fact that
all tests are unexecuted. Use private cfg(test) observation only, preserving the
production API/FD formula and no-sleep requirement. No ACL/provider/admission extension
is requested.

**P2 S2-R13 — nonrepresentable stat sizes use the wrong error on32-bit Linux.**

[checked_stat_size:1196](../../crates/market-data/src/daily_price_only/source.rs#L1196)
maps failed i64-to-usize conversion to ArithmeticOverflow. Rustix's32-bit Linux Stat
has an i64 size, so a regular file with st_size greater than usize::MAX reaches that
branch. P3 requires a nonrepresentable actual stat size to return SizeMismatch;
ArithmeticOverflow is for checked arithmetic/capacity/counter failures.
Impact: the portable Linux implementation violates its fixed typed error contract for
that size condition, although it still rejects the file. Bounded correction: map this
stat-size conversion failure to SizeMismatch, keeping checked conversion and caps.
Gap: T06 tests constructor arithmetic, not the actual stat-size conversion helper;
add a target-appropriate pure private case without allocating a giant buffer/file.
No32-bit execution was performed.

**P2 S2-R14 — reject lexical parent roots before allocating component copies.**

[root_components:874](../../crates/market-data/src/daily_price_only/source.rs#L874)
reserves the component vector, then clones each normal component at881–887. Only
when its traversal reaches ParentDir at889 does it reject the root. For a bounded
absolute path containing a later `..`, earlier components have already been copied.
The [fixed P1 ordering](2026-10-14-r2-raw-snapshot-implementation-contract.md#L179)
requires root validation before cloning any component. Impact: invalid roots cause
avoidable allocation before rejection; a reservation failure can precede the required
UnsafePath result. This is a validation-order defect, not a path-escape or unbounded
allocation claim. Bound correction: a nonallocating component-validation pass rejects
ParentDir/Prefix before reserving or cloning names. Gap: T13 checks the returned error,
but does not observe this required no-copy-before-validation boundary.

**P2 S2-R15 — validate recorded body-size constraints before opening batch trees.**

[parse_manifest file validation:1808](../../crates/market-data/src/daily_price_only/source.rs#L1808)
checks names/duplicates, but not recorded positive size or the configured body cap.
[capture_batch:1357](../../crates/market-data/src/daily_price_only/source.rs#L1357)
then opens data directories and captures/parses batch.json. The known recorded-size
fault is only rejected after the body pre-stat at1277–1288. A row with size0 or a
recorded size over max_source_file_bytes therefore performs batch-tree/metadata I/O
despite already containing disqualifying metadata.

[P4 step5](2026-10-14-r2-raw-snapshot-implementation-contract.md#L431) requires all
metadata-only constraints before opening those data paths. Impact: unnecessary I/O,
allocation and descriptor work on a known-invalid row; the whole scan still rejects,
so this is not truncated success. Bounded correction: validate each recorded size
in the retained-entry preflight, preserving SizeMismatch/SourceScanLimit and later
actual-stat/byte/hash comparisons. Gap: T07/T18 assert the eventual error but do not
assert that AfterMetadataCapture/data-tree opens are unreachable for this fault.

**P2 S2-R16 — valid fixtures depend on an unspecified creation umask.**

[Integration private_dir:152](../../crates/market-data/tests/daily_price_only_snapshot.rs#L152)
and [private-test private_dir:2300](../../crates/market-data/src/daily_price_only/source.rs#L2300)
use create_dir_all and set0700 only on the final path. Intermediate Raw-down directories
such as raw/manifests, its provider directory, and the data provider/market/date parents
keep creation permissions. Under a normal group-writable umask such as0002, those
directories are0775 and the reader correctly rejects WritableNode before the intended
test guard. Impact: supposed known-valid controls and lock/race cases can fail solely
from fixture setup, not a reader defect. This condition was not executed or observed.
Bounded correction: explicitly give every newly created Raw-down fixture directory
protected modes; preserve the deliberately writable outside data root in T32 and all
intentional mode-fault cases. No production permission repair or mode weakening.
Gap: establish the fixture's full protected subtree policy rather than assuming host
umask from an old CI run.

## Input identity and preservation

Inventory:`target/r2-daily-beta/wp-4-integration/source-wave-24-S2-review-inputs.json`,
133lines SHA256 `5d2829c071973ac12195c7e03cedb91963381a990757b535a89d7a68cab828dd`.
All29 pins matched SHA256 and newline counts at startup; final verification repeats
all29 plus the inventory itself. All target working bytes equal the frozen source commit.

| Target | Lines | SHA256 | Startup pin |
| --- | ---: | --- | --- |
| `crates/market-data/src/daily_price_only/source.rs` | 2804 | `41ae987cd9b50abebf44d1cb01f44311317d3c73f5ed62907afa43b86a2ba092` | MATCH |
| `crates/market-data/tests/daily_price_only_snapshot.rs` | 1665 | `c375faad9fa7731354d850e5bd5d6b5b9cfebf3b13952117665524812c7b06d4` | MATCH |
| `crates/market-data/src/daily_price_only/mod.rs` | 9 | `700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a` | MATCH |

source.rs and the integration target are new relative to a888cd6 and were reviewed
in full, including all five private tests. mod.rs changes only module prose and
`pub mod source;`; `mod contract; pub use contract::*;` remains. No root re-export or
dependency change. The combined scoped diff has4473 insertions/3deletions.
All19 readonly references are also byte-identical to the baseline, not merely to
the current pin inventory. Existing S1 reader/writer/orphan reconciliation, wire,
historical v2/v3, normalizer and StoreError callers therefore remain unchanged.

| Readonly reference | Lines | SHA256 | Startup pin |
| --- | ---: | --- | --- |
| `crates/market-data/src/storage.rs` | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` | MATCH |
| `crates/market-data/src/contract.rs` | 398 | `27b0a4d71f55ea2fea89eb54b6fe92483f859f2ae59c608885415a1eb680d3d4` | MATCH |
| `crates/market-data/src/normalize.rs` | 2019 | `be5b4d3444b355c40295ca33bdc9d40e3aa1e359146fcaca293b4a1f745cd6b9` | MATCH |
| `crates/market-data/src/historical_price_only.rs` | 1052 | `d30e7a95bb695c180e47eedb6b11209dc7fceb598a65cc4b60a16456a48a4b52` | MATCH |
| `crates/market-data/src/historical_price_only_artifact.rs` | 3924 | `60298769aa267a4cd32d7e0c3343b76f65fddd79a0b449bd1881d53408fb6065` | MATCH |
| `crates/market-data/src/daily_price_only/contract.rs` | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` | MATCH |
| `crates/market-data/src/lib.rs` | 288 | `fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75` | MATCH |
| `crates/market-data/Cargo.toml` | 36 | `d2af9a324bf1e421c23f4c15b5768882f8b041502e351f7e1f1ac671c95a1997` | MATCH |
| `crates/market-data/tests/daily_price_only_storage.rs` | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` | MATCH |
| `docs/contracts/owner-beta-daily-etf11-input-v1.md` | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` | MATCH |
| `docs/reviews/2026-10-14-r2-daily-input-design.md` | 926 | `e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670` | MATCH |
| `docs/reviews/2026-10-14-r2-daily-input-design-amendments.md` | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` | MATCH |
| `docs/reviews/2026-10-14-r2-raw-manifest-evidence-review.md` | 290 | `e0322d1774dbd0124d1619f7f69be983dc43a035df8d71ac8ec324d5e43c089b` | MATCH |
| `docs/reviews/2026-10-14-r2-raw-manifest-corrections-review.md` | 199 | `f65bb3e3a47f8cea39dfccd74656602639340249f56ddbc76881fc846eb5e88b` | MATCH |
| `docs/reviews/2026-10-14-r2-caller-role-raw-ci.md` | 26 | `989a3891493f3ece231de15050f6bb3d139f1034edf3c0fe87e4aef4b686a47c` | MATCH |
| `Cargo.lock` | 5609 | `d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e` | MATCH |
| `docs/reviews/2026-10-14-r2-raw-snapshot-implementation-contract.md` | 898 | `c6f15291ba24eef23421f3e80ead194bb4e5adac6a5733271eb0aeb05cbe6056` | MATCH |
| `crates/market-data/src/historical_price_only_v3_artifact.rs` | 2487 | `9a604f95e9d5df92f1a54c3b3f23b8221513c19c1fc64f135c826947c6b02244` | MATCH |
| `crates/market-data/tests/daily_price_only.rs` | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` | MATCH |

The full898-line accepted contract was read, including section12. P1–P9 are accepted
and P10's Linux-only narrowing is binding; historical proposal wording is not an open
approval gate. D4/D5 remain950/740lines at their exact pinned hashes.

| Evidence artifact | Lines | SHA256 | Startup pin |
| --- | ---: | --- | --- |
| `docs/reviews/2026-10-14-r2-snapshot-source-handoff.md` | 50 | `78b54aa391bcee322b3ac057db44b21c5bbbe9b1a9f9410565be863e398a90fd` | MATCH |
| `docs/reviews/2026-10-14-r2-admitter-helper-ci-recovery.md` | 77 | `1cceaae4932ee3671538fb77e7665ce7de2e0bc1e7c19cb7b81251df466ee156` | MATCH |
| `docs/reviews/2026-10-14-r2-admitter-helper-ci.md` | 118 | `2587d1804ee035d7e30a618bd79ed7802b48a73596b2aa7767a4b3a682df7200` | MATCH |
| `target/r2-daily-beta/ci/a888cd6/ci-evidence.json` | 1455 | `77241980b864ac78fcfb1168b2ae49a34f9c1c5ec99dfd7a00aab4474c41d5fb` | MATCH |
| `target/r2-daily-beta/wp-4-integration/wave22-I0c-CI-coordinator-verification.json` | 444 | `63c31c8ccf7c38e505f6052e48ce4381544a3d4260a25efa52a5096f70858eed` | MATCH |
| `target/r2-daily-beta/wp-4-integration/wave23-I1b-S2-final.md` | 64 | `699a37b5a1e8e4ff4d55feba04e13bb53d694a1ec6583b9b162171b745993b6c` | MATCH |
| `target/r2-daily-beta/wp-4-integration/source-wave-23-S2-test-inventory.json` | 78 | `380b2c761202baa8291d22a1c84213f06fe2637bf77e7ea9aab906e45723aede` | MATCH |

Read the source handoff, full64-line private final and complete test inventory.
Their UNREGISTERED wording describes the recovered pre-registration stage.
The Wave24 inventory and actual mod.rs now establish REGISTERED_BUT_UNCOMPILED_UNEXECUTED;
that historical wording is not a current input contradiction.
The [CI recovery supplement](2026-10-14-r2-admitter-helper-ci-recovery.md#L16)
records old a888cd6/run38545976070 attempt1 success and the previous collector's
routing/extraction deviations. Its51Linux Rust,6separate chart, strict Clippy/required,
PG59/0039/no-SKIP/cleanup evidence contains neither new snapshot file. Old success
cannot validate this delta. Private CI receipts were hashed only; no raw log was read.
No hosted run query/dispatch or provider operation was performed.

## P1–P10 disposition matrix

“Inspected” below denotes source reasoning only, never acceptance or execution.

| Contract | Disposition | Concrete evidence / qualification |
| --- | --- | --- |
| P1 API/ownership | Present but blocked | Opaque private scope/limits/reader/snapshot/batch/file shapes and fixed getter signatures at39–400; no public FD/path/mutable extractor or data-bearing Debug/Clone/Serialize/Default. R01–R05 prevent compile/strict-warning readiness. |
| P2 scope/completeness | Inspected match | Scope45–74: exact kis/kis-calendar, kr, supplied UID !=MAX, zero has no bypass. Complete parse1754 onward, no date filter; lookup260 matches all SourceFileReference fields, safe name first, no copied_from/filesystem fallback. |
| P3 bounds/resources | Inspected with defect/gaps | Positive twelve caps99, checked layouts147, reserves709/743, root852, byte bounds1215/1283/1712; full retained-byte and FD formulas1077/1156. R13 typed stat-size discrepancy; R09/R11/R12 limit tests incomplete/broken. No host-RSS/OOM or wall-clock guarantee claimed. |
| P4 Raw parsing/equality | Defect | Exact manifest Arc/ranges/hash1754–1873, pre-dedup record counts, resource side channel, bounded files seed2102, unknown fields/optional legacy fields and full metadata equality1427 are present. R06 duplicate-field EOF parity and R15 recorded-size preflight must be corrected. |
| P5 capture/path/identity | Inspected with defects/gaps | Descriptor-relative no-follow opens815/939/1213/1267; readonly NONBLOCK leaves; full bytes/stat/path checks1122–1154 and1638. Directory fingerprints omit size/link/time; Raw-down UID/mode versus outside ancestry preserved. R07 failure order, R08 lock fingerprint and R14 lexical-root allocation order differ from contract; P9 gaps remain. |
| P6 lock/lifetime | Partial | One try_lock_shared1027; immediate locked path revalidation1035; successful snapshot field order223 and ResourceGuard695 keeps lock through Drop. Failure order violates lock-last (R07). T22/T23/T26/T28 are unexecuted and qualified below. |
| P7 errors/redaction | API inspected, one discrepancy |29 fieldless variants403, fixed code/Display436/472, source None478; no StoreError growth, sensitive payload or log. R13 size-error mapping; T29 compile blockers R04 do not change the safe production shape. |
| P8 registration/preservation | Match | Two new files plus sequential mod.rs prose/source registration only;19 references exact baseline bytes. No workflow/dependency/normalizer/provider/historical source edit. Diagram/main integration remains coordinator-owned later scope. |
| P9 required regressions | Not complete |34 definitions exist; all32 row mappings inspected, but R04/R05/R09–R12 block compilation, assertions or required distinguishing coverage; R16 affects valid controls. No test executed. |
| P10 platform | Inspected cfg match, unexecuted | Entire private platform machinery under target_os=linux at509; non-Linux branch204 returns UnsupportedPlatform before I/O and keeps public signatures. T31 is non-Linux-only. Portable strict-warning exposure remains R05; no platform execution or ACL proof. |

## S2-T01–32 guard-to-written-test dispositions

S = `crates/market-data/src/daily_price_only/source.rs`;
I = `crates/market-data/tests/daily_price_only_snapshot.rs`.
All listed tests are written and UNEXECUTED; “match” is static guard/assertion inspection.
All Linux rows share the P1 compile/P2 warning blockers and R16 fixture qualification.

| Guard ID | Source guard | Written test anchor | Disposition / exact remaining qualification |
| --- | --- | --- | --- |
| S2-T01 | S223/914/1156 | [I369](../../crates/market-data/tests/daily_price_only_snapshot.rs#L369) | Multi-batch order, captured bytes/hash/size and aggregate/FD stats asserted. Native descriptor observation remains R12; from_bytes borrow warning R05. |
| S2-T02 | S1754/1846 | [I447](../../crates/market-data/tests/daily_price_only_snapshot.rs#L447) | CRLF/whitespace/reordered keys and physical lines/full hash asserted. S1 comparison covers the initial common valid fixture; no execution/parity result exists. |
| S2-T03 | S1773/1785/1793 | [I506](../../crates/market-data/tests/daily_price_only_snapshot.rs#L506) | Complete EOF accepted/count1, torn tail byte-retained, terminated/non-EOF malformed rows rejected. Duplicate-field EOF parity missing R06; resource-tail check gap R12. |
| S2-T04 | S1816/1821/1846 | [I538](../../crates/market-data/tests/daily_price_only_snapshot.rs#L538) | Equal whitespace duplicate keeps first line/order/count2; conflicting entitlement rejected. Inference blocker R02; duplicate aggregate charge not asserted (R12). |
| S2-T05 | S1052/1773/1793 | [I570](../../crates/market-data/tests/daily_price_only_snapshot.rs#L570) | Exact byte/record count and one-under byte/over-record duplicate, torn-tail byte budget asserted; complete unterminated record included. |
| S2-T06 | S99/147/176 | [I619](../../crates/market-data/tests/daily_price_only_snapshot.rs#L619) | Twelve zero fields, large byte/count products and depth33 cases written. Body-policy assertion wrong R09, initial assignment warning R05; stat conversion case missing R13. |
| S2-T07 | S1215/1283/1712 | [I670](../../crates/market-data/tests/daily_price_only_snapshot.rs#L670) | Metadata/body exact and one-under caps asserted; operation/read-length/probe observations absent R12. Torn-tail bytes tested in T05. |
| S2-T08 | S1830/1997/2102 | [I697](../../crates/market-data/tests/daily_price_only_snapshot.rs#L697) | Per-batch, total-file, batch caps and many small metadata fields tested through public errors. Seed-before-element/open proof missing R12. |
| S2-T09 | S743/1059/1291/1414 | [I734](../../crates/market-data/tests/daily_price_only_snapshot.rs#L734) | Exact aggregate and one-under failure written; pre-allocation/duplicate charging observations missing R12. |
| S2-T10 | S709/922/1077 | [I764](../../crates/market-data/tests/daily_price_only_snapshot.rs#L764) | Formula exact/under, shared dates and deep root; counters match formula by inspection. Actual open/high-water/forbidden-open distinction missing R12. |
| S2-T11 | S852/1876/2168 | [I818](../../crates/market-data/tests/daily_price_only_snapshot.rs#L818) | Unknown-depth32/33 and root byte/component caps written; latter cases masked by stale invalid manifest R11. |
| S2-T12 | S984/1016/1044 | [I856](../../crates/market-data/tests/daily_price_only_snapshot.rs#L856) | Missing Raw/control/lock/manifest rejects without repair; empty existing manifest and C+7 descriptors asserted. |
| S2-T13 | S815/818/1223 | [I902](../../crates/market-data/tests/daily_price_only_snapshot.rs#L902) | Ancestor/Raw/market-control/date/batch/body/manifest/lock symlink and parent/ordinary-file cases; distinct missing levels/relative/metadata leaf in R12; pre-validation allocation R14. |
| S2-T14 | S552/605/621 | [I1032](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1032), [S2505](../../crates/market-data/src/daily_price_only/source.rs#L2505) | FIFO/directory, forbidden file/batch modes, wrong supplied UID; synthetic uid0 and link policy. No root execution or actual cross-UID permission evidence. |
| S2-T15 | S585/621 | [I1098](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1098) | Hardlinked body/manifest/lock/metadata cases written; invalid import blocks target R04. |
| S2-T16 | S1427/1801 | [I1146](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1146) | Wrong manifest scope and metadata date/ID/request/time/size/order assert full mismatch. Full Eq includes all fields by inspection; explicit mode case missing R12. |
| S2-T17 | S484/1808/1509/1133 | [I1233](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1233) | Identical duplicate/reserved/traversal/initial extra temp/subdirectory rejected. Final-pass extra-entry fixture absent R12. |
| S2-T18 | S1278/1288/1331/2164 | [I1291](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1291) | Missing body/metadata, zero recorded size, truncation, corruption and torn metadata; exact typed errors with valid controls. No batch skipped; recorded-size preflight late R15. |
| S2-T19 | S1106 | [I1342](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1342) | Committed-only traversal; complete/incomplete orphans remain untouched; content/mode/UID/inode/link tree comparison excludes atime. |
| S2-T20 | S1106/1376/350/385 | [I1370](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1370) | All old/new partition batches in first order; no source filter. Later-query overlap fixture absent R12; crate getter warning R05. Provider interval verification excluded. |
| S2-T21 | S260 | [I1389](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1389) | Exact hit, missing batch/file, wrong provider/hash/retrieval time; absent copied-source name rejected without fallback. Other field checks directly visible in source. |
| S2-T22 | S223/695/1027 | [I1432](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1432) | Two simultaneous readers plus separately opened writer; writer rejects until both Drop, then succeeds. Actual lock result unexecuted. |
| S2-T23 | S1027/1035/1036 | [I1451](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1451), [S2528](../../crates/market-data/src/daily_price_only/source.rs#L2528) | Busy writer and lock replacement before/after acquisition have deterministic controls. Lock volatile-field mismatch R08; no retry/open replacement. |
| S2-T24 | S1122/1141/1154/1565 | [S2548](../../crates/market-data/src/daily_price_only/source.rs#L2548) | Raw/control/manifest/batch/metadata/body replacement hooks written. Metadata fixture hits extra-entry instead (R09); BeforeFinalNames mutation coverage absent R12. |
| S2-T25 | S538/792/1638 | [S2641](../../crates/market-data/src/daily_price_only/source.rs#L2641) | In-place manifest/metadata/body, readonly-mode change, link change and synthetic UID check written. No actual chown/ABA/continuous-attestation claim. |
| S2-T26 | S695/1027 | [I1466](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1466) | Separate writer appends then fresh snapshot sees second batch after Drop; pre-Drop actual attempt is not synchronized (R12). No sleep used. |
| S2-T27 | S381/1317 | [I1493](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1493) | Successful returned bytes remain captured after disk corruption; fresh scan HashMismatch. drop_non_drop warning R05; no continuous disk-validity promise. |
| S2-T28 | S924/1016/695 | [S2729](../../crates/market-data/src/daily_price_only/source.rs#L2729) | Seven phase failures plus normal Drop: tree comparison, FD samples and writer acquisition. Failure-order R07, wrong busy assertion R09, concurrent FD observer R10. |
| S2-T29 | S403/436/472/478 | [I1512](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1512) | Mode/hash/UUID/path/request/copy/entitlement/OS sentinels written, finite production diagnostics visible; helper-shadow/dyn-error equality block compilation R04. |
| S2-T30 | S1999/2012/2109/1330 | [I1590](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1590) | Unknown Raw field and omitted optional continuation/copy, correct-hash invalid provider bytes accepted as opaque. Dup-known-field classification still R06; not provider verification/admission. |
| S2-T31 | S204/509 | [I1637](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1637) | Non-Linux-only definition, UnsupportedPlatform on relative nonexistent root; public API retained and source branch contains no I/O. Excluded on Linux, never executed elsewhere. |
| S2-T32 | S55/571/939/969 | [I1621](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1621), I1085 | Explicit real fixture UID, outside root mode777 accepted; wrong supplied UID in T14. No actual different OS principal/ACL/mount or denied-access proof. |

## Resource, trust and evidence conclusions

Before opening `/`, source validates bounded root bytes/components and preflights C+7.
All directories use RDONLY/DIRECTORY/NOFOLLOW/CLOEXEC; leaves add NONBLOCK and are
pre-stat regular/type/owner/link checked before reading. Boundings compare pre-stat,
fstat and named stat. No euid inference, UID-zero bypass, canonicalize, permission
fallback, lock creation, mutation, repair, reconciliation or provider body parser is
present in production source. Outside ancestors retain identity/UID/mode without
being assigned Raw's UID; Raw-down nodes require the explicit UID and no022.

One complete manifest Vec is moved into Arc without duplicating evidence bytes;
line ranges exclude only LF. Per-entry metadata is parsed with byte/depth/file caps,
full Eq and original FileEntry order. Distinct batches/files and aggregate bytes are
charged before retained entries/body allocations, with checked arithmetic/fallible Vec
reservations. Duplicate temporary metadata is not retained. serde/domain allocations
and allocator slack remain the contract's bounded-but-not-fully-fallible overhead.
Directory names come from finite root/scoped/date/batch/basename values; RawDir borrows
the existing FD with4KiB buffer and rewind, without dup. Body reads/final comparisons
use8KiB scratch plus a one-byte probe, not read_to_end or another body buffer.

For B>0, code computes `C+9+D+2*B+F`; for B=0, `C+7`. Shared-date directories are reused
by parent/name, and empty snapshots do not open data-scope directories. Final stats
are accumulated actual captured lengths and logical descriptor counts, not configured
maxima. Physical high-water evidence is not established by those counters alone (R12).
Failed opens release their reservation; later post-open failure exits the entire scan
and owned handles unwind. No partial snapshot is returned. Failure close order remains R07.

Final descriptor-byte comparisons for manifest/metadata/bodies are bracketed by named
and held stat checks. Membership and every retained directory/leaf/lock binding are
revalidated before return. Directory fingerprints intentionally exclude child-sensitive
size/nlink/mtime/ctime; regular evidence includes subsecond mtime/ctime and excludes
atime. No copied-source reconstruction or normalization is performed. Captured bytes
stay immutable after return; retained readonly FDs do not freeze writable inodes.
Sequential rechecks cannot prove immunity to ABA, mutation/restoration, hostile mounts/
filesystem or an uncooperative trusted writer. Shared locking is advisory, not a
continuous filesystem attestation. Those accepted limitations are not new defects.

## Checks actually performed and limits

Read root AGENTS.md, current ROADMAP R2-04/09 and Wave24, including actual startup/
deadline. No applicable additional instruction file was found under the reviewed
market-data/docs paths. The task's report-only scope leaves roadmap/plan/diagram and
other-worker files unchanged; no delegation workflow or agent tool was invoked.

Performed SHA256/newline checks of inventory and all29 pins; committed three-target
byte equality; scoped baseline diff and19-reference byte comparisons; all34 written
name counts against the private inventory/cfg locations; full898-line contract and
complete source/test reads; bounded symbol/path/type inspection. Readonly adjacent
type checks were storage ManifestEntry/FileEntry/S1 parser/name rules, contract
FetchMode/RequestMetadata/SourceFileReference, and domain IDs/time/hash, not a general
re-review of older architecture.

Supplemental domain reads also match baseline bytes:
`ids.rs`274lines `7c54ddfe3416f10dcc8712810aa4e19d8c2a020561be6641e9adecdc3c1a8056`;
`time.rs`412lines `6bfd42349ee7e7c4e92298095f63fec9d2a1244e0620dc1f80e2669211b0b545`;
`hash.rs`177lines `7d5b49d42d79e86caf4bd05618f3f6871a2fd3f4a5383111f8f32960600eefd1`.
They confirm Copy/Ord BatchId, exact typed timestamp equality and from_bytes(&[u8]).

Cached dependencies under
`/home/l1nnx/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` were inspected:
rustix1.1.4 fs/mod.rs, fs/raw_dir.rs and backend/linux_raw/fs/types.rs;
fs2-0.4.3 lib.rs/unix.rs; serde_derive1.0.229 de/struct_.rs.
Cargo.lock pins these versions. RawDir's borrowed-FD/buffer API and linux_kernel cfg,
Linux Stat field widths and fs2's nonblocking flock/contended errno are visible there.
An initial lookup of serde_derive1.0.228 failed; the pinned1.0.229 source was then read.
Optional standard-library source/docs were absent from the searched normal rustup
cache paths; standard-library compile conclusions above are static API/type reasoning,
not compiled diagnostics. No dependency installation or fallback network lookup occurred.

`rustfmt --edition 2024 --check --config skip_children=true` on all three targets:
exit0. This parses/formats syntax, not names/privacy/inference/borrows/lint or behavior.
Scoped committed-source `git diff --check`:exit0. Report `git diff --no-index --check`
against /dev/null:exit1 with no whitespace diagnostics (new-file difference); independent
inert report whitespace validation:PASS. Final inventory/all29 pins, committed target
bytes and19 baseline references:MATCH. P1–P10/T01–32 disposition and written-name/order
inventory checks:PASS. Report local links/line bounds:PASS; final count/hash are supplied
in the handoff after the last report edit.
No Cargo/rustc/Clippy/build/test, extracted helper/parser, checker/validator/provisioner/
self-test, SQL/DB/root/Docker, provider/network/production/workflow action or Git write
was performed. The only network-capable helper was the prescribed current memory hook.
No raw CI log or credential/provider payload was persisted.

## Unresolved, unknown and handoff

Unresolved source/test work:S2-R01–R16. Coordinator owns bounded correction,
independent disposition, acceptance, commits and any later single exact-candidate
hosted verification after existing-run inspection. No fix was made by this reviewer.

Not verified:new compilation or diagnostics, strict all-target lint, any of the34
new test definitions, actual Linux lock/path/resource/race/cleanup behavior, non-Linux
build/no-I/O execution,32-bit size behavior, cross-principal permissions/ACL/mount
arrangements, real installer UID/root/caps or consumer lifetime policy. These unknowns
are not presented as runtime failures. Prior successful CI is baseline only.

Missing required frozen inputs:none. Conflicting requirements:none. Required guard
rows left uninspected:none. Other concrete findings:none beyond the16 groups above.
Scope deviations:none; supplementary type reads and own-hook operational receipts
are within the authorized exceptions. The current context hook failed MCP_ERROR with
no response/history recovered; no failed call was treated as saved/skipped. The same
fresh request's finish returned SKIPPED/no_durable_change, saved=false, incomplete=false.
No memory content or alternate/stale envelope/Basic Memory was used.

Negative bootstrap/rollback,0060, provider/copy reconstruction, normalization/admission,
diagrams before structural main integration, runtime/Owner and final C_R2 remain
accepted later boundaries, not missing source-review requirements. P10 is not reopened.

Further decomposition needed to complete this review:none; completed directly within
its60minute bound. Background review work/processes:none. Pending permissions/waits:none.
Auto-review denials:none. Only this assigned report is the writable work product;
all frozen targets/references/evidence bytes are preserved.
