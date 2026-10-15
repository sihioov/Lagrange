# R2 I4d bounded sealed-chain replay

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09.
Baseline: `47c7dbca1d1898916c7cc734374254623fd23594`, whose
[full CI recovery](2026-10-16-r2-daily-success-ci.md) preceded this change.
Status: direct coordinator source review; fresh hosted checks pending.

## Scope and behavior

Only [artifact.rs](../../crates/market-data/src/daily_price_only/artifact.rs),
[replay.rs](../../crates/market-data/src/daily_price_only/replay.rs) and
[mod.rs](../../crates/market-data/src/daily_price_only/mod.rs) change.
No historical approval registry, first-input/continuation verifier, SQL, queue,
binary activation, provider request or runtime configuration changes.

The new reader begins with all four expected daily pins and an actual approved
historical base plus canonical hash-pinned policy. It reads admission metadata
backward from that exact tip. Each record must match its requested hash, canonical
encoding and policy; the tip must match every supplied pin. Added-session counts
must decrease by exactly one until the first record. Missing parents, inconsistent
counts, repeated identities and malformed records fail closed. There is no latest
pointer, directory-wide candidate search or fallback to another input.

Navigation is private and unapproved. The reader then reopens full bundles in
oldest-to-newest order using the unchanged first-admission and continuation APIs.
Those APIs check the actual approved base, all file commitments and the complete
predecessor relationship. Only their opaque results can populate the returned
chain. A computation input is obtained through the existing capability constructor,
which revalidates retained descriptors. The final tip must still match the requested
pins. Each predecessor's buffers/descriptors are dropped after its child has been
fully verified; only the final cumulative leaf is retained in the result.

The caller must explicitly choose a positive maximum admission count no larger
than the schema session guard. The bound is checked before each metadata read,
and the tip's declared depth is rejected before reading its parent when too large.
Vector/hash-set reservations are fallible. Existing per-file D4 byte bounds remain.
This local work limit does not grant acquisition budget or activation permission.
A caller callback can stop work before/after each metadata/full-leaf operation;
an individual synchronous file operation is not claimed to be preemptible.

The metadata reader shares the existing root/directory owner, mode, ACL, no-follow,
identity and exact-entry checks. It opens only admission.json using the unchanged
bounded file reader and revalidates before releasing its handles. The other three
filenames are required, but their contents are not verified until full replay.
The metadata return value cannot become an admitted input. Existing full reads,
publication, file validation and historical capability factories remain unchanged.

## Written checks and remaining evidence

Eleven navigation tests cover exact one-/three-node order, explicit limits and
pre-parent cutoff, all four tip pins, policy mismatch before IO, missing parents,
wrong depth/first-parent shape, changed/noncanonical bytes, metadata size guards,
cancellation before/between reads and typed value-free errors. Four Linux storage
tests cover metadata-only reads versus an unsafe body, wrong layouts, extra entries,
unsafe modes/symlinks and oversized admission metadata. They use unapproved records
and private storage fixtures; they do not manufacture an approved historical base.

Explicit-file edition-2024 rustfmt, whitespace and direct source review are the
local checks. No local compilation, lint, project test/parser/helper or database
execution occurred. All 15 new definitions are WRITTEN_UNEXECUTED until the new
exact-candidate hosted run. Earlier test names and all previous evidence remain.
This is not an independent review.

Actual approved-base whole-chain replay and daily success publication remain
unexecuted. Full runtime orchestration, superseded-session cancellation, I5-I7,
integrated independent review, distinct UID/ACL/crash/installer rollback, diagrams
before main, operational activation, Owner use, consecutive trading-day observation
and final C_R2 remain open. Existing DB publication/ancestor/revocation gates remain
mandatory; offline file replay does not establish current authority.
