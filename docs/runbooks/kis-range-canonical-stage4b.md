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

## Owner-beta range verifier (separate bounded path)

The owner-only beta does **not** multiply the per-session Stage4B package into
1,608 independent approvals. That would require 1,608 reviewed schedule and
listing artifacts even though the approved price-only output makes no
historical intraday-time or listing-interval claim.

`verify_historical_price_only_beta_input` is a separate, narrower boundary for
`kis-historical-price-only-beta-v1`. It requires two independently reviewed
pins as inputs:

- the serialized immutable manifest hash for the fixed Stage5 source batch
  `3d4f061f-8b8c-54f3-bb44-4d491b3ad256` (exactly 187 source files); and
- the serialized immutable manifest hash for one exact seven-file KSD action
  batch over `2020-01-31..2026-08-19`.

The verifier discovers neither pin. It re-reads the pinned Stage5 files once,
derives all 1,608 normalized batch IDs from the source manifest and the
checked-in calendar/listing-snapshot identities, then verifies every Stage4A
document, ETF11 row, source-row hash/size/query link, and normalized file hash.
It separately re-reads all seven pinned KSD files, accepts only verified bonus
issues, and rejects every other nonempty action class. Success yields an
opaque, non-serializable `HistoricalPriceOnlyBetaInput`; callers cannot create
or deserialize one without `RawStore` verification.

The provider-free command
`kis-historical-price-beta-verify --raw-root ... --stage5-manifest-sha256 ...
--action-manifest-sha256 ...` exposes this check. Its output is limited to
static contract flags, counts, batch IDs, and hashes. It writes no Raw,
Curated, approval registry, database row, or five-pin and makes no network,
provider, Docker, or systemd call.

This verifier is only the authenticated input seam. The in-memory materializer
below applies verified bonus split factors and produces a date-only price
candidate without invented open/close timestamps or total-return artifacts.
Its candidate remains owner-only, `vendor_snapshot=true`, `strict_pit=false`,
and `PRICE_RETURN_ONLY`; a separate review manifest is still required. The
candidate encodes the closed `OWNER_ONLY` audience scope, but this in-memory
layer does not authorize any user identity because it has no publication or
access path. Downstream publication must enforce owner identity independently.
Until that manifest is independently approved, the owner beta remains
unregistered, unpublishable, and unavailable to recommendation/backtest.

## In-memory historical price-only beta candidate

`materialize_historical_price_only_beta(&HistoricalPriceOnlyBetaInput)` is the
bounded materializer at this seam. It returns an opaque in-memory candidate and
does not serialize, write a file, create a database-ready type, or register a
publication. Its rows contain date-only `(instrument, session_date)` keys,
separate raw `open/high/low/close/volume/trading_value` fields, and separate
split-adjusted `open/high/low/close` fields. Volume and trading value are kept
raw; there is no dividend or total-return field.

Only verified `RangeAction::BonusIssue` evidence is accepted. Source files must
already follow the Stage5 producer's fixed ETF11 instrument order and numeric
range-window order, so `window-9` precedes `window-10` regardless of lexical
filename order. Bonus actions must be ordered by
`(instrument_id, ex_date, record_date, split_factor, acquired_at)`; noncanonical
input is rejected rather than silently sorted. Duplicate or conflicting bonus
actions with the same `(instrument_id, ex_date)` identity are rejected. For a
bar whose date is strictly before an action `ex_date`, the materializer
multiplies the exact raw price by that action's reciprocal `split_factor`; the
action is not applied on or after `ex_date`. Reciprocal factors and the
cumulative factor use scale 8 with half-even rounding after each
multiplication. Adjusted prices use scale 4 with one final half-even round.
Output ordering is canonical by instrument and date.

The candidate carries the exact Stage5 and KSD batch/manifest pins, source
file metadata, normalized-session witnesses, canonical bonus evidence, exact
source-file/action-file/session/row counts, and a SHA-256 over an explicit
canonical representation of those values and the rows. Bonus evidence exposes
only `acquired_at` retrieval provenance; it does not expose or imply
`available_at` point-in-time semantics. Fixed metadata is
`vendor_snapshot=true`, `strict_pit=false`, `capability=PRICE_RETURN_ONLY`,
`OWNER_ONLY`, `in_memory=true`, `materialized=false`, and `ready=false`. The
materializer fails closed on input/order mismatches, duplicate bars or actions,
unsupported actions, invalid split factors, raw or rounded OHLC invariant
violations, and fixed-point or canonical-hash overflow/serialization failure.
Acquisition timestamps retained in provenance are source retrieval evidence
only; no market-open, market-close, or invented session timestamp is generated.

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
