# KIS range Stage4B-0: canonical evidence gate

Stage4B-0 is a local, in-memory boundary after the isolated Stage4A
`kis-daily-range-normalized` Raw contract. The only accepted entry point is
`load_verified_range_canonical_evidence`: it reads one pinned evidence-package
manifest from a trusted absolute directory, verifies every referenced artifact
path/size/SHA-256/schema, and reads KSD responses through immutable RawStore.
Raw DTOs are crate-private deserialization shapes; callers cannot construct the
private verified evidence type directly. The manifest hash must come from an
independently approved pin; callers must not calculate it from the package
being loaded. It accepts one session document only
when the following evidence packages are present and internally hash
consistent:

- a reviewed per-session historical schedule, including open, close, and an
  optional break interval;
- a versioned listing-master snapshot containing exactly the fixed 11 ETF
  instruments, ETF kind, lot size, and a non-empty listing interval for every
  instrument. Its snapshot ID/hash must match the approved Stage4A universe;
  the package artifact hash binds the interval-bearing JSON bytes, so the
  current-reference YAML alone cannot pass;
- all seven KSD action response classes from one pinned KIS Raw manifest, each
  with an exact allowlisted endpoint/TR/query range, filename/hash/size, and
  `rt_cd=0`/`output1` response validation. Stage4B accepts a single terminal
  page only: request `tr_cont` must be present and empty, and known response
  continuation fields (`cts`, `ctx_area_fk`, `ctx_area_nk` and their known
  `200` variants) must be missing, null, or blank. Any non-empty marker is a typed permanent
  `IncompleteActionPagination` failure. The evidence package does not preserve
  a continuation chain, so KSD multipage completeness remains blocked until a
  separately reviewed adapter records every page and its exact headers. Only
  an actual empty array counts as a zero result, while nonempty unsupported
  event classes fail closed;
- an explicit operator approval of the non-strict PIT policy
  `kis-historical-vendor-snapshot-v1`.

The builder parses only the Stage4A original-price OHLCV rows. Prices must be
finite, positive decimals; volume is a non-negative integer; and the OHLC
invariants and exact 11-instrument session coverage are enforced. The result is
`RangeCanonicalCandidate`, an in-memory evidence object whose deterministic
identity binds the Stage4A batch/entry/file hashes, upstream range lineage,
calendar/listing/action evidence hashes, artifact hashes, bridge version, and
PIT policy hash.

Stage4A session-bar documents are consumed only at schema/normalizer v2. The
v2 lineage records each source row's canonical content hash and byte size;
legacy v1 documents are rejected before any canonical candidate is built, so a
replayed v1 payload cannot collide with a v2 deterministic identity. The
repository-controlled approved evidence registry is intentionally empty until
an operator reviews and commits a real package pin; temporary test fixtures use
an internal test-only loader and cannot self-approve through the production
loader.

Before accepting the Stage4A document, the loader re-reads its upstream
`kis-daily-range` manifest by lineage batch ID, checks the serialized manifest
hash, exact source file metadata/readback, and each row's source-file plus
canonical-row hash/size and query link. Action rows are retained in range
coverage but only rows
attributable to the target session enter the candidate's action list.

## Deliberate limitations

KIS `inquire-daily-itemchartprice` is a current vendor snapshot acquired at
retrieval time. KIS does not provide availability, revision, or knowledge-time
evidence for historical rows. Stage4B therefore records `acquired_at` only,
requires explicit non-strict-PIT approval, and never backdates `available_at`
or claims strict historical PIT.

The checked-in XKRX dates artifact has an audit-only source schedule and cannot
be used as historical open/close evidence. Historical schedule exceptions (for
example CSAT sessions and regime changes) must arrive in a separately reviewed
per-session schedule package; they are retained rather than flattened to the
current 09:00/15:30 contract.

The current listing YAML and KIS current-reference responses are not historical
listing evidence. A listing snapshot with effective intervals is required.
Actions are not synthesized, and an empty action list without an exact-range
zero-result attestation is rejected. The only supported action representation
at this boundary is evidence for an already-reviewed bonus-issue mapping;
other KSD event rows remain typed blockers.

This stage does not write Raw or Curated data and is not wired to
`PublicationBundle`, worker backfill, PostgreSQL, recommendation, backtest,
Paper, or live/order paths. A READY dataset, canonical publication, and strict
PIT claim remain blocked pending separate review of schedule, listing, action,
gap/completeness, and lineage contracts.
