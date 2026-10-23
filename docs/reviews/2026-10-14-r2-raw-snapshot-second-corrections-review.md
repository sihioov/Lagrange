# WP-4/I1b-S2-F2-R — second Raw snapshot correction review

Completed independent source-only review, R2-04/09, 2026-10-14 UTC.
Actual startup:15:05:18.207 UTC; reporting deadline:16:05:18.207 UTC.
Frozen handoff:`bbac2e951967e0ca3a7633171e5e09e2374bd696`.
Comparison baseline:`7545094404628974c1bd626c84f6c3f704555f5f`.
Original source:`411326865706ea9e7a1690aa49742cd0254d8450`.

**No concrete P1/P2 finding remains in this reviewed correction.** FR01–FR09,
E01, the original R01–R16/R12a–h and C01 are resolved at source-review level
for their identified conditions. No newly introduced concrete source/test defect
was found. This conclusion does not establish compilation, strict lint, executable
test results, platform/ACL behavior, source acceptance or runtime acceptance.
The coordinator retains acceptance and any later executable verification.

The full current source, integration target, all49 definitions, full898-line
accepted contract, both complete prior reviews and complete supplied delta were
inspected directly. Required FR/R/R12/C01/P/T/definition rows left uninspected:none.
All49 definitions are **WRITTEN_UNEXECUTED**:19 private Linux,29 integration Linux,
one non-Linux. There are48 expected Linux names, not49 executed tests.

S denotes `crates/market-data/src/daily_price_only/source.rs`; I denotes
`crates/market-data/tests/daily_price_only_snapshot.rs`. Numeric S/I anchors below
refer to these exact current files. The definition table supplies clickable names
and source bounds for every written test.

**Frozen inputs and comparison**

| Target | Lines | SHA256 |
| --- | ---: | --- |
| [source.rs](../../crates/market-data/src/daily_price_only/source.rs) | 4413 | `0a6c29c7fff0bdbc48c5af0f1950c8522542270e43d374b9ee8ae6243b2729d3` |
| [daily_price_only_snapshot.rs](../../crates/market-data/tests/daily_price_only_snapshot.rs) | 1970 | `a5db9e0adc4ca909835be3a717c5579a8c1b356f7c1b7036f4dc6cae8cbb7f61` |
| [mod.rs](../../crates/market-data/src/daily_price_only/mod.rs) | 9 | `700469d6a510aa3ae7ad6a27fb42a78a76b243256e5c31cb5292104b4a6be74a` |

The343-line [Wave28 input inventory](../../target/r2-daily-beta/wp-4-integration/source-wave-28-S2-review-inputs.json)
has SHA256 `5506ed71915fde4631c0af6eb9029d673dbccf60d573f5a0dfcebd2949d5c744`.
Its sourceCommit/sourceBaseline match the revisions above. All56 pins matched at
startup and final:3 targets,22 references,31 artifacts; zero missing/mismatched
bytes or line counts. The launch supplement's56 pins supersede earlier historical
46-pin coordinator checks; those counts describe different inventories.

The474-line [candidate test inventory](../../target/r2-daily-beta/wp-4-integration/source-wave-28-S2-test-inventory.json)
has SHA256 `91c08ebbd5f9969470f90c0614462de77b6b496b4133e9701f87d24ec9f03bf6`.
All49 actual short names, function lines, qualified names and platform flags match.
All45 baseline names remain, with precisely four additions:S3249/S3496/S3549/S4123.
No definition is ignored. Historical34/45-definition inventories remain preserved.

Supplied baseline copies match their pinned3987/1896-line hashes. Plain `diff -u`
reports source+694/-268 and integration+159/-85, combined+853/-353. The supplied
1749-line delta has SHA256 `ec132c244bf22e22bcd06f6a749014a33d7305b8760506a768a4f45b92c2a19c`;
its alternate hunk alignment gives+851/-351. All59 supplied hunks match both
baseline/current text slices, counts and positions exactly. Both alignments yield
the same426/74 net lines and final bytes. No Git command/object query was used.

Eight regions were independently compared byte-for-byte against the supplied
baseline:API/getters S39–391; errors/name policy S406–511; fingerprints/name pool
S584–763; construction/final-lock layout S796–846; root/preflight S1019–1098;
size/preflight S1427–1455; complete Raw parser S2100–2572; C01 S3964–3996.
All match. These are text-preservation checks, not compiler or behavior evidence.

**FR01–FR09/E01 dispositions**

Resolved below means the particular prior source/test condition is corrected by
the inspected bytes. It does not certify an executable pass or all future lints.
Prior severities are retained for traceability; there are no new severity IDs.

