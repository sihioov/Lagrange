# R2 snapshot second correction scope — Wave27

R2-04/09 remain DOING. This records coordinator recovery and bounded correction
decisions for source `7545094404628974c1bd626c84f6c3f704555f5f`.
No snapshot source, compiler, Clippy, test, CI or operational acceptance is claimed.

## Recovered evidence

The independent [correction review](2026-10-14-r2-raw-snapshot-corrections-review.md)
has543 lines, SHA256
`ebe51618235e06b3afe7e8075ecf7feb4c137d918a67c7be3ee715803f07d344`.
Reviewer a4b5c1b9 was actual idle, activeTurn null and pending0 at
13:33:35.732UTC. The full substantive final was recovered13:33:46UTC and lifecycle
reconfirmed13:38:03UTC. Private final/recovery receipts are retained under
target/r2-daily-beta/wp-4-integration/wave26-I1b-S2-F-R-*.
Heartbeat b6352577 was deleted only after full recovery was persisted.

The coordinator checked all38 frozen pins,543 report lines and119 local
report-link/line bounds, read the complete dispositions and compared the concrete
remaining findings with the source. All45 definitions remain WRITTEN_UNEXECUTED:
15 private Linux,29 integration Linux, one non-Linux. The report's resolved labels
for13 original groups are source-review dispositions; R05/R10/R12 remain partial.
The [original review](2026-10-14-r2-raw-snapshot-source-review.md), all R12a-h,
P1-P10, T01-T32 and C01 are preserved as obligations, not collapsed into a pass.

## Accepted bounded corrections

| Finding | Coordinator disposition and concrete boundary |
| --- | --- |
| FR01 P1 | Accept. [S3143](../../crates/market-data/src/daily_price_only/source.rs#L3143) consumes non-Copy/non-Clone ScanLimits before3154 reuse. Reconstruct from copied values; do not widen capability traits. |
| FR02 P1 | Accept. [S3234](../../crates/market-data/src/daily_price_only/source.rs#L3234) unwrap needs Debug on opaque BoundedParseError. Match/fixed error; never format its serde payload. |
| FR03 P2 | Accept strict-lint exposure at [S3396](../../crates/market-data/src/daily_price_only/source.rs#L3396). Use reverse selection without suppressing warnings; no executed Clippy result exists. |
| FR04 P2 | Accept. [S3012](../../crates/market-data/src/daily_price_only/source.rs#L3012) cannot identify a closed lifetime after a parallel reader reuses a shared-inode FD slot. Introduce the private owner/observer boundary below, preserving all opens and native lock evidence. This is a test-oracle defect, not an observed descriptor leak. |
| FR05 P2 | Accept. [S2824](../../crates/market-data/src/daily_price_only/source.rs#L2824) appends the closing suffix and yields malformed non-EOF JSON. Keep malformed and genuine EOF cases distinct; prove is_eof under permissive cap and resource rejection/no tree under the finite cap. Add direct total-file/batch preflight observations. |
| FR06 P2 | Accept. [I1697](../../crates/market-data/tests/daily_price_only_snapshot.rs#L1697) removes bounded completion and can hang after lock-release failure. Bound every receive, preserve actual WouldBlock, probe release nonblockingly before append and avoid joining a blocked worker. |
| FR07 P2 | Accept. [I426](../../crates/market-data/tests/daily_price_only_snapshot.rs#L426) examines only the first of three returned file views. Require all batches/files, order, lengths, identity, distinct expected bytes/hash/size and exact metadata commitments. |
| FR08 P2 | Accept fixed diagnostic mismatch. [S1826](../../crates/market-data/src/daily_price_only/source.rs#L1826) is shared by initial and later membership. Lost captured names must be SourceChanged; initial absence and extra-entry errors remain unchanged. Add final-phase body and metadata move-outside cases. No successful bypass is alleged. |
| FR09 P2 | Accept. [S1708](../../crates/market-data/src/daily_price_only/source.rs#L1708) keeps parsed metadata alive through body capture, contrary to [P3](2026-10-14-r2-raw-snapshot-implementation-contract.md#L341). End its lexical scope after full Eq. This is bounded temporary overlap, not an RSS or retained-byte cap bypass. |
| E01 P2 | Coordinator correction. Preserve the frozen bad inventory and supply the correctly qualified private module names in a successor; regenerate final-candidate names after source correction. |

FR07/08/09 are previously unreported fixed-contract gaps; they are not represented
as regressions introduced by Wave25. Existing correct lock-last ownership, parser
resource-before-EOF classification, explicit limits/UID and immutable byte capture
must remain. No public API, dependency, registration or contract revision is needed.

### FD ownership and test evidence decision

Use a private file owner for real production source handles with test-only lifetime
observation. It must contain the sole File, close that File before marking the
unique lifetime closed, and retain its observation destination across moves and
beyond the thread-local collection scope. Wrap immediately after native open, before
fallible post-open checks. Internal borrowing preserves existing read/stat/lock
behavior, readonly flags, budgets, one lock and lock-last field ownership. No new
descriptor or public extractor/trait is introduced.

Every open, including common ancestors and failure temporaries, remains accounted
for. Live handles retain native checks. After close, use unique lifetime identity,
fixture-unique native evidence where sound, and a separately opened lock competitor.
Do not query a shared ancestor's old FD slot and call that ownership. A written
same-inode/different-owner case must distinguish lifetimes without requiring OS FD
allocation order. No global FD counts, suite serialization, dup/try_clone, unsafe
manual close, sleeps, or test-only alternative cleanup algorithm.

## E01 successor and preserved inputs

The original source-wave-26-S2-test-inventory.json remains414 lines, SHA256
`2902947e9bccc7ad5acc4f2470ef8c9ffff04fe9c8bc7e1153c972bb71b6f441`.
The successor source-wave-27-S2-baseline-test-inventory.json is423 lines, SHA256
`1d8ef030a3b3e285fe1f5cc8b6edde9cb9bb667bd5bf8bab3c9efb79e8738a54`.
Exactly15 private cargoResultName prefixes change from
daily_price_only::source::linux::tests:: to
daily_price_only::source::linux::private_tests::. All45 function definitions and
source positions were textually matched; platform/WRITTEN_UNEXECUTED states remain.
This is the7545094 baseline inventory, not future corrected candidate evidence.

Source3987lines/4577d5f1, integration1896/754a32ec and mod9/700469d6 remain unchanged.
The898-line c6f15291 accepted contract and both reviews remain frozen. Current
baseline copies are supplied read-only; the worker may write only source.rs and
daily_price_only_snapshot.rs.

## Routing and next gate

The [Wave27 plan](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md)
classifies one intermediate package with medium confidence. Initial implementation
and first Luna/max correction both failed independent verification, including new
compile blockers and repeated R10/R12 evidence defects. Escalate only the model
to Sol/max; retain effort max. The configured Astra/high profile is explicitly for
the coordinator and does not fit. Current provider discovery confirms Sol/max.
Keep the independent reviewer separate from implementation.

Freeze and commit the brief before notified launch. Recover actual idle/pending0
and a complete substantive result at the90-minute bound, then validate exact delta,
all pins and written tests and obtain independent correction review. No new S2 CI
or push occurs before review closure and an existing-run query. Prior
a888cd6/run38545976070 attempt1SUCCESS remains I0c partial evidence only; nested
helper rows/exits remain UNKNOWN. Preserve c0ac599/929fcd6 partial successes and
older failures. Negative bootstrap/rollback,0060, provider/copy reconstruction and
admission, I6 diagrams before main, runtime, Owner and final C_R2 remain open.
