# WP-4/I1b-S2-D — exact committed Raw snapshot implementation contract

Status: **ACCEPTED for bounded source implementation; P10 below narrows the platform.**
2026-10-14; R2-04/09; branch `excited-dolphin`. Direct architecture work, no delegation.
Paseo actual startup: `2026-10-14T09:20:40.793Z`; reporting deadline: `10:05:40.793Z`.
Brief/acceptance HEAD: `1a2321bfe6c6d3978ff60a48d7a2aadb895007c7`.
Requested selection is codex/gpt-6-astra/xhigh under the architecture rule; this
session cannot independently inspect or change its own model/effort configuration.

This contract supplies one complete, bounded, read-only, single-scope snapshot
primitive. It does not implement or activate it. The accepted source candidate is
`c0ac599cb1ede900e155f724687c57410b4bd0d9`; the existing CI success is partial source
acceptance, not evidence for this proposed primitive. The coordinator's recovery at
09:11:14.901 UTC and 26-source/49-Rust/6-chart comparison are supplied brief evidence;
the frozen [CI report:3](2026-10-14-r2-caller-role-raw-ci.md#L3),
[direct results:16](2026-10-14-r2-caller-role-raw-ci.md#L16), and
[limits:24](2026-10-14-r2-caller-role-raw-ci.md#L24) remain unchanged.

## 1. Authority, inputs and decision labels

`B` denotes an inherited binding requirement, with its source below. `P1`–`P9`
denote coordinator decision proposals made by this report. All signatures, private
representations, algorithms, extra guards and future tests below are proposals under
those labels unless expressly identified as inherited. Accepting this document would
freeze implementation choices; it would not supply a live UID, scan budget, rights,
retention rule, provider request, DB authority or runtime activation.

| Binding source | Requirement used here |
| --- | --- |
| [D5:519](2026-10-14-r2-daily-input-design-amendments.md#L519), [D5:528](2026-10-14-r2-daily-input-design-amendments.md#L528) | Snapshot signature, exact lines, verified metadata/file handles, existing shared lock, explicit installer-verified Raw UID, no-follow opens, no writes/reconciliation or euid substitution. |
| [D5:40](2026-10-14-r2-daily-input-design-amendments.md#L40), [D5:77](2026-10-14-r2-daily-input-design-amendments.md#L77) | Discover intervals independently of partition date; exact source identity; scan-limit failure never truncated success. |
| [D4:155](../contracts/owner-beta-daily-etf11-input-v1.md#L155), [D4:176](../contracts/owner-beta-daily-etf11-input-v1.md#L176) | Separate exact batch.json, manifest-line and provider-byte commitments; only LF is excluded from a line hash. |
| [D4:125](../contracts/owner-beta-daily-etf11-input-v1.md#L125), [D4:145](../contracts/owner-beta-daily-etf11-input-v1.md#L145) | Fixed final wire limits, checks before deserialization/output allocation, no truncation/saturation. |
| [D1:79](2026-10-14-r2-daily-input-design.md#L79), [D4:591](../contracts/owner-beta-daily-etf11-input-v1.md#L591) | Finite errors, no arbitrary strings or chained parser errors. |
| [D1:190](2026-10-14-r2-daily-input-design.md#L190), [D1:234](2026-10-14-r2-daily-input-design.md#L234) | Independent enumeration, bounded descriptor reads; later independent verification/copy/admission remains separate. |
| [D1:264](2026-10-14-r2-daily-input-design.md#L264), [D5:533](2026-10-14-r2-daily-input-design-amendments.md#L533) | Installer trust is external; proposed principal numbers are not observed assignments; do not chown Raw or relax access. |
| [D5:540](2026-10-14-r2-daily-input-design-amendments.md#L540), [D5:681](2026-10-14-r2-daily-input-design-amendments.md#L681) | Copy/normalization are later seams; registration has sequential ownership; preserve historical bytes/IDs. |
| [Wave20:2363](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L2363), [gates:2392](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L2392) | Report-only scope, frozen inputs, no execution/delegation, contract acceptance before source work. |

The complete 17-file freeze was checked with `sha256sum` and `wc -l`; every value
matches `target/r2-daily-beta/wp-4-integration/source-wave-20-S2D-inputs.json`.
That inventory itself hashes to
`610da261013457e4bb79d64e75255a0c449d98df3fa51b688e2128ecaaa41112`.

| Frozen reference | Lines | SHA-256 |
| --- | ---: | --- |
| crates/market-data/src/storage.rs | 3656 | b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef |
| crates/market-data/src/contract.rs | 398 | 27b0a4d71f55ea2fea89eb54b6fe92483f859f2ae59c608885415a1eb680d3d4 |
| crates/market-data/src/normalize.rs | 2019 | be5b4d3444b355c40295ca33bdc9d40e3aa1e359146fcaca293b4a1f745cd6b9 |
| crates/market-data/src/historical_price_only.rs | 1052 | d30e7a95bb695c180e47eedb6b11209dc7fceb598a65cc4b60a16456a48a4b52 |
| crates/market-data/src/historical_price_only_artifact.rs | 3924 | 60298769aa267a4cd32d7e0c3343b76f65fddd79a0b449bd1881d53408fb6065 |
| crates/market-data/src/daily_price_only/mod.rs | 8 | e143131e3bc23628179d6f2fe2bff0f0539b960f86d311b5ad9627f1610b2135 |
| crates/market-data/src/daily_price_only/contract.rs | 4183 | f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523 |
| crates/market-data/src/lib.rs | 288 | fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75 |
| crates/market-data/Cargo.toml | 36 | d2af9a324bf1e421c23f4c15b5768882f8b041502e351f7e1f1ac671c95a1997 |
| crates/market-data/tests/daily_price_only_storage.rs | 644 | 1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45 |
| docs/contracts/owner-beta-daily-etf11-input-v1.md (D4) | 950 | fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f |
| docs/reviews/2026-10-14-r2-daily-input-design.md (D1) | 926 | e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670 |
| docs/reviews/2026-10-14-r2-daily-input-design-amendments.md (D5) | 740 | 8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6 |
| docs/reviews/2026-10-14-r2-raw-manifest-evidence-review.md | 290 | e0322d1774dbd0124d1619f7f69be983dc43a035df8d71ac8ec324d5e43c089b |
| docs/reviews/2026-10-14-r2-raw-manifest-corrections-review.md | 199 | f65bb3e3a47f8cea39dfccd74656602639340249f56ddbc76881fc846eb5e88b |
| docs/reviews/2026-10-14-r2-caller-role-raw-ci.md | 26 | 989a3891493f3ece231de15050f6bb3d139f1034edf3c0fe87e4aef4b686a47c |
| Cargo.lock | 5609 | d3528661c8ad9d15d4cc7b6d4f4f91e530c851770b8a79751c2c2a680645783e |

One supplementary read-only source, the specific v3 reader, has 2487 lines and SHA
`9a604f95e9d5df92f1a54c3b3f23b8221513c19c1fc64f135c826947c6b02244`:
`crates/market-data/src/historical_price_only_v3_artifact.rs`. It is an architectural
comparison, not an additional accepted implementation dependency. None of the four
concurrent I0c files was read or pinned.

## 2. P1 — ownership and the exact API

**Proposal and rationale.** Own the new API and its private filesystem machinery in
`crates/market-data/src/daily_price_only/source.rs`. Use existing Raw data types, but
do not call or modify an existing RawStore read/write method. This avoids an unlock
gap, legacy behavior changes, and StoreError exhaustiveness changes. The accepted
[S1 API:1490](../../crates/market-data/src/storage.rs#L1490) explicitly ends its
shared lock at [storage.rs:1606](../../crates/market-data/src/storage.rs#L1606).
Wrapping that call and subsequently reading paths cannot satisfy D5 A6.

Public namespace: `market_data::daily_price_only::source`. The following is the
complete public signature inventory for this package; bodies are intentionally not
provided. Names of opaque types do not convey data rights or daily admission.

```rust
use std::path::Path;
use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
use crate::contract::{FetchMode, ResponseKind, SourceFileReference};

pub struct ApprovedRawScope { /* private fields specified below */ }
impl ApprovedRawScope {
    pub fn new(provider: &str, market: &str, trusted_raw_owner_uid: u32)
        -> Result<Self, DailyEvidenceError>;
    pub fn provider(&self) -> &'static str;
    pub fn market(&self) -> &'static str;
    pub fn trusted_raw_owner_uid(&self) -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanLimitValues {
    pub max_manifest_bytes: usize,
    pub max_manifest_records: usize,
    pub max_batches: usize,
    pub max_files_per_batch: usize,
    pub max_files: usize,
    pub max_batch_metadata_bytes: usize,
    pub max_source_file_bytes: usize,
    pub max_retained_bytes: usize,
    pub max_open_descriptors: usize,
    pub max_data_root_bytes: usize,
    pub max_data_root_components: usize,
    pub max_metadata_json_depth: usize,
}
pub struct ScanLimits { /* private values: ScanLimitValues */ }
impl ScanLimits {
    pub fn new(values: ScanLimitValues) -> Result<Self, DailyEvidenceError>;
    pub fn values(&self) -> ScanLimitValues;
}

pub struct CommittedRawReader<'root> { /* private data_root: &'root Path */ }
impl<'root> CommittedRawReader<'root> {
    pub fn new(data_root: &'root Path) -> Self;
    pub fn snapshot(&self, scope: ApprovedRawScope, limits: ScanLimits)
        -> Result<CommittedRawSnapshot, DailyEvidenceError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotStats {
    pub manifest_bytes: usize,
    pub manifest_records: usize,
    pub batches: usize,
    pub files: usize,
    pub retained_evidence_bytes: usize,
    pub open_descriptors: usize,
    pub peak_open_descriptors: usize,
}
pub struct CommittedRawSnapshot { /* private ownership specified below */ }
impl CommittedRawSnapshot {
    pub fn scope(&self) -> &ApprovedRawScope;
    pub fn limits(&self) -> &ScanLimits;
    pub fn stats(&self) -> SnapshotStats;
    pub fn manifest_bytes(&self) -> &[u8];
    pub fn manifest_sha256(&self) -> &ContentHash;
    pub fn batches(&self) -> &[CommittedRawBatch];
    pub fn lookup_exact(&self, reference: &SourceFileReference)
        -> Result<CommittedRawFile<'_>, DailyEvidenceError>;
}
pub struct CommittedRawBatch { /* private ownership specified below */ }
impl CommittedRawBatch {
    pub fn batch_id(&self) -> BatchId;
    pub fn batch_date(&self) -> TradingDate;
    pub fn retrieved_at(&self) -> UtcTimestamp;
    pub fn mode(&self) -> FetchMode;
    pub fn manifest_line_bytes(&self) -> &[u8];
    pub fn manifest_line_sha256(&self) -> &ContentHash;
    pub fn manifest_line_number(&self) -> usize;
    pub fn batch_json_bytes(&self) -> &[u8];
    pub fn batch_json_sha256(&self) -> &ContentHash;
    pub fn files(&self)
        -> impl ExactSizeIterator<Item = CommittedRawFile<'_>> + '_;
}
pub struct CommittedRawFile<'snapshot> {
    /* private batch: &'snapshot CommittedRawBatch, index: usize */
}
impl<'snapshot> CommittedRawFile<'snapshot> {
    pub fn batch(&self) -> &'snapshot CommittedRawBatch;
    pub fn file_name(&self) -> &'snapshot str;
    pub fn kind(&self) -> ResponseKind;
    pub fn content_hash(&self) -> &'snapshot ContentHash;
    pub fn size_bytes(&self) -> u64;
    pub fn bytes(&self) -> &'snapshot [u8];
}
// DailyEvidenceError and its complete methods are specified in section 7.
```

The `&self` receiver and borrowed data-root constructor supply the root omitted by
D5's interface shorthand; the two explicit snapshot arguments and return type remain
exactly D5's. `new(data_root)` performs no filesystem work or allocation. Snapshot
validates the root under supplied limits before cloning any component or opening `/`.
The returned value does not borrow the reader or the caller's Path.

Private representation, frozen by P1/P3 to remove descriptor ownership ambiguity:

| Type | Private fields and ownership |
| --- | --- |
| ApprovedRawScope | `provider: RawProvider` (private enum `Kis`, `KisCalendar`), `trusted_raw_owner_uid: u32`; market is the validated constant `kr`, not an arbitrary retained string. |
| ScanLimits | `values: ScanLimitValues`; construction is the only validation path. |
| CommittedRawSnapshot | Owned scope, limits, stats, one `Arc<Vec<u8>>` holding the complete captured manifest, its ContentHash, `Vec<CommittedRawBatch>` in first-occurrence order, a sorted `Vec<(BatchId, usize)>` lookup index, and a private resource guard. |
| CommittedRawBatch | One owned parsed `storage::ManifestEntry`; clone of the same manifest Arc; exact `Range<usize>` excluding LF, 1-based physical line number and line hash; one captured batch.json leaf; `Vec<CapturedLeaf>` in exact FileEntry order. |
| CapturedLeaf | Owned readonly `File`, owned `Vec<u8>` of captured bytes, ContentHash, private initial stat fingerprint, and parent-directory index plus validated basename needed for revalidation. Provider metadata is not cloned into this type. |
| Resource guard | Retained directory-descriptor graph from `/`, its parent/name edges and initial identities; owned manifest descriptor/fingerprint; a separate, single owned commit-lock descriptor/identity, closed last. No lock-descriptor clones. |
| CommittedRawFile | Borrowed batch plus validated index; references the aligned ManifestEntry FileEntry and CapturedLeaf. It owns no descriptor, bytes or independent lifetime. |

Linux-specific fields stay in a private `cfg(target_os = "linux")` representation. The public types
and error signatures remain available on other targets; snapshot returns
`UnsupportedPlatform` there. Do not expose `File`, `AsFd`, `AsRawFd`, a pathname,
`Read`, `Seek`, mmap, an `into_*` ownership extractor or mutable access. No public
Clone/Default/Serialize/Deserialize/Debug/Display implementation on reader, scope,
snapshot, batch or file views; no derived Debug on private data-bearing containers.
Byte slices are intentional evidence access, never implicit diagnostic formatting.

The only crate-visible metadata getters are:

```rust
impl CommittedRawBatch {
    pub(crate) fn entry(&self) -> &crate::storage::ManifestEntry;
}
impl<'snapshot> CommittedRawFile<'snapshot> {
    pub(crate) fn entry(&self) -> &'snapshot crate::storage::FileEntry;
}
```

They serve later in-crate discovery/verifier code, not logs. Existing ManifestEntry,
FileEntry, RequestMetadata and SourceFileReference can contain arbitrary submitted
strings and already derive Debug ([storage.rs:36](../../crates/market-data/src/storage.rs#L36),
[contract.rs:281](../../crates/market-data/src/contract.rs#L281)). Do not derive an error
or wrapper that recursively formats them, or use payload-bearing `assert_eq!` output
in redaction tests. Their existing definitions are unchanged.

## 3. P2 — scope, completeness and discovery

ApprovedRawScope::new accepts only provider literals `kis` or `kis-calendar`, market
`kr`, and an explicitly supplied UID other than `u32::MAX` (reserved invalid UID).
It allocates no submitted strings. Rejection is `InvalidScope`. UID zero has no
special bypass: if explicitly supplied it is compared exactly like every other UID;
this is not authority to execute as root. Never derive the production value from
euid, a username, a file being inspected, a fixture, or D1's proposed numeric IDs.
The provider/market set comes from [D4:161](../contracts/owner-beta-daily-etf11-input-v1.md#L161)
and [contract.rs:31](../../crates/market-data/src/contract.rs#L31); rejecting the
reserved UID is an additional P2 input guard.

Installer-verified inputs, supplied separately at later integration: trusted Raw
owner UID, absolute data root, protected ancestor/mount arrangement, and effective
read/traverse access for the actual consumer principal. Policy/deployment inputs:
every ScanLimitValues field, separately from scope. The constructor only validates
the supplied scope representation; it cannot attest who approved the UID or grant
entitlement. There is no rights reference, admission token or policy-hash assertion
in this type. No live values are supplied by this report.
Later runtime adapters must obtain root/scope/limits from their trusted installer or
release-policy configuration, never a producer candidate, provider metadata or an
untrusted request. A public Rust constructor validates representation, not that
configuration's provenance; implementing the runtime adapter is excluded here.

**Complete scope boundary.** One call captures the entire existing manifest for that
provider/market, from byte zero through the size observed after taking its shared
lock, with the S1 torn-tail rule below. No caller-selected batch list, partition-date
filter, byte-offset resume, requested civil interval, latest-pointer or cached index
may narrow this package's scan. Limits are failure boundaries, never selection
boundaries. An empty existing manifest produces an empty snapshot, not a missing-data
proof; missing controls fail. Complete duplicate records count before deduplication.

The full `manifest_sha256()` binds this scan's observed bytes for local comparison;
it is not a new D4 hash preimage, admission pin or global invalidation rule. In
particular, an ignored tail or unrelated appended evidence can change that hash
without changing an older batch's exact selected line/batch/file identity. Later
sealed-source checks use D5's relevance rules, not equality of whole-scope hashes.

`batches()` plus the crate-visible metadata getters are the interval-discovery seam:
later code enumerates every committed batch/file and validates request intervals
itself. This package does **not** label F_DT/T_DT strings as a verified interval,
select a chain, or parse provider bodies. A batch partitioned before an extension
must still be discoverable. This implements the storage prerequisite of
[D5:42](2026-10-14-r2-daily-input-design-amendments.md#L42), without moving A1's
provider/coverage rules into the storage primitive. No `discover(start,end)` method
that silently falls back to ManifestEntry.date is permitted.

`lookup_exact` does no filesystem work. It matches the reference's provider/market
to this snapshot, batch ID to the committed index, exact file name to exactly one
FileEntry, content hash and recorded batch retrieval instant to the reference.
Scope disagreement returns `ScopeMismatch`; absent committed batch/file returns
`NotCommitted`; differing hash/time returns `ReferenceMismatch`. Unsafe reference
names return `InvalidFileName` before searching. The date is taken from the committed
entry; SourceFileReference has no date field
([contract.rs:303](../../crates/market-data/src/contract.rs#L303)). Lookup never follows
copied_from, synthesizes a source, or accepts a caller's on-disk pathname.

Different scopes require different explicitly configured snapshots. This primitive
does not promise a cross-scope atomic cut. Later copy-chain orchestration must define
its own set/order/lifetime without an implicit cross-scope open here. There is no
new provider scope, acquisition permission or source priority.

## 4. P3 — byte, count, descriptor and work bounds

**Proposal and rationale.** Use explicit independent positive caps, checked before
the operation they protect. S1 already requires positive byte/record limits and
checked max+1 ([storage.rs:2257](../../crates/market-data/src/storage.rs#L2257)); the
new multi-file seam must additionally bound metadata, bodies and live descriptors.
No Default, environment fallback, unlimited special value, saturating arithmetic,
partial-success flag or implicit production budget exists.
These are per-call/per-snapshot limits. They do not bound other open descriptors,
other snapshots or total process concurrency; later runtime policy must account for
that separately rather than treating a successful local cap check as a host budget.

| ScanLimitValues field | Exact unit and enforcement point |
| --- | --- |
| max_manifest_bytes | All bytes of manifest.jsonl, including blank lines, LF, equal duplicates and ignored torn tail; stat before allocation; capture at most observed length plus a one-byte EOF probe. |
| max_manifest_records | Complete nonblank records before dedup, across the whole scope; LF-terminated record charged before typed parsing; complete unterminated final record charged before retention/dedup. |
| max_batches | Distinct committed batch IDs; charged before retaining a new entry/index or opening any of that batch's tree. |
| max_files_per_batch | FileEntry count in each attempted manifest/batch.json parse, checked by a bounded sequence visitor before materializing the next FileEntry. Equal record duplicates are not exempt from this parsing guard. |
| max_files | Sum of files in retained distinct batches; computed with checked addition before opening batch data or allocating its captured-leaf vector. It is not a per-session D4 count. |
| max_batch_metadata_bytes | Exact bytes of each batch.json; no aggregate JSON document parse beyond this cap. |
| max_source_file_bytes | Exact bytes of each provider file; must be at most D4's 67,108,864 bytes. Actual/recorded size must also be positive. |
| max_retained_bytes | Aggregate captured evidence byte lengths: full manifest once + every distinct batch.json + every retained provider file. Arc references/line ranges do not count the same manifest bytes again. Reserve this charge before allocating/capturing each byte buffer. |
| max_open_descriptors | All descriptors owned by this operation/snapshot, including `/`, ancestors, Raw/control/data directories, commit.lock, manifest, batch.json and provider files; reserve before every open or dup. No uncharged transient descriptors. |
| max_data_root_bytes | OS path bytes on Unix, checked before making an owned name/path buffer. It does not cap provider metadata strings by pretending they are paths. |
| max_data_root_components | Number of normal components between `/` and the supplied data root; checked before descriptor graph/name allocation or traversal. |
| max_metadata_json_depth | Container depth of Raw manifest-line and batch.json parsing, root=1; positive and at most 32. This added Raw parser guard is P3; D4 independently fixes 32 for its own wire documents. |

ScanLimits::new rejects zero fields and policy-invalid values as `InvalidLimits`.
It rejects max_source_file_bytes > 67,108,864 or max_metadata_json_depth > 32;
these cannot be made permissive by a larger storage configuration. It verifies
byte-limit +1 and count +1 representability and byte capacities <= isize::MAX;
checked-conversion or arithmetic failure is `ArithmeticOverflow`. Products/sums
needed for vector layouts, node counts, offsets and each actual read are checked
again before use. Small mutually restrictive caps are legal: a call may then fail
before opening its first descriptor. Do not silently increase one cap to fit another.

Equality to a maximum succeeds when every other guard succeeds. An observed byte,
record, batch, file, depth, root-path or descriptor count above its configured cap
returns `SourceScanLimit` (Display/code `SOURCE_SCAN_LIMIT`), with no partial snapshot.
An actual negative/nonrepresentable stat size, or size differing from committed size,
is `SizeMismatch`; an otherwise representable size above a cap is SourceScanLimit.
Allocation reservation failure is `AllocationFailed`; no size appears in its error.

Use checked_add/checked_mul/try_from for counters, capacity estimates, read offsets,
stat sizes and usize/u64 conversions. Do not use `as` to narrow, saturating operations,
unchecked `max + 1`, `read_to_end`, collecting an unbounded iterator, or reserve from
untrusted `size_hint`. Explicit Vec growth uses fallible reservation after the count
and byte guards. Bound all read slices by the previously checked remaining length.
Use fixed-size scratch buffers (8 KiB for reads and 4 KiB for directory entries) plus
a one-byte stack probe; these are implementation scratch sizes, not acquisition
budgets. A zero-byte read before the expected length is SizeMismatch. A byte after
the expected length is SourceChanged (or SourceScanLimit if it exceeds a byte cap).
Return `Io` on interrupted/failed I/O; this package contains no unbounded EINTR retry.

The aggregate limit is explicitly a **retained evidence-byte budget**, not a claim
about allocator overhead, total RSS or globally recoverable OOM. Parsed metadata,
indexes, descriptor tables and allocator slack also occupy memory. They are bounded
by manifest/batch byte caps, depth, record/file/root-component counts and checked
vector capacities; they are not provider bodies hidden outside the aggregate charge.
Keep one parsed ManifestEntry per unique batch, release duplicate/temporary metadata
immediately, retain no serde Value tree and clone no metadata into file handles.
Deserializing a bounded input can still allocate inside serde/domain types. As with
the accepted [S1 review:116](2026-10-14-r2-raw-manifest-corrections-review.md#L116),
do not claim a hard process-memory or fully fallible-allocation guarantee.

At construction, the full manifest byte buffer is already charged and becomes the
shared immutable Vec without copying it (`Arc::new(vec)`). Exact lines are ranges.
Batch metadata capture is charged before allocation; its temporary parsed copy is
discarded after equality against the manifest entry. Captured provider bytes remain
charged until failure cleanup or snapshot Drop. The final recheck compares streamed
descriptor bytes against the captured slices; it allocates no second body buffer.
Return stats describe actual evidence lengths and descriptor high water, never policy
maxima masquerading as measured usage.

**Descriptor accounting.** Retain each distinct directory once. Let C be normal
data-root components, D distinct dates among committed batches, B distinct batches,
and F their total files. With at least one batch, retained descriptors are exactly
`C + 9 + D + 2*B + F`: `/` + C ancestors; raw; three manifest directories; two data
scope directories; two controls; D date directories; B batch directories; B metadata
files; F provider files. With no batches they are `C + 7`, and data-scope directories
need not exist. All arithmetic is checked. Reserve the next descriptor before each
open; preflight the computable minimum before opening `/`. Directory enumeration
borrows the held descriptor with rustix RawDir and rewinds it, so it needs no dup.
If an implementation needs a temporary descriptor, it must charge it and return to
the coordinator to amend this exact ownership/formula, rather than hide the extra FD.

Batch enumeration is bounded by expected files + batch.json + at most two dot
entries. Compare names from the fixed directory buffer directly to the expected set;
do not allocate a list of arbitrary directory entries. The first unexpected or
duplicate entry fails, including directories/symlinks and leftover temporary files.
No enumeration of date/provider directories is needed to discover batches.

**Storage caps versus D4 caps.** max_manifest_bytes here caps the append-only Raw
log, not D4's 16 MiB *daily artifact manifest*. max_batch_metadata_bytes is also a
new storage cap, not max_policy_bytes or max_normalized_document_bytes. Storage
batches/files/records are not D4 sessions, civil dates, chains or per-session evidence.
The fixed D4 limits remain: policy 262144; final manifest/normalization 16777216 each;
admission 1048576; bars 134217728; sessions 20000; civil dates/addition 370;
chains/session 32768; file evidence/session 262144; source/document bytes 67108864;
copy depth 16; pages/chain 10; cash rows/chain 100000; JSON depth 32
([D4:127](../contracts/owner-beta-daily-etf11-input-v1.md#L127)). Later D4 conversion
and verification must enforce all of these again. Passing a larger Raw scan cap
does not change any final file/document cap or permit larger acquisition.

**Synthetic values only.** A small fixture may explicitly use manifest=65536 bytes,
records=8, batches=4, files/batch=4, files=16, batch.json=16384 bytes, body=8192 bytes,
aggregate=262144 bytes, descriptors=96, root bytes=4096, root components=32, depth=32.
The byte maxima satisfy 65536 + 4*16384 + 16*8192 = 262144. A pure stat fixture may
use trusted UID 4242 and wrong UID 4243. A real temporary-filesystem test instead
supplies the owner of its own fresh synthetic fixture explicitly; it cannot assume
4242 owns hosted files. These are neither installed principals nor production budgets.

## 5. P4 — complete manifest and metadata handling

**Inherited parsing semantics (B), implementation proposal (P4).** The new bounded
parser must match the accepted S1 algorithm at
[storage.rs:2318](../../crates/market-data/src/storage.rs#L2318), independently checked
by parity fixtures. Keep S1 source byte-identical; do not refactor its private parser
as part of this package. A new local bounded ManifestEntry deserialization seed may
enforce file count/depth before materialization without changing the existing type.

Make the bound mechanism concrete: a nonallocating lexical container-depth pass
over each already byte-bounded record ignores braces/brackets inside JSON strings
and tracks backslash escapes; it enforces only the configured depth. It does not
classify JSON validity or decide the EOF-tail rule. A typed ManifestEntry seed then
uses a bounded files-sequence seed, checking the next element's count before that
element is deserialized. Known nested types keep their existing Deserialize behavior;
the byte cap bounds their strings/query/header allocations. Typed serde parsing alone
decides EOF/non-EOF, after any explicit resource failure. No Value tree or new public
metadata parser is introduced. Temporary duplicate/final-tail parsing is bounded by
the input, depth and per-batch file guards; total distinct-file/batch caps apply before
moving a parsed entry into retained state or opening its data tree.

1. Scan the captured buffer once for LF boundaries; preserve physical line numbers.
   Ignore only ASCII-whitespace-only lines. CR, spaces, key order, and unknown Raw
   metadata bytes remain in the retained buffer and exact hash slices.
2. Charge a nonblank LF-terminated record before parsing, even if malformed. A final
   unterminated record is accepted if typed parsing succeeds, then charged before
   deduplication. Ignore it only when the actual serde error class is EOF. A terminated
   EOF error, non-EOF typed error or other malformed tail is InvalidManifest.
3. A final unterminated candidate must be parsed to distinguish complete from torn;
   this is one byte/depth/file-bounded lookahead, not permission to retain a record
   over the cap. If already at the record cap, a complete final record fails, while
   genuine serde EOF may still be ignored. A resource-guard failure is never
   relabeled EOF and cannot be ignored. All tail bytes still consume the byte cap.
4. Require exact parsed provider/market equality to scope. Check safe file names.
   Compare repeated BatchIds using full ManifestEntry PartialEq, not a semantic hash
   or a reserialization; equal repeats retain the first line/hash/physical position
   and first-occurrence order, unequal repeats return ManifestConflict.
5. Charge distinct batches/files and validate all metadata-only constraints before
   opening their data paths. Use bounded vectors and a sorted batch-ID index for
   lookup; do not make traversal order the evidence selection rule.

The underlying Raw serde schema stays the one at
[storage.rs:36](../../crates/market-data/src/storage.rs#L36): optional continuation and
copied_from retain existing defaults; known duplicate fields/type failures retain
serde rejection behavior; unknown Raw fields remain ignored in parsed equality but
are protected by exact byte hashes. The new seed must not use a lossy Value map or
apply D4's new wire schema to old Raw metadata. Depth/count rejection is the explicit
new storage guard. Test omitted optional fields and unknown-field bytes for parity.
D4's canonical-key/duplicate/unknown-field rules remain mandatory when later producing
or reading D4 wire values ([D4:22](../contracts/owner-beta-daily-etf11-input-v1.md#L22)).

For every retained batch, open its exact derived
`raw/provider=<p>/market=<m>/date=<entry.date>/batch=<entry.batch_id>/batch.json`
and every listed file. This is the current layout
([storage.rs:644](../../crates/market-data/src/storage.rs#L644)). Read bounded exact
batch.json bytes; deserialize to ManifestEntry with the same bounded Raw rules; require
full equality to the committed parsed entry, including files/order, request metadata,
mode, entitlement reference and recorded time. Hash the exact batch.json independently
of its manifest line. This equality follows
[storage.rs:2028](../../crates/market-data/src/storage.rs#L2028); canonical/prettified
byte equality between JSONL and batch.json is neither required nor correct.

Every listed filename must satisfy the existing plain-name rules
([storage.rs:2607](../../crates/market-data/src/storage.rs#L2607)); additionally reject
the reserved `batch.json` name and duplicate file names, even identical duplicates.
These additional P4 guards prevent aliasing the metadata or an ambiguous lookup.
Different basenames with equal content hashes are allowed storage facts; repeated
provider page bytes are a later provider-validation question. Empty file lists may
be represented as committed empty batches; they do not prove required evidence.

The snapshot does not certify D4 filename/UUID/timestamp lexical validity merely by
parsing a legacy domain type. D4's stricter 128-byte ASCII filename and nonnil UUID
rules apply during later adaptation ([D4:45](../contracts/owner-beta-daily-etf11-input-v1.md#L45));
storage can enumerate an otherwise safe older name that that later adapter rejects.
No downstream caller may treat a snapshot as a bypass of those checks.

For each provider file, recorded size must be 1..max_source_file_bytes; stat and
captured lengths must equal it; hash exact captured bytes and compare to the committed
FileEntry content_hash. Do not parse its JSON, validate rt_cd, select kinds, enforce
KSD pagination, normalize, validate entitlement text, or follow copied_from here.
FetchMode::Synthetic remains representable storage evidence for provider-free fixtures;
there is no production-admission exemption. A successful snapshot can contain an
opaque invalid provider body whose size/hash are internally consistent.

No current-time read, new batch UUID, normalization ID, filesystem traversal order,
mtime or ctime enters an evidence hash. Stat timestamps are mutation guards only.
Recorded retrieval time is preserved from the committed entry; the snapshot neither
invents per-request precision nor validates that time against a wall clock. This
keeps the deterministic/raw-time boundary in D5 A6 and D4 Origin unchanged.

Only manifest membership makes a batch eligible. Uncommitted orphan/partial directories
are not visited, repaired, reconciled, or included; a complete committed line naming
missing/torn batch.json or a missing/partial file fails the entire snapshot. A torn
manifest tail does not cause its alleged batch to be opened. Existing reconciliation
at [storage.rs:1773](../../crates/market-data/src/storage.rs#L1773) is not used.

## 6. P5/P6 — lock lifetime, identity and atomicity

**P5 decision: both captured immutable bytes and retained readonly descriptors.**
The snapshot owns the shared commit lock from before manifest capture through all
batch/file checks, return, every borrowed consumer view, and Drop. Never unlock at
return, reacquire after S1, reopen a provider file by pathname for a consumer, or
return a live descriptor reader. Captured bytes are the sole consumer data source.
Descriptors retain object identity and support construction-time validation; POSIX
readonly descriptors themselves do not make a writable inode immutable.

**P6 decision: one nonblocking shared-lock attempt.** `fs2::FileExt::try_lock_shared`
on the already-existing verified commit.lock returns LockBusy when a cooperating
writer holds it. Other lock errors are PermissionDenied/Io. There is no wait, retry,
timeout budget, sleeping or polling in the primitive. This preserves shared-lock
mutual exclusion while making work bounded; S1's blocking behavior is untouched.
The caller later decides when to retry. Regular-file reads can still stall in the OS;
this is not a hard wall-clock deadline or protection against a hostile filesystem.

Construction is an all-or-nothing private state machine:

1. Validate limits and scope, then lexical data-root bytes/components. Require absolute
   path without parent components or NUL; preserve S1's acceptance of benign lexical
   current-directory normalization, but never resolve symlinks. Do not call canonicalize.
2. Open `/` and walk every component descriptor-relatively. Use
   `RDONLY|DIRECTORY|NOFOLLOW|CLOEXEC` for directories, and
   `RDONLY|NOFOLLOW|CLOEXEC|NONBLOCK` for regular leaves. Use pre-open statat with
   SYMLINK_NOFOLLOW, openat, fstat, then named statat to bind the object. Reject a
   symlink/non-directory at every level and a nonregular leaf before reading it.
   Pre-stat prevents opening a known FIFO/device; NONBLOCK prevents FIFO wait if its
   type changes in the open race. A symlink observed by pre-stat is UnsafePath,
   independent of whether openat would return ELOOP. No CREATE, write, mkdir, chmod,
   chown or fsync.
3. Retain all parent descriptors and names. Apply exact trusted UID and no group/other
   write (`mode & 0o022 == 0`) from the appended raw directory downward, including both
   manifest/data branches, every date/batch directory and every regular leaf. Owner
   write is permitted for compatibility with the existing immutable-write convention.
   Outside ancestors are safely walked and revalidated, not assumed to have Raw UID.
4. Open existing commit.lock, require regular type and link count one, validate its
   parent/name/descriptor identity, take the shared lock, then revalidate the entire
   control path and lock before opening manifest.jsonl. A replaced lock is never
   retried by adopting its new inode. Missing lock/manifest/tree is MissingControl.
5. Open/bound/read the manifest through its verified descriptor; revalidate before
   parsing. Capture and hash its complete observed bytes, including any ignored tail.
   Enumerate retained committed entries using section 5. Empty manifest is legal.
6. For each retained entry open only its derived data directories and batch.json;
   check equality and exact directory membership; open/capture every listed body.
   Account resources before all opens/allocations. Preserve original bytes and times.
7. While still holding the same lock, perform a second bounded descriptor-byte
   comparison for manifest, every batch.json and provider file against captured bytes.
   Bracket each comparison by descriptor and named stat checks. Re-enumerate each
   batch's exact leaf set. Revalidate every retained directory edge, every leaf and
   the still-named lock immediately before constructing the successful return value.
   No retries on changed identity/content and no successful prefix on any failure.
8. Transfer all owned resources into the snapshot without dropping/unlocking the
   guard. Failure drops partial captures/handles and closes the lock last. On normal
   Drop, close provider/batch/manifest and directory descriptors, then the sole lock
   descriptor last; close releases its advisory lock. Do not clone that descriptor
   or let borrowed views outlive the snapshot. Drop emits no log, panics, writes or
   purported successful revalidation; closing cannot report an error to the caller.

The existing writer writes new bodies before exclusive locking, then publishes final
metadata and appends the manifest under that lock
([storage.rs:1216](../../crates/market-data/src/storage.rs#L1216),
[storage.rs:1258](../../crates/market-data/src/storage.rs#L1258)). Thus a cooperating
writer may prepare an uncommitted new directory while a snapshot exists, but cannot
publish another committed entry until Drop. Do not reject harmless parent-directory
mtime/link-count changes caused by such preparation.

Initial and revalidation checks are precisely:

| Object | Required stable properties and policy |
| --- | --- |
| Every directory edge, including outside ancestors | Parent/name remains a directory with the held descriptor's dev/inode; retain and compare type, uid and permission mode to initial values. From raw downward also require the explicit Raw UID and no 022. Do not compare directory size/nlink/mtime/ctime, which can change from unrelated child creation. |
| commit.lock | Same held and named regular dev/inode, exact trusted UID, unchanged permission mode, no 022, nlink=1. Lock content/length/mtime are not evidence and need no zero-length assumption. Never read lock content. |
| manifest, batch.json, provider file | Same named and held dev/inode, type regular, exact trusted UID, unchanged mode with no 022, nlink=1, unchanged nonnegative size, mtime and ctime including subsecond fields. Exclude atime. Check both fstat and statat against the initial fingerprint. |
| Captured evidence bytes | Initial exact hash/size and final descriptor-byte equality; manifest/batch hashes are their own commitments, provider hashes must equal FileEntry. |

These extend S1's descriptor/owner checks at
[storage.rs:521](../../crates/market-data/src/storage.rs#L521) and complete path walk at
[storage.rs:888](../../crates/market-data/src/storage.rs#L888). Hardlink rejection,
retention of every ancestor and mtime/ctime comparison are explicit P5 additions.
The historical readers demonstrate related descriptor/link-count practices
([v2:1235](../../crates/market-data/src/historical_price_only_artifact.rs#L1235),
[v2:1311](../../crates/market-data/src/historical_price_only_artifact.rs#L1311),
[v3:1315](../../crates/market-data/src/historical_price_only_v3_artifact.rs#L1315));
do not reuse their euid or exact-0600 policies. Their implementation stays unchanged.

**What this proves, and the explicit trusted-writer assumption.** Under the existing
immutable-write protocol, stable path binding and shared lock define one committed
manifest view; every returned byte was captured from a matching committed file and
remains immutable in this snapshot's memory. A cooperating append waits for Drop.
Observed replacements, link-count changes, content/size changes and fingerprint
changes during construction reject the snapshot. A descriptor cannot be redirected
by renaming its path, and a consumer never reopens the new pathname.

An advisory lock does not stop an uncooperative process holding a writable FD, a
trusted UID changing committed files, a privileged process, mutable mounts or an
adversarial filesystem. Stat/byte rechecks are sequential, not a kernel multi-file
transaction; an ABA replacement or coordinated mutation/restoration between checks
cannot be universally excluded. Hashes prove byte consistency, not who authorized
the content. nlink=1 does not prove absence of an already-open writable descriptor.
No test should claim otherwise. The installer/trusted owner must preserve existing
committed files and lock identity, use the existing commit protocol, and maintain
protected path/mount/access arrangements. This dependency is visible, not inferred
from owner-mode bits or the word Approved. D5's trust boundary requires it; a hostile
trusted writer would require a different storage capability outside this assignment.

After return, a malicious/uncooperative path mutation cannot change captured bytes,
but the API does not continuously re-attest path existence. Later sealed-source
verification takes a fresh snapshot and exact lookup; a previous in-memory snapshot
does not prove current availability. Intentional reads may update filesystem atime;
"read-only" means no application mutation syscall or reconciliation, not frozen
kernel access-time accounting. Holding the lock indefinitely can delay the writer;
the caller owns prompt Drop, and no elapsed-time policy is invented here.

## 7. P7 — finite redacted errors, no StoreError growth

**Proposal and rationale.** Define the following new, fieldless enum in source.rs.
It is the DailyEvidenceError at the D5 seam, initially snapshot-specific. Do not add
StoreError variants or route failures through StoreError. There is no From<StoreError>
and no boxed/source error. This prevents a repeat of the three exhaustive caller
match failures recorded at [plan:1996](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1996)
and [ROADMAP:504](../ROADMAP.md#L504). pipeline.rs and worker.rs are not this package's
files, and wildcard weakening is not an integration strategy.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DailyEvidenceError {
    UnsupportedPlatform, InvalidScope, InvalidLimits, ArithmeticOverflow,
    SourceScanLimit, AllocationFailed, LockBusy, PermissionDenied, Io,
    MissingControl, MissingCommittedEvidence, UnsafePath, UntrustedOwner,
    WritableNode, NonRegularFile, HardLinkedFile, SourceChanged,
    InvalidManifest, ScopeMismatch, ManifestConflict, InvalidBatchMetadata,
    BatchMetadataMismatch, InvalidFileName, DuplicateFile, UnexpectedBatchEntry,
    SizeMismatch, HashMismatch, NotCommitted, ReferenceMismatch,
}
impl DailyEvidenceError {
    pub const fn code(self) -> &'static str;
}
impl std::fmt::Display for DailyEvidenceError { /* writes code() only */ }
impl std::error::Error for DailyEvidenceError {
    // source() always None, including parser/domain/OS/lock failures.
}
```

`code()` is the exact SCREAMING_SNAKE_CASE conversion of each listed variant, including
`SOURCE_SCAN_LIMIT`, `IO`, `NOT_COMMITTED`. Display equals code; derived Debug is the
fieldless variant name. No paths, line numbers, UIDs, hashes, expected/actual sizes,
serde errors, errno numbers, free-form reasons, provider payloads or metadata are
retained in the error. A successfully returned snapshot has explicit data getters;
that does not make its metadata safe to log. No tracing/logging occurs in this module.

| Condition | Error |
| --- | --- |
| ELOOP or ENOTDIR during guarded path traversal; observed symlink; lexical parent/relative/invalid path | UnsafePath |
| Wrong owner; forbidden 022; regular leaf has another type; nlink != 1 | UntrustedOwner; WritableNode; NonRegularFile; HardLinkedFile, respectively |
| ENOENT opening required controls/ancestors before lock-bound construction | MissingControl |
| ENOENT for a batch directory, batch.json or listed body named by a committed entry | MissingCommittedEvidence |
| Lost/changed binding after an object was captured, including later ENOENT | SourceChanged |
| EACCES/EPERM | PermissionDenied; no fallback even when the process could run privileged |
| Busy nonblocking lock | LockBusy |
| Other read/open/stat/lock/enumeration failure, including interruption | Io |
| Invalid complete Raw line; invalid batch.json JSON/type | InvalidManifest; InvalidBatchMetadata |
| Manifest provider/market disagreement; conflicting repeated BatchId | ScopeMismatch; ManifestConflict |
| Parsed batch.json differs from committed entry | BatchMetadataMismatch |
| Unsafe/reserved name; repeated listed filename; unexpected/duplicate enumerated entry | InvalidFileName; DuplicateFile; UnexpectedBatchEntry, respectively |
| Expected leaf absent at the end of initial exact-membership enumeration | MissingCommittedEvidence; if it was previously captured, SourceChanged |
| Recorded zero/negative or unequal stat/captured size; unequal committed provider hash | SizeMismatch; HashMismatch |
| Positive configured cap exceeded; checked arithmetic failure; fallible reservation failure | SourceScanLimit; ArithmeticOverflow; AllocationFailed |

Consult only serde error classification (`is_eof`) before discarding a final-tail
error. Classify OS errors without formatting them. A private bounded-parser guard
uses a typed side channel for depth/file-limit failure; never parse an error string
to distinguish it from malformed JSON. Error precedence for multiple simultaneous
faults is not a public promise; isolated single-fault tests assert the precise enum,
and multifault tests assert rejection. Construction failure never exposes partial
metadata. This extends the verified correction at
[storage.rs:2354](../../crates/market-data/src/storage.rs#L2354) and its
[sentinel assertions:123](../../crates/market-data/tests/daily_price_only_storage.rs#L123).

## 8. P8 — exact implementation ownership and sequence

One worker can implement the complete capability after this contract is accepted.
Further architecture decomposition is not required for this primitive. Do not split
manifest-lock acquisition and body validation into separately usable capabilities:
that would recreate the unlock/reopen boundary being removed. Tests and private
fault-injection hooks belong to the same worker as the implementation.

| Sequence/owner | Exact future writable allowlist | Deliverable |
| --- | --- | --- |
| S2 implementation worker | `crates/market-data/src/daily_price_only/source.rs` (new); `crates/market-data/tests/daily_price_only_snapshot.rs` (new) | Complete API, bounded parser/read/lock/resource machinery, private cfg(test) hooks and meaningful tests. No other files. |
| Sequential registration owner: coordinator or existing I1a owner, only after both files exist and are recovered | `crates/market-data/src/daily_price_only/mod.rs` | Add `pub mod source;`; adjust module-level prose to distinguish pure unapproved wire from snapshot storage evidence; retain `mod contract; pub use contract::*;` unchanged. No root re-export of snapshot types. |
| Coordinator after source review/hosted evidence | Existing roadmap/status/plan/review records under its own authority | Acceptance, immutable input freeze, exact candidate, later verification evidence; not assigned to the source worker. |

The new snapshot test filename is an explicit P8 narrowing/partition of D5's wider
I1b test ownership, leaving accepted S1 tests unchanged. All code stays inside the
already registered market-data crate and daily module
([lib.rs:47](../../crates/market-data/src/lib.rs#L47),
[mod.rs:6](../../crates/market-data/src/daily_price_only/mod.rs#L6)). No Cargo.toml,
Cargo.lock, dependency, provider/normalizer, pipeline/worker, existing storage source,
historical reader, wire contract or test change is necessary. The existing rustix/fs2/
serde/hash dependencies suffice ([Cargo.toml:14](../../crates/market-data/Cargo.toml#L14),
[fs2:21](../../crates/market-data/Cargo.toml#L21),
[Unix dependency:28](../../crates/market-data/Cargo.toml#L28)). No workflow or validator
file belongs to this package. If a required API cannot be implemented with these
dependencies, report the concrete gap; do not install, change versions or broaden scope.

Do not add module stubs ahead of files or overlap registration edits. No commands,
requests or instructions have been sent to another worker by this report. The
concurrent I0c targets remain excluded: `crates/job-queue/tests/owner_beta_publish.rs`,
`crates/api-server/tests/http_owner_beta_access.rs`, and
`deploy/db/integration-validation/{validate,static-check}.sh`.

Purely internal module registration adds no workspace/package/runtime/API/DB edge.
No architecture diagram edit/render is proposed here. If later integration changes
such an edge, the coordinator owns the evidence-bound diagram update before main
integration, under [AGENTS.md:251](../../AGENTS.md#L251); do not draw this proposal as
an implemented edge or run Docker for this assignment.

## 9. P9 — meaningful fixtures and guard-to-test acceptance matrix

All rows below are **required future evidence, not tests executed in this assignment**.
Use fresh synthetic temporary trees, explicit scope/UID/caps and actual batch.json
plus files. S1's manifest-only fixture has no body files
([daily_price_only_storage.rs:24](../../crates/market-data/tests/daily_price_only_storage.rs#L24));
reusing it unchanged would fail before reaching most new guards. Tests must establish
their valid control case before introducing the one intended fault. Failure at an
earlier guard is not evidence for the claimed guard.

Private cfg(test) hooks in source.rs identify exact phases: after lock open, after
shared lock, after manifest capture, after metadata capture, after a body capture,
before final byte comparison, and before final named checks. Hooks are unavailable
to production callers; use barriers or synchronous mutation, never sleeps to guess
a race. Pure private stat/budget helpers can accept synthetic stats/counters in unit
tests; they must not introduce a public bypass for UID/path checks.

| ID | Guard / meaningful fixture | Required assertion |
| --- | --- | --- |
| S2-T01 | Valid two-batch, multi-file snapshot; pretty batch.json versus compact/noncanonical-key JSONL | All bytes/hashes/sizes, parsed equality, first-order batches, borrowed file identity and measured stats; no normalization or writes. |
| S2-T02 | CRLF, leading/trailing spaces, blank lines and reordered object keys | Exact line byte/hash excluding only LF; physical line numbers; full manifest hash includes all original bytes. Parity with S1 on common valid fixtures. |
| S2-T03 | Complete unterminated final record; torn serde-EOF tail; terminated torn row; non-EOF invalid typed tail | First accepted/charged; second ignored but byte-charged; latter two rejected. No alleged torn-tail batch open. |
| S2-T04 | Equal parsed duplicate with different JSON whitespace; conflict in one metadata field | Keep first exact line and order; count both records; conflicting batch fails entire snapshot. |
| S2-T05 | Exact and +1 manifest byte/pre-dedup record caps, including final unterminated complete record | At-cap success, one-over SourceScanLimit, duplicate count cannot evade it; truncated success impossible. |
| S2-T06 | Each zero limit; max+1, multiplication/conversion overflow; too-large body/depth configuration | InvalidLimits/ArithmeticOverflow before any filesystem hook; no created control or tree. Test actual checked helpers rather than allocating huge buffers. |
| S2-T07 | Exact/+1 batch.json and body byte caps; cap crossed by torn-tail bytes | SourceScanLimit before corresponding buffer allocation/read; actual read instrumentation never exceeds checked expected length + one probe. |
| S2-T08 | Per-batch file count and unique batch/total-file caps; many small empty metadata fields | Reject before over-limit element retention or corresponding open; bounded deserializer cap is exercised, not only a post-parse len check. |
| S2-T09 | Aggregate where each individual file fits but their sum exceeds max_retained_bytes | Exact aggregate succeeds, next capture rejected before allocation; duplicates/Arc lines counted once as specified; error releases all charged resources. |
| S2-T10 | Descriptor cap one below/exactly required, including deep root and repeated date sharing | Exact high water and formula; rejection before forbidden open; no temporary dup leakage; no lock FD clone. |
| S2-T11 | Root-byte/component and JSON-depth exact/+1 boundaries | Rejection before owned component traversal/deeper parse work; no unbounded skipped unknown-field nesting. |
| S2-T12 | Missing Raw/control parent, missing manifest or missing commit.lock | MissingControl, no mkdir/lock creation/reconciliation; empty existing manifest instead returns empty snapshot. |
| S2-T13 | Relative/parent roots, ancestor/Raw/control/data/date/batch symlinks, leaf symlink, ordinary file directory component | UnsafePath; no canonicalize fallback; tests reach each distinct path level. |
| S2-T14 | FIFO/directory/synthetic special leaf; wrong expected UID; group/other writable directory/leaf; pure stat UID zero cases | Typed type/owner/mode errors without blocking; no root bypass. Pure stat checks do not claim actual cross-UID access proof. |
| S2-T15 | Hardlinked lock, manifest, batch.json, provider file | HardLinkedFile before acceptance. Same bytes in distinct independent files remain valid storage facts. |
| S2-T16 | Wrong provider/market in committed row; wrong batch.json date/ID/files/order/request/mode/time | ScopeMismatch or BatchMetadataMismatch as appropriate; equality is complete, not selected-field or serialization equality. |
| S2-T17 | Duplicate identical listed names, reserved batch.json, traversal names; extra leaf/temp/subdirectory | DuplicateFile/InvalidFileName/UnexpectedBatchEntry; exact membership rechecked at end. |
| S2-T18 | Missing/torn committed metadata, missing body, truncation, recorded zero size, same-size corruption | Correct missing/invalid/size/hash rejection; no batch skipped on error. |
| S2-T19 | Complete uncommitted orphan plus incomplete uncommitted directory | Neither is enumerated or exposed; snapshot does not repair/append. Compare all fixture content/modes/inodes before/after, excluding atime. |
| S2-T20 | Batch partition earlier than F_DT/T_DT overlap; unrelated partition ordering | All committed metadata available to later discovery; no date-based exclusion or claim that its query has already been validated. |
| S2-T21 | lookup_exact hit, missing batch/file, mismatched scope/hash/retrieval instant; copied reference naming absent source | Exact hit uses retained bytes; precise lookup errors; no implicit copy-source read or added scope. |
| S2-T22 | A fresh separately opened writer lock and two simultaneous snapshot readers | Shared readers coexist; writer's nonblocking exclusive lock fails until both snapshots drop, then succeeds. Do not use dup of the same open file description as the competitor. |
| S2-T23 | Writer holds exclusive lock before snapshot; change lock pathname before/after shared acquisition | LockBusy for ordinary contention; SourceChanged for captured inode replacement; replacement never adopted; no partial snapshot. |
| S2-T24 | Replace manifest, control directory, Raw ancestor, batch directory, metadata or body at deterministic capture/final hooks | SourceChanged at observed changed binding, including same-length/same-content replacement. No path reopen yields replacement bytes. |
| S2-T25 | In-place manifest/metadata/body mutation after capture; chmod/owner/link changes via synthetic stat seam | Changed fingerprint/final bytes reject; mutation-and-restore beyond observable checks is explicitly outside the proof. |
| S2-T26 | Append attempted through a separate cooperating writer while snapshot exists | Cannot commit until Drop; next fresh snapshot sees appended complete record. No claim about an uncooperative direct append. |
| S2-T27 | Mutate fixture file after successful return, deliberately violating trusted-owner convention | Existing bytes()/hash remain captured; no continuous on-disk validity claim; fresh snapshot rejects inconsistency. |
| S2-T28 | Every construction-stage injected failure and normal Drop | Retained FD count returns to baseline, competing writer can lock, no created files or modified evidence; no body/path diagnostics. |
| S2-T29 | Sentinels in malformed mode/hash/UUID, filename, entitlement/request/copy metadata and synthetic OS failure | Display/code/Debug contain only finite codes; source() is None; no sentinel through a wrapped error or logging path. |
| S2-T30 | Bounded unknown Raw fields, omitted optional continuation/copy fields and invalid provider JSON with correct storage hash | Raw parse parity preserved; provider body accepted as opaque storage bytes; none is labeled verified provider/daily evidence. |
| S2-T31 | Every non-Linux target, including other Unix targets | Same public signatures, UnsupportedPlatform without I/O; report unexecuted platform cases explicitly. |
| S2-T32 | Real temporary-root ownership with different expected UID plus readable/writable ancestor case | Explicit Raw UID enforcement, S1 outside-ancestor policy preserved, denied permission never repaired. Actual cross-principal mounts/ACLs remain a separate later gate. |

The old private race tests prove only their named observation points
([storage.rs:3591](../../crates/market-data/src/storage.rs#L3591),
[storage.rs:3622](../../crates/market-data/src/storage.rs#L3622)). The accepted
[S1 review matrix:122](2026-10-14-r2-raw-manifest-corrections-review.md#L122) explicitly
records absent post-read/lock-lifetime and cross-UID proof. New race rows above close
specific coverage gaps; they cannot prove immunity to a malicious trusted writer.

Preservation checks for the future coordinator/reviewer: byte-identical accepted
storage.rs, contract.rs, normalize.rs, historical v2/v3 sources, D4/D5, pure wire
contract/tests and Cargo inputs; no change to existing writer/read_committed_manifest/
orphan reconciliation or S1 tests; no new StoreError variant/match wildcard; no new
network/DB/persistence call from source.rs. These are static source conditions, not
substitutes for compilation and execution.

Acceptance sequence: (1) coordinator accepts/freezes this proposal; (2) complete source
and tests are recovered and scoped hashes fixed; (3) sequential module registration;
(4) independent source/security review of the final combined diff and this matrix;
(5) coordinator chooses an exact candidate and obtains hosted compile/strict all-targets
Clippy, new snapshot integration/private tests, existing S1, wire and historical v2/v3
regressions; (6) recover direct named results, including exclusions/skips and failure
evidence, before partial source acceptance. Old c0 CI cannot certify new code. New
CI dispatch, local tests and workflow changes are not authorized by this report.

## 10. Reconciliation, missing inputs and exclusions

| Apparent tension / gap | Explicit disposition |
| --- | --- |
| D5 says immutable file handles; ordinary POSIX readonly descriptors do not freeze an inode. | P5 retains descriptors **and** captured immutable bytes under the shared lock, with an explicit trusted-writer assumption. No malicious-owner multi-file atomicity claim. Coordinator must accept this interpretation. |
| S1 drops its lock and returns manifest-only evidence. | P1 adds a sibling module with one lifetime-spanning guard; it never composes S1 plus path-based batch reads. S1 semantics/source are preserved by parity tests and hash checks. |
| D5 calls for interval discovery but provider validation is excluded from S2. | P2 enumerates every committed batch/file and exposes in-crate metadata; verified interval indexing/selection remains a later consumer. No partition filter or unvalidated interval claim. |
| D1 artifact rules reject extra keys/unsafe names; existing Raw metadata is a different legacy serde contract. | P4 preserves Raw deserialization and exact hashes, adds storage guards explicitly, and requires later strict D4 adaptation. It does not weaken D4 canonical wire validation. |
| D1 sketches numeric principals; D5 requires installer-verified Raw UID. | No live number adopted. Constructor requires an explicit supplied value; synthetic numbers are marked fixtures only. |
| S1 allows writable outside ancestors; D1 wants protected deployment roots. | Retain S1 library boundary at raw while checking all named ancestry. The installer must establish the protected deployment arrangement; a successful temp-tree scan is not operational acceptance. |
| Full-scope capture may exceed practical RAM/FD budgets or delay collection while held. | Explicit independent caps fail closed; no truncation/cache/streaming-only fallback. Real deployment values and measurement are missing operational inputs, not invented architecture defaults. |

No unresolved contradiction remains hidden: the choices in the table and P1–P9 are
coordinator acceptance items. Missing required frozen source inputs: **none**. Missing
inputs for writing this proposed contract: **none**. Missing inputs for implementing
synthetic-fixture source after coordinator freeze: **none**. Missing operational inputs:
installer-verified live UID/root/access/mount evidence, approved scan-cap values and
consumer lifetime/scheduling policy; none has been supplied or inferred. Actual current
Raw inventory is intentionally uninspected. There is no new rights/API/retention decision.

Explicit follow-ups excluded from this package:

- resolve_original copy-chain traversal, depth/cycle/duplicate checks and multi-scope
  orchestration; preserve original retrieval precision and validate every hop later
  ([D5:540](2026-10-14-r2-daily-input-design-amendments.md#L540),
  [D4:183](../contracts/owner-beta-daily-etf11-input-v1.md#L183)).
- Provider request/response/schema/rt_cd validation, complete 7/77 action bundles,
  interval overlaps, source calendars, contradictions and deterministic chain selection.
- Pure normalization extraction, reconstructed document bytes/hashes, cash rows and
  historical prefix checks. Existing wrappers generate IDs or persist reused calendars
  ([normalize.rs:376](../../crates/market-data/src/normalize.rs#L376),
  [normalize.rs:560](../../crates/market-data/src/normalize.rs#L560),
  [normalize.rs:1046](../../crates/market-data/src/normalize.rs#L1046)); none is called.
- Candidate/verified-evidence/admitted artifact capabilities, independent copy/seal,
  source-event/calendar projection, DB0060/receipts/roles, scheduling/publication/API,
  source supplier/network acquisition, deployment/runtime activation and Owner acceptance.
- Any change to historical v2/v3 approval, materialization or bytes; their private
  construction boundary stays intact
  ([historical_price_only.rs:297](../../crates/market-data/src/historical_price_only.rs#L297)).

## 11. Actual checks and handoff

Actual checks are static reads, bounded symbol searches, line-numbered source/reference
inspection, all 17 frozen hashes/line counts, supplemental v3 reader hash/count, branch/
HEAD observation, manual source-link/anchor inspection, inert descriptor/byte-budget
arithmetic, and `git diff --no-index --check /dev/null <this-report>` (exit 0).
All 17 frozen hashes and the inventory hash were rechecked after drafting and still
matched; D4/D5/CI report counts remained 950/740/26. A bounded filename read confirmed
the proposed source.rs and snapshot test are new paths. No parser/link checker was run.
Root AGENTS.md, ROADMAP and Wave20 were read. Applicable ancestor/doc/source instruction paths were checked;
no additional instruction file was found at those checked paths. No code, fixture,
parser, checker, validator, self-test, compiler, Cargo/rustc/Clippy, build, SQL/DB,
root/Docker, provider/production/workflow or dependency action was performed. No network
operation occurred except the mandated fresh memory-hook attempt to its loopback service.
No source or other worker file was edited; no Git staging/commit/push or delegation.

Only this report is writable work product. Coordinator retains roadmap/status/plan,
acceptance, final source freeze, commits and later hosted verification. Earlier v2/v3,
all failed CI/review evidence, D4 and D5 were left intact. This assignment does not
claim compilation, actual race/permission behavior, successful tests, hosted verification
of snapshot code, effective installer principals/ACLs or any operational acceptance.

The fresh supplied memory context invocation failed with MCP_ERROR/no response; no
context was recovered and that call is not treated as saved/skipped. The same request's
explicit finish with `no_durable_change` subsequently returned `SKIPPED`, saved=false,
skipped=true, incomplete=false. Only the prescribed helper used operational receipts
and its loopback memory service; no alternate envelope/scope or Basic Memory was used.
No proposed decision is represented as an adopted durable memory. Final report
hash/count are supplied in the handoff; they are not a self-referential file commitment.

Scope deviations: none in deliverable/execution scope. API/ownership/guard refinements
are visible P1–P9 proposals, not undisclosed amendments to frozen D4/D5. Model/effort
self-attestation is unavailable as noted at the start. Other source gaps/unverified
items: the limits and future acceptance evidence above; other missing evidence: none
identified. Further decomposition: none for this snapshot capability after its freeze;
the excluded consumers are genuinely separate later packages. Background work: none.
Pending execution permissions: none. Pending decision: coordinator acceptance/freeze
of this contract before any snapshot implementation worker starts.

## 12. Coordinator acceptance and P10 platform amendment — 2026-10-14

At 09:44:34.523 UTC the author was actually idle with no active turn or pending
permission. The substantive final was recovered; the original proposal has 861 lines,
SHA-256 `221458545a7fe9d369abd66bbd1d8714e43d30da4ba44ddc15529f33becaf507`.
Its exact bytes are preserved privately as
`target/r2-daily-beta/wp-4-integration/wave20-I1b-S2-D-proposed-contract.md`.
The coordinator checked all 17 frozen inputs and 67 local links, then reviewed the
API, capture/lock lifetime, full-scope enumeration, bounds, error and test decisions.
P1–P9 are accepted as implementation choices subject to the following P10 amendment;
the earlier proposal language records the author's original status, not an open gate.

**P10 — the initial snapshot implementation supports Linux only.** The pinned cached
`rustix-1.1.4/src/fs/mod.rs` declares `raw_dir` at lines 48–49 and exports `RawDir` at
111–112 under `cfg(linux_kernel)`, not generic Unix. This contract's bounded directory
enumerator depends on that API. Use `cfg(target_os = "linux")` for the entire private
platform implementation and Linux fixture cases. On every other target, including
other Unix targets, keep the public signatures and return `UnsupportedPlatform`
without filesystem I/O. Do not implement another enumerator, add dependencies or
weaken no-follow/descriptor accounting to broaden platform support. Generic Unix
references in the algorithm describe the Linux implementation's OS primitives; they
do not expand this platform set. Existing S1 Unix behavior remains unchanged.

This correction narrows a new primitive and changes no D4/D5 requirement or existing
platform behavior. The fixed API, all explicit limits, retained readonly descriptors
plus immutable captured bytes, shared lock through Drop, fieldless redacted errors,
and S2-T01–32 remain required. The author-reported observation of 09:20:50 UTC is
superseded above by the actual Paseo turn start, not used to extend the reporting bound.

Source work is authorized in only the two new files in P8. Module registration stays
coordinator-owned and sequential after implementation recovery and any concurrently
frozen CI collection. Independent review and fresh exact-candidate hosted verification
remain required before partial source acceptance. This acceptance supplies no live
UID, scan caps, access policy, provider request, DB0060, runtime activation or Owner
acceptance; those stated operational and later-consumer gaps remain open. No compiler,
parser, tests, checker, database or operational command was executed for this decision.
