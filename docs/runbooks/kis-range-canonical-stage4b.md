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
separate Stage4B approved-evidence registry
(`configs/evidence/kis-range-canonical-approved-manifests.json`) is intentionally
empty until an operator reviews and commits a real package pin; temporary test
fixtures use an internal test-only loader and cannot self-approve through the
production loader. This is not the historical price-only beta approval registry.

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
`kis-historical-price-only-beta-v2`. It requires two independently reviewed
pins as inputs:

- the serialized immutable manifest hash for the fixed Stage5 source batch
  `3d4f061f-8b8c-54f3-bb44-4d491b3ad256` (exactly 187 source files); and
- the serialized immutable manifest hash for one exact seven-file KSD action
  batch over `2020-01-31..2026-08-19`.

The verifier discovers neither pin. It re-reads the pinned Stage5 files once,
derives all 1,608 normalized batch IDs from the source manifest and the
checked-in calendar/listing-snapshot identities, then verifies every Stage4A
document, ETF11 row, source-row hash/size/query link, and normalized file hash.
It separately re-reads all seven pinned KSD files. Bonus issues remain the only
action mapped into price factors. The historical-v2 seam additionally accepts
target-universe dividend rows only when the full official dividend schema is
valid, `record_date` is inside the exact range, `stk_divi_rate` is numeric zero,
and both stock-dividend and odd-lot payment dates are blank. Those rows are
recorded as observed cash-only evidence and deliberately excluded from
`PRICE_RETURN_ONLY`; their cash amount and rate are never added to returns.
Positive stock dividends and every other target non-bonus class remain typed
blockers. The generic daily normalizer and Stage4B package path retain their
original reject-all-target-dividends behavior. Success yields an
opaque, non-serializable `HistoricalPriceOnlyBetaInput`; callers cannot create
or deserialize one without `RawStore` verification.

The ignored-cash-dividend evidence binds a fixed treatment ID, positive target
row count, canonical row commitment hash, dividend source-file hash, pinned KSD
batch/manifest, and retrieval timestamp. It is not a zero-result attestation:
`all_response_arrays_empty` remains true only when every Raw response array was
actually empty. The v2 materializer and sealed artifact include this commitment
in their candidate/manifest identity, while retaining no Raw response body,
request metadata, dividend cash flow, inferred ex-date, or total-return factor.

### Candidate-only historical pin discovery

`discover_historical_price_only_beta_pins(&RawStore)` and the provider-free
`kis-historical-price-beta-pin-discover --raw-root ...` command are a
metadata-only candidate seam before the explicit verifier. Discovery reads
only the already committed source and KIS action manifest rows through
`read_committed_manifest`; it does not reconcile orphan batches, read any
response body, call a body reader, create a Raw directory, append a manifest,
or approve a pin.

Discovery first selects exactly one committed entry whose batch ID is the
contractual `HISTORICAL_PRICE_ONLY_BETA_SOURCE_BATCH_ID`; unrelated 187-file
batches never create source ambiguity. It then validates that selected entry's
exact Stage5 metadata shape: `kis-daily-range/kr`, credentialed mode, and 187
`Bars` files in canonical producer order — each fixed `KR_ETF_CORE_SYMBOLS`
symbol in order, followed by unpadded windows `1..=17` — with filenames
`daily-bars-range-window-{window}-{symbol}-page-01.json`. Every request is
credentialed, uses the exact daily-bars endpoint, has one
`tr_id=FHKST03010100` and one blank `tr_cont`, and has exactly the six
documented query keys: market `J`, the exact symbol, compact beta start,
compact `FID_INPUT_DATE_2` within the beta range, period `D`, and original
price `1`. Duplicate or extra query/header keys fail closed. Each symbol's
first window ends at the beta end and later metadata end dates strictly
decrease. These are metadata-only checks; discovery does not read bodies or
claim the body-dependent exact oldest-date progression, which remains the
explicit verifier's responsibility.

There must also be exactly one matching credentialed KIS action batch for
`2020-01-31..2026-08-19`: seven initial-page files, one for each allowlisted
KSD class, with exact endpoint/query/TR metadata, blank continuation, and no
duplicate, extra, or continuation file. The returned non-serializable accessor
exposes only the fixed contract/range and the two batch
ID/manifest-hash/file-count triples. It is a candidate, not an approval.