| Prior finding | Disposition | Current evidence, trigger distinction and remaining verification limit |
| --- | --- | --- |
| FR01 P1 | Resolved | [S3414](../../crates/market-data/src/daily_price_only/source.rs#L3414) retains copied ScanLimitValues and constructs a fresh ScanLimits for each control/fault scan. No consumed capability is reused; Copy/Clone were not added to ScanLimits. Exact one-file control and all materialization/open assertions remain. Compilation is unexecuted. |
| FR02 P1 | Resolved | [S3618](../../crates/market-data/src/daily_price_only/source.rs#L3618) maps opaque BoundedParseError to fieldless InvalidManifest before unwrap. The legacy sequence Eq assertion remains; no parser/payload Debug implementation was introduced. |
| FR03 P2 | Resolved | [S3777](../../crates/market-data/src/daily_price_only/source.rs#L3777) uses filter_map(...).next_back() and still checks the final retained total against actual bytes minus the rejected body. No last() or lint allowance remains in this path. Strict Clippy is unexecuted. |
| FR04 P2 | Resolved | [S541](../../crates/market-data/src/daily_price_only/source.rs#L541) owns one real File; S576 drops it before recording unique-lifetime closure. All five native opens wrap before fallible post-open checks. S2598–2707 retains the Rc observation destination across moves/TLS exit. S3249 distinguishes independent owners of the same inode without assuming FD allocation order. Native live checks, fixture-unique post-close checks and S4324 cleanup-time WouldBlock complement closure events. Shared ancestors are never queried by old FD slot after close. Native execution remains unknown. |
| FR05 P2 | Resolved | [S2950](../../crates/market-data/src/daily_price_only/source.rs#L2950) ends the genuine tail at the colon; S2962 separately restores the old malformed suffix. S3414 tests both permissive legacy and bounded Json.is_eof classifications, then cap1 -> SourceScanLimit with exactly one materialized FileEntry and no tree/metadata/provider open. Independent S3496 total-file and S3549 unique-batch controls succeed at2 and reject at1 before tree/leaf opens. No Raw/fixture parser was executed in this review. |
| FR06 P2 | Resolved | [I1686](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1686) bounds attempt, continuation and completion receives at5seconds. It requires an actual separately opened pre-Drop WouldBlock observation, then a fresh nonblocking exclusive acquisition after Drop before entering append_manifest. That probe unlocks/drops before append. Error/timeout closes channels; a worker without completion is not joined. Completion is reported after inner native resources unwind. Fresh capture checks the appended ID. OS stalls are not a promised hard deadline. |
| FR07 P2 | Resolved | [I398](../../crates/market-data/tests/daily_price_only_snapshot.rs#L398) checks both batches and all three file views in order, exact iterator lengths/exhaustion, fixture-derived name/kind/bytes/hash/size, pointer association and every line/metadata commitment. The fixture explicitly distinguishes all body bytes, hashes and lengths. Assertions no longer self-compare a returned hash only against returned bytes. |
| FR08 P2 | Resolved | [S1852](../../crates/market-data/src/daily_price_only/source.rs#L1852) accepts a phase-specific missing error. Initial membership atS1741 uses MissingCommittedEvidence; post-capture/final atS1810/S1354 use SourceChanged. Extras remain UnexpectedBatchEntry. S4123 establishes valid control, moves body and metadata separately outside the batch at BeforeFinalNames, checks the exact remaining name and invoked hook, and requires SourceChanged. It reaches final membership before later path checks. |
| FR09 P2 | Resolved | [S1770](../../crates/market-data/src/daily_price_only/source.rs#L1770) scopes the full parsed metadata copy/Eq throughS1775. Owned temporary fields drop before S1779 metadata-leaf construction and S1798 body vector/capture. Full Eq and error mapping remain. This is lexical lifetime reasoning, not measured RSS/allocator evidence. |
| E01 P2 | Resolved in successor evidence | [Candidate inventory](../../target/r2-daily-beta/wp-4-integration/source-wave-28-S2-test-inventory.json) uses daily_price_only::source::linux::private_tests:: for all19 private rows. All49 names/lines/gates match source. Frozen incorrect414-line and corrected423-line baseline artifacts remain historical. Inventory correction is not test execution proof. |

**Original R01–R16 and C01 dispositions**

| Original finding | Disposition | Concrete current source/test condition |
| --- | --- | --- |
| R01 | Resolved | Parent S204 calls pub(super) linux::snapshot atS1099, without public platform machinery. |
| R02 | Resolved | S2112 declares Vec<PendingBatch> before duplicate first.entry field access. Full parser is unchanged from the corrected baseline. |
| R03 | Resolved | S348 uses map(move ...), retaining shared self and actual index. I398 now distinguishes every aligned returned file view. |
| R04 | Resolved | I16 imports std::fs::hard_link; I1797 redaction cases use evidence_error rather than shadowing the helper, and source().is_none() rather than dyn Error equality. New touched private tests do not recreate the old unwrap/ScanLimits blockers. |
| R05 | Resolved for identified patterns | Required crate-only getters have callers S284/S313/S368; named ScanContext/ParsedManifest/TreeState, slice parameters, generic checked_u64, local per-case limits, direct slices, NLL view use and Linux-only PendingBatch preserve earlier corrections. FR03 is corrected. No concrete new strict-warning blocker was identified; all-targets/all-features -D warnings remains UNEXECUTED. |
| R06 | Resolved | S2329 checks each known field's presence before parsing its second value, including Option<Option<String>> for first-null entitlement. I592 retains all8 incomplete-second-value legacy/S2 rejection cases. Completed-invalid second values are covered by the inspected same pre-check, without a separate fixture. |
| R07 | Resolved | ConstructionResources S796 owns lock last, and successful ResourceGuard S840 preserves that ordering. Newly opened lock is stored atS1565 before remaining checks. Temporary provider/metadata and completed batches unwind before the outer owner; S4324 observes non-lock closure while a real separate exclusive competitor still sees WouldBlock. |
| R08 | Resolved | Separate LockFingerprint S666 omits contents/length/times. Evidence fingerprints retain those guards; C01/S3964 mutates the same lock inode's contents/length/mtime and requires success. Replacement and contention cases remain. |
| R09 | Resolved | I779 checks64MiB success and +1 InvalidLimits. S3998 replacement helper saves metadata outside membership before replacement. S4251 asserts typed WouldBlock rather than using the ordinary Io mapper. |
| R10 | Resolved | No process-global FD count. Unique owner IDs and post-native-close events replace reusable slot identity as the closure oracle. Live handles still use native checks; post-close native checks are restricted to the unique fixture tree. S3249 distinguishes same-inode owners and TLS-scope exit. No suite serialization or global count is assumed. |
| R11 | Resolved | I940 restores valid depth32 bytes and succeeds with valid_control before independent one-under root-byte/component faults. Depth33 no longer masks these cases. |
| R12 | Resolved for a–h identified gaps | Each group is separately mapped below. Genuine EOF, independent total-file/batch preflight observations, deterministic closure lifetimes and bounded writer communication complete the formerly partial groups. Narrower branch/fixture limits are recorded separately. |
| R13 | Resolved | S1427 returns SizeMismatch for negative/nonrepresentable actual stat size; configured cap exceedance remains SourceScanLimit. S3333 retains cfg32/cfg64 helper assertions without allocating a huge file. Neither width was executed. |
| R14 | Resolved | S1019 validates all lexical root components before reserve/copy. S3823 observes no root/name reserve/copy/open for relative/parent faults. No canonicalize/follow fallback. |
| R15 | Resolved | S1291 validates all retained recorded sizes before batch tree/metadata/body I/O. S3855 independently observes no BeforeBatchTreeOpen/AfterMetadataCapture or metadata/provider descriptor for zero/over-cap metadata. Later actual size/hash checks remain. |
| R16 | Resolved | S2883 and I161 protect each newly created Raw-down fixture directory, using the fixture's explicit actual UID. Intentional outside-root777 and mode faults remain; there is no host-umask dependency or production permission repair. |
| C01 | Resolved/preserved | S3972 clones hook_path for the moved hook; S3991 still uses original path after capture. Entire S3964–3996 regression body is byte-identical to baseline. No moved original PathBuf use was introduced. |

| R12 group | Disposition | Valid control, observed operation and distinguishing written oracle |
| --- | --- | --- |
| a — bounded read/allocation | Resolved | S3351:3-byte exact capture plus one EOF probe; short/growing input and cap-crossing probe; expected4/cap3 has zero reads/buffer events. I792 tests exact/under actual metadata/body caps. |
| b — seed/EOF/preflight | Resolved | S3414 permissive malformed=false/EOF=true classification, cap1 resource rejection before second FileEntry; no tree/leaf opens. S3496/S3549 independently test total files/batches at2/1 with valid controls and pre-open observations. |
| c — bytes/descriptors/duplicates | Resolved | S3647 matches live native owners to counts/high water, reserve-before-open and reserve-before-buffer order, actual evidence sums and under-cap rejected provider allocation/open. I636/S3625 retain exact/under duplicate budgets and one manifest Arc charge. Real File destruction now precedes lifetime events; native post-close checks avoid shared ancestors. |
| d — path levels | Resolved | S3823 relative/parent no-copy/no-open observations; I1028 independent valid controls for root/ancestor/Raw/manifests/provider/market/data/date/batch/metadata/body/control symlinks and ordinary non-directory paths. |
| e — metadata mode | Resolved | I1337 changes only batch.json FetchMode after valid control and requires BatchMetadataMismatch; full Eq compares the committed entry. |
| f — final membership/binding | Resolved | S3998 late extra -> UnexpectedBatchEntry and same-byte metadata replacement -> SourceChanged at BeforeFinalNames; S4123 adds separate captured body/metadata loss with exact remaining names -> SourceChanged. |
| g — old partition/later interval | Resolved | I1569 preserves an earlier2020 partition with later20261012–20261014 F_DT/T_DT and checks exact captured query/order. It asserts discovery availability, not provider interval truth. |
| h — writer attempt/liveness | Resolved | I1686 waits for actual separate-writer WouldBlock before Drop, checks release nonblockingly, appends through the existing writer, and checks fresh capture. All receives are bounded; timeout does not force a blocked join. |

**P1–P10 preservation**

| Accepted contract | Disposition | Current evidence and limits |
| --- | --- | --- |
| P1 exact API/ownership | Inspected match | S39–391 is byte-identical. Capability fields/construction remain private, exact getter/lifetime signatures and two crate-only entry getters remain. OwnedSourceFile is private platform machinery; no public FD/path/Read/Seek/mutable/into extractor or capability Debug/Serialize/Deserialize/Clone/Default was added. |
| P2 scope/completeness | Inspected match | S45 accepts only kis/kis-calendar, kr and explicit UID!=MAX; UID0 receives the same comparison, no euid inference/bypass. S2107 walks the whole captured manifest; no date/batch/offset/civil filter. S262 exact lookup matches all six reference fields, safe name first, retained bytes only, no copied-source/path fallback. |
| P3 positive finite resources | Inspected match | S99 validates12 positive caps, checked+1/layout arithmetic,64MiB body/depth32 policy. S847/S889 reserve before open/evidence buffer; root/recorded-size preflight and exact FD formulas remain. S1775 now releases parsed metadata before bodies. Evidence-byte accounting is not total RSS, allocator/OOM or global-process resource proof. |
| P4 exact Raw/schema/parity | Inspected match | S2100–2572 is byte-identical:LF alone excluded, physical lines/CR/spaces/key order/full manifest retained; complete records counted before dedup; first full-Eq duplicate retained/conflict rejected. File seed caps before materialization; known duplicates before second value; legacy optional/unknown fields remain. Resource side channel precedes EOF; metadata full Eq atS1772. |
| P5 bytes/descriptors/path trust | Inspected match | All five opens are readonly descriptor-relative no-follow and wrap one real File before later checks. Raw-down UID/mode/type/link policy differs correctly from outside ancestry. Directory/lock/evidence fingerprints remain separate. Final streamed byte/stat/binding/membership checks reject changes; captured bytes remain immutable after return. Advisory/non-ABA/non-continuous limits remain explicit. |
| P6 lock/drop lifetime | Inspected match | S1219 has one nonblocking shared attempt on existing verified lock; locked ancestry/binding revalidation precedes manifest. Construction and returned ownership keep the sole lock last. No dup/retry/create/unlock gap. New test owner observes actual destruction, not an alternate cleanup algorithm. T22/T23/T26/T28 remain unexecuted. |
| P7 finite redacted errors | Inspected match | S406–511 preserves all29 fieldless variants, exact codes/Display, source None and name policy. No StoreError growth, diagnostic payload, serde/OS/path/body/request/copy/entitlement/errno disclosure or production logging. FR08 fixes the initial-versus-captured missing class; FR02 avoids parser Debug. |
| P8 ownership/preservation | Match | Only source.rs and snapshot test differ in the supplied correction. mod.rs and all existing storage/S1/writer/orphan, wire, historical, normalizer and Cargo inputs match frozen references. Registration/root exports/dependencies/workflows/diagrams remain unchanged. |
| P9 meaningful written guards | Inspected with stated coverage limits | All49 definitions and T01–T32 rows were inspected. Earlier concrete oracle/liveness defects are corrected; valid controls, observed phases and precise assertions are mapped below. Some narrower lookup/duplicate/parity cases are not isolated definitions. No test execution or universal race/ACL proof is inferred. |
| P10 Linux-only | Inspected cfg match | S512 gates the entire private machinery; S206 returns UnsupportedPlatform before I/O on every other target, including other Unix. Portable public signatures remain; I1942 is excluded on Linux. Platform compilation/execution remains unknown; the accepted platform boundary was not reopened. |

The accepted898-line contract, including section12's P10, was read in full.
Historical proposal wording is not an unresolved approval gate. D4/D5 remain
950/740lines at their pinned hashes. The API returns opaque unapproved storage
evidence, without provider validation, reconstruction/copy, normalization/admission,
DB/runtime authority or new operational policy.

**Ownership, observer and parser reasoning**

The five successful native opens atS978/S1140/S1482/S1549/S1648 immediately wrap
their OwnedFd-to-File conversion before fstat/fingerprint/named checks. No wrapper
constructor opens/duplicates a descriptor. Internal file() borrows the held File;
read/stat/RawDir/lock operations use that reference. The lock helper stores it in
ConstructionResources.lock_file before remaining fallible checks and invokes
AfterLockOpen after fstat/identity observation but before post-open fingerprint/
named validation. Replacements at that phase still reject, never adopting the new
inode; AfterSharedLock retains locked path revalidation before manifest capture.

OwnedSourceFile has exactly one Option<File>. Its Drop takes and destroys that
File before the cfg(test) closure event. Moves carry its Rc destination and unique
open ID; observation storage contains no owner back-reference or Rc cycle. Reused
FD numbers/shared device-inode pairs cannot change an earlier lifetime's ID.
Observers can outlive their TLS collection scope without losing closure events.
Current test calls release RefCell borrows before closing owners; repeated identity
assertions borrow immutably, and cleanup-time queries do not recursively capture.
No concrete RefCell borrow panic or owner leak was found in these written paths.
Test event vectors/Rc allocations are instrumentation overhead, not evidence-budget
or production allocator claims; production has no observer state or test cleanup.

Post-close native identity checks are limited to this fixture's unique tree.
tree_state's temporary reads close before checks; parallel tests do not own those
objects. Shared ancestors use recorded destruction of the actual File, never
post-close raw-slot identity. Live owners still require native identity matches,
all successful opens are counted, and the cleanup-time separately opened writer
must see WouldBlock before the sole lock's close. Normal Drop and eight injected
failure phases check once-only closure, last lock event, native fixture closure,
writer availability and unchanged tree, excluding atime. This is meaningful
written evidence, not an executed leak/parallel/lock result.

Inside failing provider/metadata capture, temporary handles unwind before the
outer scan owner. Completed batches declared after the scan owner drop first.
ConstructionResources drops manifest/directories/names before its final lock field;
the helper's newly opened lock now follows that order even for validation failure.
Successful transfer moves the same owners into ResourceGuard without new opens
or locking. Snapshot batches drop before the guard, which closes the sole lock last.
The short parsed-metadata scope atS1770–1775 preserves full equality and releases
its owned strings/vectors before any body-bearing construction.

Accounting remains C+7 for an empty scope and C+9+D+2B+F otherwise. The captured
manifest is moved into one Arc, line commitments are ranges, distinct metadata/body
bytes are charged before allocation, and provider charge also precedes open.
Actual owned opens/high water and captured byte lengths are checked by the written
observer. RawDir borrows the existing directory with4KiB buffer/rewind, no dup.
Captures/final comparisons use8KiB scratch plus one-byte probes, no unbounded read.
No caller truncation/filter, hidden cap or replacement Value-tree parser exists.

Terminated nonblank rows count before parse; complete unterminated rows count
before dedup. Only an actual final Json.is_eof error is ignored. Explicit depth,
file-count and allocation resource failures retain their typed classes. Duplicate
known fields, including optional null, reject before consuming the second value.
Unknown Raw metadata remains ignored in parsed equality but protected by exact
bytes; provider bodies stay opaque. Final missing captured membership is now
SourceChanged, while initial absence and late extras keep their distinct errors.

T26's three5second communication bounds avoid the original unbounded receive.
An unsuccessful post-Drop exclusive probe returns before blocking append. A timed
out worker is not joined; closing continuation/completion channels permits normal
error paths to exit. This does not make underlying OS I/O deadline-bound or prove
that an arbitrary stalled OS worker can be killed. No test ran or background
process was started by this reviewer.

**T01–T32 guard dispositions**

Every row is written/static evidence, UNEXECUTED. Exact definition names follow.
“Match” concerns the specified guard/oracle, with the listed limits; it is not a
compiler/test PASS. Native filesystem controls are fresh, synthetic and explicit-UID.

| Guard | Written anchors | Control, isolated fault/operation, oracle and disposition |
| --- | --- | --- |
| T01 | I398; S301/S348/S1392 | Match. Two batches/three independently distinct bodies; checks all ordered views, lengths/exhaustion, fixture identity/hash/size/bytes and batch association, every exact line/metadata commitment and measured stats. FR07 closed. |
| T02 | I501; S2107 | Match. S1 compares common valid fixture entry/line/position before CRLF/spaces/key-order/blank rewrite; S2 then checks exact LF-excluding bytes/hash, physical1/3, full hash and count2. Modified-case S1 rerun is not written. |
| T03 | I560/I592; S3600/S2533 | Match. Complete unterminated count1, byte-retained ignored torn tail, terminated/non-EOF malformed rejection; all8 known duplicate incomplete second values reject against legacy; bounded sequence Eq maps errors safely. T08 adds genuine EOF/resource distinction. |
| T04 | I636; S3625/S2179 | Match. Equal whitespace duplicate count2 keeps first exact line/order, one batch/file capture and Arc charge; entitlement conflict -> ManifestConflict. Exact/under duplicate retained budget asserted. |
| T05 | I691; S1253/S2139 | Match. Exact manifest bytes/one complete final record; one-under byte cap rejects; duplicate complete final record counts before dedup; torn-tail bytes cannot evade cap. No prefix success. |
| T06 | I740; S3333/S99/S1427 | Match. All12 zero caps, byte+1/layout overflow,64MiB/+1 and depth33; actual negative/nonrepresentable stat helper with width gates. Constructor has no I/O; huge allocations and widths unexecuted. |
| T07 | I792; S3351/S3855/S2051 | Match. Actual metadata/body exact/under caps; counting reader exact/probe/short/growing/cap cases and zero-read allocation rejection. Recorded-size preflight observes no data-tree/leaf open for known faults. |
| T08 | I819; S3414/S3496/S3549 | Match. Public per-batch/total/batch and many-small-field caps; private first-element control, malformed and genuine EOF classification, cap1 resource before second materialization and no data opens. Independent total/batch2/1 controls observe pre-open failure. FR01/05 closed. |
| T09 | I856; S3647/S889 | Match. Individual files fit while aggregate exact succeeds/one-under rejects. Actual sums, reserve-before-buffer/provider-open, rejected body absent and duplicate charge once; all owned handles close. Parsed metadata release is lexical evidence, not RSS measurement. |
| T10 | I886; S3647/S847/S1293 | Match. Exact/under FD formula, shared dates and deep root; live native owner/reservation/attempt counts equal current/peak, no duplicate live FD/open ID, no tree opens after full preflight failure. Per-owner closure replaces global/reused-slot counts. |
| T11 | I940; S1019/S2239 | Match. Independent valid depth32/root equality controls, unknown depth33 and one-under root bytes/components. Restore valid bytes before root faults; lexical depth pass is bounded, not a JSON validity classifier. |
| T12 | I982; S1212/S1243 | Match. Valid controls before missing lock/manifest/control parent/Raw, MissingControl and unchanged absence; existing empty manifest succeeds with C+7. No repair/create. |
| T13 | I1028; S3823/S958/S1019 | Match. Each distinct no-follow path level and ordinary non-directory faults have controls; Linux parent/relative roots reject with no copy/reserve/open. No canonicalize/fallback. |
| T14 | I1223; S3310/S608/S704 | Match for source/fixture policy. FIFO/directory body -> NonRegularFile; wrong expected UID -> UntrustedOwner; body/batch022 -> WritableNode; synthetic UID0 equality/no bypass and link checks. No actual cross-principal/device/ACL experiment. |
| T15 | I1289; S3549/S608/S678 | Match. Separate valid controls before hardlinked body/manifest/lock/metadata -> HardLinkedFile. New batch-cap valid control has independent same-hash body files across two batches. Same-batch distinct-basename equal-hash pair is not an isolated fixture; source has no hash-uniqueness prohibition. |
| T16 | I1337; S1772/S2169 | Match. Committed provider/market faults -> ScopeMismatch. Fresh controls before isolated metadata date/ID/request/time/mode/size/order changes -> BatchMetadataMismatch. Full derived Eq includes all fields. |
| T17 | I1434; S3998/S4123/S1852 | Match. Identical duplicate/reserved/traversal names, initial extras/temp/subdir have precise errors. Final hook extra -> UnexpectedBatchEntry; lost captured names -> SourceChanged. Late-extra case reaches the successful final phase rather than making a separate full pre-scan. |
| T18 | I1492; S3855/S1438 | Match. Valid controls before missing metadata/body, torn metadata, zero recorded size, truncation and same-size corruption -> precise missing/invalid/size/hash errors. Dedicated no-tree size preflight; no failed batch skipped. |
| T19 | I1543; S1325 | Match. Valid committed control, complete/incomplete uncommitted orphans; only committed batch exposed and full content/mode/UID/inode/link tree unchanged after Drop, excluding atime. No reconciliation. |
| T20 | I1569; S1325/S352 | Match. Earlier2020 partition remains in first order with later F_DT/T_DT query; exact captured query available. No date exclusion or provider-interval verification claim. |
| T21 | I1599; S262 | Match for written cases and inspected field guards. Valid hit, absent batch/file, provider/hash/time mismatch and absent copied-source name -> precise errors. Market and unsafe/reserved lookup names are checked in source but lack isolated lookup-only fixtures. |
| T22 | I1642; S1219/S840 | Match. Two simultaneous shared readers; separately opened exclusive writer denied until both drop, then succeeds. No dup competitor. is_err denial here is supplemented by actual WouldBlock in T23/T26/T28. |
| T23 | I1661; S3944/S3964/S1532 | Match. Existing writer exclusive -> LockBusy without retry, unlock restores valid scan. Deterministic lock replacement before/after acquisition -> SourceChanged. Same inode content/length/time change remains valid. C01 retained. |
| T24 | S3998/S4123; S1346/S1354 | Match. Controls then deterministic Raw/control/manifest/batch/metadata/body replacements at capture/final phases -> SourceChanged; metadata inode saved outside set. Late extras and separate move-outside captured body/metadata cases distinguish final membership and binding classes. |
| T25 | S4163; S1920/S1960/S1977 | Match. Valid controls then in-place manifest/metadata/body mutation and mode changes -> SourceChanged; hardlink at reached AfterBodyCapture -> HardLinkedFile; synthetic owner mismatch is separate. No real chown/ABA/mutation-restore proof. |
| T26 | I1686; S1219/S840 | Match. Actual pre-Drop nonblocking writer WouldBlock, bounded handshake, post-Drop release probe before existing append, bounded completion/error cleanup and fresh second-ID capture. No scheduler-only inference/sleep or production retry policy. |
| T27 | I1779; S383 | Match. Valid snapshot/lookup, post-return deliberate body corruption leaves captured bytes unchanged; after Drop fresh capture -> HashMismatch. No continuous path validity claim. |
| T28 | S3249/S4251/S4324; S541/S796/S840 | Match. Same-inode owner distinction/TLS exit; eight phase failures and normal Drop close every observed real owner exactly once, lock last, tree unchanged and writer free afterward. At cleanup, temporaries are gone while actual separate writer still sees WouldBlock. Native behavior remains unexecuted. |
| T29 | I1797; S406/S438/S480 | Match. Valid controls before malformed mode/hash/UUID/name and request/copy/entitlement/OS sentinel paths; finite Display/code/Debug and source None. Fieldless production API cannot retain nested payloads; no wrapped logging path. |
| T30 | I1895; S2329/S2559 | Match. Valid control then unknown Raw metadata and omitted optional continuation/copy; full Eq remains, correct-hash non-JSON provider body stays opaque. Raw parity is distinct from D4 wire/provider validation. |
| T31 | I1942; S206/S512 | Static cfg match. Non-Linux-only definition uses valid explicit scope/limits and nonexistent relative path -> UnsupportedPlatform. Source branch has no I/O; excluded on Linux, unexecuted elsewhere. |
| T32 | I1926/I1223; S45/S704 | Match for library boundary. Fixture-derived explicit UID and writable outside ancestor permit valid capture; wrong expected UID rejects. No euid inference, repair/fallback or actual cross-UID/ACL/mount/access proof. |

**Narrower coverage limits and relevance**

| Previously noted narrower case | Current disposition |
| --- | --- |
| Completed-invalid second value of a known duplicate | No separate fixture. All8 incomplete-value cases distinguish pre-value rejection, and unchanged source performs the same presence check before any completed value. R06/FR05 corrections stand; no contrary source behavior was found. |
| Lookup-only market and unsafe/reserved name cases | Not isolated in I1599. S262 checks names first and both provider/market before lookup; storage-level name/market cases exist elsewhere. This is a branch-coverage limit, not an identified source defect or new policy. |
| Independent files with equal content hash | S3549 now exercises a valid pair across two independent batch trees with cloned FileEntry/BODY and newly created files. Thus the broad “no equal-hash independent pair” statement is no longer accurate for this candidate. An isolated same-batch/different-basename pair remains absent. No content-hash uniqueness rule exists in source. |
| S1 comparison on modified CRLF/reordered keys | I501 compares S1 on the original common valid fixture, then checks modified exact bytes/hash/positions only in S2. Modified-case S1 rerun remains absent. Both inspected parsers preserve LF-only exclusion and legacy typed equality; no executed parity claim is made. |

These limits are preserved or narrowed within the fixed correction scope. They do
not justify an invented architecture/platform/policy amendment. No additional
mandatory source correction was identified; coordinator executable verification
and disposition of coverage limits remain follow-ups.

**All49 definition dispositions**

Private names below have exact prefix `daily_price_only::source::linux::private_tests::`.
Linux integration names have prefix `linux_tests::` in binary daily_price_only_snapshot.
T31 has no module prefix. Source declaration/cfg inspection and inventory comparison
establish these names; no Cargo listing was executed. Every row is WRITTEN_UNEXECUTED.

<!-- Definition rows and complete pin checks follow from the checked metadata. -->

| Definition / source anchor | Gate / Linux expectation | Static disposition |
| --- | --- | --- |
| [s2_t28_shared_inode_owners_keep_distinct_lifetimes_across_observation_scope](../../crates/market-data/src/daily_price_only/source.rs#L3249) (S3249) | private Linux / expected | Match; same-inode separate owners, moves/TLS exit and once-only native-owner closure; T28/FR04. |
| [s2_t14_synthetic_raw_stat_owner_mode_and_link_guards_include_uid_zero](../../crates/market-data/src/daily_price_only/source.rs#L3310) (S3310) | private Linux / expected | Match; valid UID0/explicit-owner controls, wrong owner, mode and links; T14/T15. |
| [s2_t06_stat_size_conversion_uses_size_mismatch_for_nonrepresentable_values](../../crates/market-data/src/daily_price_only/source.rs#L3333) (S3333) | private Linux / expected | Match; negative/width-gated nonrepresentable actual size; T06/R13. |
| [s2_t07_bounded_reads_probe_once_and_reject_caps_before_allocation](../../crates/market-data/src/daily_price_only/source.rs#L3351) (S3351) | private Linux / expected | Match; exact/probe/short/growth/cap and zero-read pre-allocation rejection; T07/R12a. |
| [s2_t08_file_seed_rejects_before_materialization_and_data_tree_open](../../crates/market-data/src/daily_price_only/source.rs#L3414) (S3414) | private Linux / expected | Match; exact control, permissive malformed/EOF, cap1 one materialized/no tree; T08/FR01/05. |
| [s2_t08_total_file_cap_rejects_before_any_batch_tree_or_leaf_open](../../crates/market-data/src/daily_price_only/source.rs#L3496) (S3496) | private Linux / expected | Match; independently valid total2 -> cap1 rejection/no tree/leaf open; T08. |
| [s2_t08_unique_batch_cap_rejects_before_any_batch_tree_or_leaf_open](../../crates/market-data/src/daily_price_only/source.rs#L3549) (S3549) | private Linux / expected | Match; valid two independent same-hash batch bodies -> batch cap1/no tree; T08/T15. |
| [s2_t03_bounded_entry_sequence_schema_matches_legacy_serde](../../crates/market-data/src/daily_price_only/source.rs#L3600) (S3600) | private Linux / expected | Match; legacy/bounded complete sequence Eq with fieldless mapped error; T03/FR02. |
| [s2_t04_identical_rows_retain_one_batch_arc_and_one_evidence_charge](../../crates/market-data/src/daily_price_only/source.rs#L3625) (S3625) | private Linux / expected | Match; record2/batch1/Arc2 and exact evidence charge; T04. |
| [s2_t09_t10_owned_resource_observations_match_the_capture](../../crates/market-data/src/daily_price_only/source.rs#L3647) (S3647) | private Linux / expected | Match; native live owner counts, pre-operation reserves, exact actual bytes and under-cap cleanup; T09/T10. |
| [s2_t13_parent_and_relative_roots_reject_before_component_copy_or_open](../../crates/market-data/src/daily_price_only/source.rs#L3823) (S3823) | private Linux / expected | Match; valid control then lexical faults with zero copy/reserve/open; T13/R14. |
| [r15_recorded_body_size_precedes_data_tree_and_leaf_io](../../crates/market-data/src/daily_price_only/source.rs#L3855) (S3855) | private Linux / expected | Match; independent controls before zero/over-cap records; no tree/metadata/provider I/O; R15/T07/T18. |
| [s2_t23_lock_path_replacement_before_and_after_shared_acquisition_fails](../../crates/market-data/src/daily_price_only/source.rs#L3944) (S3944) | private Linux / expected | Match; controls, invoked before/after lock replacement, SourceChanged; T23. |
| [s2_t23_same_lock_inode_ignores_contents_length_and_timestamps](../../crates/market-data/src/daily_price_only/source.rs#L3964) (S3964) | private Linux / expected | Match; control, same inode volatile changes remain valid; C01/T23. |
| [s2_t24_replacements_at_capture_and_final_hooks_are_rejected](../../crates/market-data/src/daily_price_only/source.rs#L3998) (S3998) | private Linux / expected | Match; deterministic replacements/final extras with distinguishing precise classes; T24/T17. |
| [s2_t24_final_membership_loss_of_body_or_metadata_is_source_changed](../../crates/market-data/src/daily_price_only/source.rs#L4123) (S4123) | private Linux / expected | Match; separate valid body/metadata final move-outside cases, exact remaining names; T24/FR08. |
| [s2_t25_in_place_bytes_mode_and_link_mutations_fail_revalidation](../../crates/market-data/src/daily_price_only/source.rs#L4163) (S4163) | private Linux / expected | Match; controls/reached phase then observable bytes/mode/link/owner faults; T25. |
| [s2_t28_injected_failures_release_lock_descriptors_and_preserve_tree](../../crates/market-data/src/daily_price_only/source.rs#L4251) (S4251) | private Linux / expected | Match; eight injected phases plus normal Drop, real-owner closure/last lock/tree/competitor; T28. |
| [s2_t28_failure_unwind_closes_temporary_leaves_before_lock_release](../../crates/market-data/src/daily_price_only/source.rs#L4324) (S4324) | private Linux / expected | Match; body failure, temporary native closure and real WouldBlock before lock release; T28/R07. |
| [s2_t01_valid_multi_batch_snapshot_keeps_exact_evidence_and_stats](../../crates/market-data/tests/daily_price_only_snapshot.rs#L398) (I398) | integration Linux / expected | Match; all batch/file views and independent exact commitments/stats; T01/FR07. |
| [s2_t02_line_bytes_keep_whitespace_crlf_order_and_physical_line](../../crates/market-data/tests/daily_price_only_snapshot.rs#L501) (I501) | integration Linux / expected | Match with modified-case S1 limit; exact bytes/hash/physical lines; T02. |
| [s2_t03_final_record_eof_tail_and_terminated_errors_are_distinct](../../crates/market-data/tests/daily_price_only_snapshot.rs#L560) (I560) | integration Linux / expected | Match; complete/torn/terminated/non-EOF classes and counts; T03. |
| [s2_t03_known_duplicate_fields_reject_before_an_incomplete_second_value](../../crates/market-data/tests/daily_price_only_snapshot.rs#L592) (I592) | integration Linux / expected | Match; all8 known fields, optional-null and legacy rejection; T03/R06. |
| [s2_t04_equal_duplicates_keep_first_line_and_conflicts_fail_closed](../../crates/market-data/tests/daily_price_only_snapshot.rs#L636) (I636) | integration Linux / expected | Match; first equal line/count, exact/under charge, conflict; T04. |
| [s2_t05_manifest_byte_and_pre_dedup_record_caps_are_exact](../../crates/market-data/tests/daily_price_only_snapshot.rs#L691) (I691) | integration Linux / expected | Match; byte/record equality and over-budget complete/torn/duplicate rows; T05. |
| [s2_t06_zero_overflow_and_policy_invalid_limits_fail_in_construction](../../crates/market-data/tests/daily_price_only_snapshot.rs#L740) (I740) | integration Linux / expected | Match; twelve zero, arithmetic/layout, body/depth policy controls; T06. |
| [s2_t07_batch_metadata_and_body_byte_caps_precede_capture](../../crates/market-data/tests/daily_price_only_snapshot.rs#L792) (I792) | integration Linux / expected | Match; valid exact actual metadata/body caps, isolated one-under; T07. |
| [s2_t08_per_batch_and_total_file_counts_fail_before_body_open](../../crates/market-data/tests/daily_price_only_snapshot.rs#L819) (I819) | integration Linux / expected | Match with private timing supplements; per-batch/total/batch/small-field caps; T08. |
| [s2_t09_aggregate_retained_bytes_accept_exact_and_reject_one_under](../../crates/market-data/tests/daily_price_only_snapshot.rs#L856) (I856) | integration Linux / expected | Match; individually valid bodies, aggregate exact/under; T09. |
| [s2_t10_descriptor_formula_is_measured_and_preflighted](../../crates/market-data/tests/daily_price_only_snapshot.rs#L886) (I886) | integration Linux / expected | Match with private native supplement; exact/under, same date and deep root; T10. |
| [s2_t11_root_path_and_unknown_json_depth_caps_are_exact](../../crates/market-data/tests/daily_price_only_snapshot.rs#L940) (I940) | integration Linux / expected | Match; independent depth32/33 and root equality/under controls; T11. |
| [s2_t12_missing_controls_do_not_repair_and_empty_manifest_is_valid](../../crates/market-data/tests/daily_price_only_snapshot.rs#L982) (I982) | integration Linux / expected | Match; fresh controls, precise missing/no repair and empty C+7; T12. |
| [s2_t13_root_control_batch_and_leaf_symlinks_fail_closed](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1028) (I1028) | integration Linux / expected | Match; independent no-follow levels/non-directory with valid controls; T13. |
| [s2_t14_special_nodes_owner_and_mode_guards_do_not_block_or_bypass](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1223) (I1223) | integration Linux / expected | Match; FIFO/directory/mode/wrong expected UID, no OS cross-UID proof; T14. |
| [s2_t15_hardlinked_lock_manifest_metadata_and_body_are_rejected](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1289) (I1289) | integration Linux / expected | Match; separate valid controls then four hardlink classes; T15. |
| [s2_t16_scope_and_complete_batch_metadata_equality_are_required](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1337) (I1337) | integration Linux / expected | Match; isolated scope/date/ID/request/time/mode/size/order mismatches; T16. |
| [s2_t17_duplicate_reserved_traversal_and_extra_entries_fail](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1434) (I1434) | integration Linux / expected | Match; controls then names/duplicate/extra precise rejection; final supplement; T17. |
| [s2_t18_missing_invalid_zero_size_and_same_size_corruption_reject](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1492) (I1492) | integration Linux / expected | Match; controls then metadata/body missing, invalid, zero, size/hash faults; T18. |
| [s2_t19_uncommitted_orphans_are_not_opened_or_reconciled](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1543) (I1543) | integration Linux / expected | Match; committed-only capture and unchanged complete/incomplete orphan tree; T19. |
| [s2_t20_batch_partition_date_does_not_limit_committed_discovery](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1569) (I1569) | integration Linux / expected | Match; old partition/later query metadata in full first order; T20. |
| [s2_t21_exact_lookup_matches_full_file_identity_without_copy_fallback](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1599) (I1599) | integration Linux / expected | Match for hit/absent/provider/hash/time/copy-name cases; market/name branch limits; T21. |
| [s2_t22_two_readers_hold_shared_lock_until_both_drop](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1642) (I1642) | integration Linux / expected | Match; actual two-reader hold and separately opened exclusive writer; T22. |
| [s2_t23_exclusive_writer_is_reported_busy_without_retry](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1661) (I1661) | integration Linux / expected | Match; valid control, typed WouldBlock/LockBusy, unlock restores success; T23. |
| [s2_t26_cooperating_append_publishes_after_snapshot_drop](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1686) (I1686) | integration Linux / expected | Match; real contention, three bounded receives, release probe, append/fresh capture; T26/FR06. |
| [s2_t27_returned_bytes_remain_captured_after_disk_mutation](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1779) (I1779) | integration Linux / expected | Match; valid lookup, captured bytes unchanged after disk corruption; fresh rejection; T27. |
| [s2_t29_fieldless_errors_never_format_raw_sentinels](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1797) (I1797) | integration Linux / expected | Match; sentinel classes, finite formatting/source None without helper shadow; T29. |
| [s2_t30_unknown_fields_optional_raw_fields_and_invalid_provider_body_stay_opaque](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1895) (I1895) | integration Linux / expected | Match; valid control, unknown/omitted Raw fields and opaque non-JSON body; T30. |
| [s2_t32_explicit_fixture_uid_and_writable_outside_ancestor_policy_are_preserved](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1926) (I1926) | integration Linux / expected | Match; explicit fresh-fixture UID and writable outside ancestor; T32. |
| [s2_t31_non_linux_returns_unsupported_without_path_io](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1942) (I1942) | non-Linux / excluded | Static cfg/oracle match; excluded on Linux, no execution elsewhere; T31. |

**Checked files and complete pin results**

The table records byte/newline identity, not a claim to have re-reviewed every
historical source. Read in full:AGENTS.md, accepted contract, both prior reviews,
current S/I/mod, second-correction scope/handoff,176-line worker final and complete
1749-line delta. Read relevant ROADMAP R2-04/09 and Wave28 including actual startup.
Readonly adjacent checks used storage ManifestEntry/FileEntry/S1 EOF/count/full Eq,
append_manifest's existing exclusive-lock path, RequestMetadata/FetchMode/full
SourceFileReference, and concrete domain Copy/hash signatures. Other historical
source/evidence artifacts were hashed for preservation, without raw logs or a
repository-wide survey. Bounded applicable AGENTS/CLAUDE path checks found only the
root AGENTS.md under the reviewed ancestors/subdirectories.

All19 original source/reference pins are unchanged, including storage/S1 reader,
writer/orphan behavior, pure wire/tests, normalizer, historical v2/v3 and Cargo.
The22 references include the correction review and second-correction scope/handoff;
the original review is preserved among the31 artifacts. There is no current
source/dependency/workflow/platform expansion.
The retained historical error/CI artifacts were not rewritten or treated as current
S2 compilation evidence. Coordinator plan/roadmap startup bookkeeping is unpinned.

| Group / checked pin | Lines | SHA256 | Start/final |
| --- | ---: | --- | --- |
| targets: [crates/market-data/src/daily_price_only/source.rs](../../crates/market-data/src/daily_price_only/source.rs) | 4413 | `0a6c29c7fff0bdbc48c5af0f1950c8522542270e43d374b9ee8ae6243b2729d3` | MATCH / MATCH |
| targets: [crates/market-data/tests/daily_price_only_snapshot.rs](../../crates/market-data/tests/daily_price_only_snapshot.rs) | 1970 | `a5db9e0adc4ca909835be3a717c5579a8c1b356f7c1b7036f4dc6cae8cbb7f61` | MATCH / MATCH |
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
| readOnlyReferences: [docs/reviews/2026-10-14-r2-raw-snapshot-corrections-review.md](../../docs/reviews/2026-10-14-r2-raw-snapshot-corrections-review.md) | 543 | `ebe51618235e06b3afe7e8075ecf7feb4c137d918a67c7be3ee715803f07d344` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-snapshot-second-correction-scope.md](../../docs/reviews/2026-10-14-r2-snapshot-second-correction-scope.md) | 98 | `1357ccf571033d1eb2ddafa3911b5b35377222dffd714cc4e58d3cd0d75a16b8` | MATCH / MATCH |
| readOnlyReferences: [docs/reviews/2026-10-14-r2-snapshot-second-corrections-handoff.md](../../docs/reviews/2026-10-14-r2-snapshot-second-corrections-handoff.md) | 75 | `c25f7ebe730c965b2fbb49303c1c4b2d6a7079c2d36989813bae625daaae95cd` | MATCH / MATCH |
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
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-26-S2-review-inputs.json](../../target/r2-daily-beta/wp-4-integration/source-wave-26-S2-review-inputs.json) | 186 | `9bffe79342b2a5a1b61691ff63b6a1b36a992ddb9357d763479ae21272cc83bf` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-final.md](../../target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-final.md) | 9 | `913a95e4340afe817b732b703509d920fae971cda01ff178c6e576bb4279d749` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-recovery.json](../../target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-recovery.json) | 46 | `4a09e4acecb7a07a81de503ca813a55701e24cbbad14a6c24030798111861251` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-coordinator-verification.json](../../target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-coordinator-verification.json) | 32 | `7e59c087ac6d8b2a50a1ddeb4b9913a30f4c61d44634d165505442e8f6d4b353` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-27-S2-baseline-test-inventory.json](../../target/r2-daily-beta/wp-4-integration/source-wave-27-S2-baseline-test-inventory.json) | 423 | `1d8ef030a3b3e285fe1f5cc8b6edde9cb9bb667bd5bf8bab3c9efb79e8738a54` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-S2-baseline-source.txt](../../target/r2-daily-beta/wp-4-integration/wave27-S2-baseline-source.txt) | 3987 | `4577d5f10bbe4dda508c89b25ff04c638a274bc5050e5f9e9b7e03b5ba4d8bb5` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-S2-baseline-snapshot.txt](../../target/r2-daily-beta/wp-4-integration/wave27-S2-baseline-snapshot.txt) | 1896 | `754a32ec22ba32f7b6c57da9e3dfb335cdeb064926ecedd1591f4ff3c74d9dd7` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-S2-correction-brief.txt](../../target/r2-daily-beta/wp-4-integration/wave27-S2-correction-brief.txt) | 64 | `b398a6b0369b9058f14e782414623c42488d8a463d512935000e38eae287a42a` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-27-S2-correction-inputs.json](../../target/r2-daily-beta/wp-4-integration/source-wave-27-S2-correction-inputs.json) | 243 | `839727f0a3a248d0070f968297853a2a8b4e105ae81b73ba02a5dab8ad80ab85` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-final.md](../../target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-final.md) | 176 | `f52a070c927450b2adc9eef07c75d9b74052551ac355f887dd9f815ee2f2f2bf` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-recovery.json](../../target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-recovery.json) | 28 | `1c88cdb9311462684b0a8bd5cab6942f83d3f8d283cd40b993fed5dcb1838a7f` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-coordinator-verification.json](../../target/r2-daily-beta/wp-4-integration/wave27-I1b-S2-F2-coordinator-verification.json) | 109 | `5e2e7ea1fd2e7b509b9eada8dd64d1e97cc75775783fba4ce3253b74b7539175` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave27-S2-second-correction.diff](../../target/r2-daily-beta/wp-4-integration/wave27-S2-second-correction.diff) | 1749 | `ec132c244bf22e22bcd06f6a749014a33d7305b8760506a768a4f45b92c2a19c` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/source-wave-28-S2-test-inventory.json](../../target/r2-daily-beta/wp-4-integration/source-wave-28-S2-test-inventory.json) | 474 | `91c08ebbd5f9969470f90c0614462de77b6b496b4133e9701f87d24ec9f03bf6` | MATCH / MATCH |
| evidenceArtifacts: [target/r2-daily-beta/wp-4-integration/wave28-S2-review-brief.txt](../../target/r2-daily-beta/wp-4-integration/wave28-S2-review-brief.txt) | 47 | `2d7e18a66c0fa021360462f8e63754a2152f53d3fdce53ee51f294378812e674` | MATCH / MATCH |

Supplemental bounded reads for concrete inherited-edition/type checks are listed
separately, because these are not additions to the frozen56-pin inventory:

| Supplemental file | Lines | Observed SHA256 | Reason |
| --- | ---: | --- | --- |
| [Cargo.toml](../../Cargo.toml) | 77 | `cd6895e63076449191b96adcebac53050311ee2e7a1dd055cf623e386abd1f7b` | market-data inherits workspace edition2024; no Cargo command was run. |
| [crates/domain/src/ids.rs](../../crates/domain/src/ids.rs) | 274 | `7c54ddfe3416f10dcc8712810aa4e19d8c2a020561be6641e9adecdc3c1a8056` | BatchId macro derives Copy/Ord and from_uuid type. |
| [crates/domain/src/time.rs](../../crates/domain/src/time.rs) | 412 | `6bfd42349ee7e7c4e92298095f63fec9d2a1244e0620dc1f80e2669211b0b545` | UtcTimestamp/TradingDate Copy and exact typed equality. |
| [crates/domain/src/hash.rs](../../crates/domain/src/hash.rs) | 177 | `7d5b49d42d79e86caf4bd05618f3f6871a2fd3f4a5383111f8f32960600eefd1` | ContentHash Eq and from_bytes borrowed-slice signature. |

**Actual checks and results**

| Actual check | Result and limit |
| --- | --- |
| Inert metadata JSON, SHA256/newline comparison:input inventory plus all56 pins, startup/final | MATCH, zero missing/hash/count mismatches. No Raw/fixture/source parser was executed. |
| Complete source/contract/review reads and59 supplied diff hunk text checks | Every old/new slice/count/position matched the supplied baselines and current texts; exactly two delta paths. No Git action or executable patch/helper. |
| Plain diff -u on the two supplied baselines/current files | Expected exit1, no error diagnostics; +694/-268 source and +159/-85 integration. Supplied hunk alignment +692/-266 source has identical final bytes. In-memory SequenceMatcher gave another +696/-270 alignment; the net/current text is the same. |
| Eight bounded source-region byte comparisons | All MATCH at the ranges reported above; registration unchanged. |
|49 textual test attributes/function definitions, actual lines/names/platforms against474-line inventory | MATCH:19 private Linux,29 integration Linux,1 non-Linux;48 Linux expected. All45 baseline names retained, four additions, no ignore. E01 names corrected. No compiler test enumeration. |
| rustfmt --edition 2021 --config skip_children=true --check crates/market-data/src/daily_price_only/source.rs crates/market-data/tests/daily_price_only_snapshot.rs | Exit0, no output. Only the requested syntax/format check; it does not resolve names/types/borrows, run Clippy or establish the inherited-edition Cargo gates. |
| Scoped inert added-line and target whitespace/final-LF checks | Zero trailing-whitespace additions or target lines; all three targets have final LF. |
| Report inert local-link/line-bound, whitespace and complete disposition checks | PASS after final draft; no broken local links/out-of-bounds anchors, missing definition/required row or trailing whitespace. |

No Cargo/rustc/Clippy/build/test, fixture/Raw parser/helper/checker/validator/
provisioner/self-test, standalone extracted probe, SQL/DB/root/sudo/Docker,
provider/network/production/workflow/CI action or dependency installation occurred.
No source function was executed, no source fix or other-document write, no Git
command/staging/commit/push, no delegation/agent/advisor/committee/Paseo workflow.
Only this report is the review work product. The exact live memory hook is the
sole authorized exception for its private operational receipts/loopback service.

Old a888cd66c032c8931a4979434168568387c83a12/run38545976070 attempt1 SUCCESS
is I0c-only partial evidence:51 direct Rust and6 separate chart rows, strict
Clippy/required and PG59/cleanup; it contains neither new S2 file. Original receipt
and supplement retain nested helper result rows/exits UNKNOWN. No new S2 CI was
supplied/dispatched/queried, and no prior run was rerun/cancelled. Previous partial
successes/failures remain historical, not proof for the49 current definitions.

**Unresolved, unknown and handoff**

Unresolved concrete source/test findings within this reviewed correction:none.
Necessary additional source/contract/API/Cargo/platform changes identified:none.
Missing necessary frozen review inputs:none. Conflicting requirements:none.
Current inventory/name/line discrepancies:none. Required review rows or definitions
left uninspected:none. The narrower written-coverage limits are explicit above.

Not verified:current compilation or actual compiler diagnostics, strict all-targets/
all-features -D warnings, Cargo formatting gates, any of49 definitions, Linux native
lock/path/race/FD/parallel/cleanup behavior, non-Linux compilation/no-I/O behavior,
32-bit size behavior, actual cross-principal UID/ACL/mount/access arrangements,
runtime allocation lifetime/RSS, installer live UID/root/caps or consumer scheduling/
lifetime policy, new exact-candidate CI. All remain UNEXECUTED/unknown evidence,
not observed runtime failures. No universal ABA/adversarial-filesystem or continuous
attestation claim is made. Captured evidence alone conveys no provider truth or
admission rights.

Outstanding follow-up:coordinator independently validates this report and finding
closure after actual idle/pending0/full substantive recovery, retains source
acceptance, and queries existing exact-candidate runs before any later single
hosted verification. Fresh compilation/strict lint and direct named Linux test
results, plus existing S1/wire/historical regressions, remain necessary evidence.
There is no reviewer-owned source repair or permission request outstanding.

Negative bootstrap/rollback,0060, provider/copy reconstruction, normalization/
admission,I6 structural diagrams before main,runtime/Owner and final C_R2 remain
accepted later packages. Those are not missing inputs or reopened requirements for
this bounded correction. P10 remains binding.

Scope deviations:none in output/execution scope. Supplemental edition/domain reads
were bounded concrete type checks, not a general adjacent architecture review.
Only authorized current memory-hook receipts are outside the report write allowlist.
The fresh context call failed MCP_ERROR/no response; no history was recovered or
used and no failed call was represented as saved. The same exact current envelope's
finish returned SKIPPED/no_durable_change, saved=false, skipped=true, incomplete=false.
No stale/guessed envelope, alternate scope, durable save or Basic Memory was used.

Further decomposition required for this completed direct review:none. Background
review work/processes:none. Background/permission waits:none. Pending permissions:
none. Auto-review denials:none. The report is completed within its60minute bound;
all frozen bytes remain unchanged. Final report line range/count and SHA256 are
returned in the substantive handoff after the last edit, without a self-referential
hash commitment in this file.