The discovery CLI emits one candidate line only after the complete metadata
match, with `body_bytes_read=false`, `raw_writes=false`, `approved=false`, and
`review_required=true`. Failure output contains only a static reason code and
never paths, names, queries, response text, entitlements, credentials, or
account/order data. A tampered or missing response body can therefore remain
undetected at discovery; the separately invoked explicit verifier must read
and authenticate the reviewed hashes and fails closed on that body condition.
An owner must review the candidate line and independently confirm both hashes
before invoking `kis-historical-price-beta-verify`; discovery itself grants no
publication, materialization, or approval authority.

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

Only verified `RangeAction::BonusIssue` evidence can change prices. The separate
historical-v2 cash-dividend commitment described above changes candidate
identity but never enters cumulative split factors or bar arithmetic. Source files must
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

The candidate carries the exact Stage5 and KSD batch/manifest pins, the fixed
cash-dividend treatment/count/row/source commitments, source
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

## Sealed owner-only beta artifact boundary

The next boundary projects the opaque in-memory candidate into exactly two
canonical files, `bars.ndjson` and `manifest.json`, under a candidate-hash
directory. The artifact state is fixed to `OWNER_ONLY`,
`vendor_snapshot=true`, `strict_pit=false`, `PRICE_RETURN_ONLY`,
`MATERIALIZED`, `UNREGISTERED`, and `NOT_PUBLISHED`. It has no conversion to
Curated, a dataset pin, READY, recommendation, backtest, Paper, or publication
types.

The directory candidate hash is an opaque producer-input commitment. The
candidate's canonical preimage includes Raw request provenance that the sealed
artifact deliberately excludes. An artifact reader therefore validates only
the trusted filesystem boundary, exact allowed files, canonical bytes,
manifest self-hash, bars semantics, fixed flags, and equality between the
directory pin and the manifest declaration. It must not claim to reconstruct
or authenticate the producer candidate hash from the artifact alone.

The projection and semantic parser remain crate-private. The Unix writer and
reader now enforce descriptor-relative `O_NOFOLLOW` opens, single-link regular
files, bounded reads, private modes, file/directory `fsync`, and atomic
no-replace publication. Existing-destination success is permitted only after a
descriptor-safe byte-identical verification; a collision never overwrites the
existing artifact. Their adversarial reader and writer reviews are accepted.
The remaining public seam is limited to a non-constructible verified handle
and a provider-free `materialize`/`check` CLI whose resolved artifact root must
be separate from Raw and Curated. No downstream consumer is exposed.

The restricted command surface is exactly:

```text
kis-historical-price-beta-artifact materialize --raw-root <ABS_DATA_ROOT> --artifact-root <ABS_ARTIFACT_ROOT> --stage5-manifest-sha256 <sha256:64hex> --action-manifest-sha256 <sha256:64hex>
kis-historical-price-beta-artifact check --artifact-root <ABS_ARTIFACT_ROOT> --candidate-content-sha256 <sha256:64hex>
```

`materialize` first resolves the existing roots and rejects an artifact root
that is the data root, an ancestor of it, or equal to/below/aliased to
`<ABS_DATA_ROOT>/raw` or `<ABS_DATA_ROOT>/curated`. The established
`LAGRANGE_ARTIFACTS_DIR` topology, for example
`/var/lib/lagrange/data/artifacts`, is a permitted sibling boundary. The gate
runs before Raw verification and creates no directory. It then performs the
explicit two-pin Raw verification, creates the opaque candidate in memory, and
passes that value directly to the no-replace writer. It never discovers a pin
or serializes an alternate candidate DTO.

`check` accepts no Raw root or source pins. Its successful report says
`raw_authenticity=NOT_REAUTHENTICATED`: it proves the trusted filesystem,
canonical artifact bytes, fixed contract, self-hash, and directory binding,
not reconstruction of the excluded Raw provenance. Both commands emit only
static reason codes on failure and never print supplied paths, file names,
requests, response bodies, credentials, batch IDs, or internal errors. There
is no replace, registration, READY, publication, recommendation, backtest, or
Paper option. The CLI root gate, public API shape, and static output boundary
passed an independent source review after implementation.

The independently reviewed v2 approval record now commits the exact Stage5 pin
`sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4`,
the seven-file KSD action pin
`sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd`,
candidate pin
`sha256:0877d42eab6626de5066c5d38d1c11959b7e2dac005a6c884eff0004c9eab050`, and
artifact pin
`sha256:afd0735dc41e56a5c07403480d66de7baf89fc638d715d0e90507032fb42fc67`.
It covers `2020-01-31..2026-08-19`, ETF11/1,608 sessions/17,688 bars, and one
ignored cash row under `CASH_ONLY_EXCLUDED_FROM_PRICE_RETURN_ONLY_V1`; the
artifact remains `OWNER_ONLY`, `MATERIALIZED`, `UNREGISTERED`, and
`NOT_PUBLISHED`. This record authorizes neither a replacement pin nor another
historical KIS request, registration, READY, publication, recommendation,
backtest, or trading.

### Installed production execution seam

The Rust CLI is built into the next `research-worker` release image; the
ten-image serving manifest remains unchanged. After that exact image and release
are installed, run it only through the root-only
`scripts/ops/kis-historical-price-beta-artifact.sh` wrapper from the installed
`current` release:

```sh
scripts/ops/kis-historical-price-beta-artifact.sh --plan
sudo scripts/ops/kis-historical-price-beta-artifact.sh --preflight
sudo scripts/ops/kis-historical-price-beta-artifact.sh --materialize \
  --stage5-manifest-sha256 sha256:<64-lowercase-hex> \
  --action-manifest-sha256 sha256:<64-lowercase-hex>
sudo scripts/ops/kis-historical-price-beta-artifact.sh --check \
  --candidate-content-sha256 sha256:<64-lowercase-hex>
sudo scripts/ops/kis-historical-price-beta-artifact.sh --approval-check
```

Provisioning creates `<LAGRANGE_ARTIFACTS_DIR>/historical-price-beta-root` as
`10001:10001` mode `0750`. The generic artifact root is `service UID:10001`
mode `0750`, allowing the API's read-only parent mount to traverse through the
data/worker group without world access while the dedicated leaf stays
`10001:10001`. The fixed leaf derivation uses the existing
`LAGRANGE_ARTIFACTS_DIR` setting and adds no required owner-only environment
key. Materialize uses a direct image-ID run bound to the manifest's
`research-worker` ID and OCI revision, with host Raw mounted only at
`/data/raw:ro` and the dedicated leaf at `/artifact-root` read-write. Check
mounts no Raw at all and the leaf read-only. Both use `/data` and
`/artifact-root` as topology-neutral container roots, `network none`, UID/GID
`10001:10001`, a read-only root filesystem, dropped capabilities, and
no-new-privileges. There is no alternate Compose service; the wrapper does not
build, inject a secret or DB/provider environment, mount Curated, publish,
register, or perform a READY transition.
The wrapper repeats the Raw/Curated separation check on host-canonical paths
because independent bind mounts hide host ancestry inside the container.
`--check` remains artifact integrity only and never auto-chains to the separate
`--approval-check`. The checker reads its compile-time embedded approval
registry and accepts no registry path; it mounts only the dedicated artifact
leaf read-only, with no Raw, Curated, DB, or provider surface. The installed
immutable `research-worker` embeds the approved v2 registry. A real
approval-check passed on 2026-08-27 with registry SHA-256
`sha256:4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380`,
reporting `APPROVED`, `OWNER_ONLY`, `vendor_snapshot=true`, `strict_pit=false`,
`PRICE_RETURN_ONLY`, `MATERIALIZED`, `UNREGISTERED`, `NOT_PUBLISHED`, and
ETF11/1,608 sessions/17,688 bars. The installed protected environment
intentionally leaves `LAGRANGE_CODE_COMMIT` unset, so the wrapper requires the
exact installed release commit as a process-local value; without it the call
fails closed as `release_commit_invalid`.

The verified invocation supplied the commit from the operator's pinned release
process, not mutable worktree `HEAD`:

```sh
sudo env LAGRANGE_CODE_COMMIT=037e686da1426260521b4c795bde47d7b5b0c5cf \
  /opt/lagrange/current/scripts/ops/kis-historical-price-beta-artifact.sh --approval-check
```

Do not write that process-local value into the protected `.env`. This evidence
does not claim recommendation success, readiness, publication, strict PIT,
total return, or production user acceptance; source changes after the
installed revision are not deployed.

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
Raw response attestation is rejected. The generic Stage4B boundary still maps
only already-reviewed bonus issues; all target dividends and other KSD event
rows remain typed blockers. The historical price-only v2 exception is confined
to its separately authenticated cash-only commitment and does not widen this
generic boundary.

This stage does not write Raw or Curated data and is not wired to
`PublicationBundle`, worker backfill, PostgreSQL, recommendation, backtest,
Paper, or live/order paths. A READY dataset, canonical publication, and strict
PIT claim remain blocked pending separate review of schedule, listing, action,
gap/completeness, and lineage contracts.
