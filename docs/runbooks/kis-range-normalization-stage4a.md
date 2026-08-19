# Stage4A KIS historical range normalization

Stage4A is an isolated, bars-only Raw intermediate. It reads immutable
`provider=kis-daily-range/market=kr` windows produced by the Stage3
`FHKST03010100` adapter and writes deterministic per-session batches under
`provider=kis-daily-range-normalized/market=kr`.

It is not an EOD publication, Curated dataset, recommendation dataset,
backtest input, Paper input, or database source. `PublicationBundle`, curation,
and downstream sinks must reject this scope. There is no worker/backfill
production wiring in this stage.

## Boundary

The normalizer re-reads every source file through Raw hash verification and
accepts only the six documented daily-range query fields:

- `FID_COND_MRKT_DIV_CODE=J`
- fixed six-digit symbol from the exact 11-instrument `kr-etf-core-v1` listing
- `FID_INPUT_DATE_1` equal to the selected range start
- `FID_INPUT_DATE_2` inside the selected range
- `FID_PERIOD_DIV_CODE=D`
- `FID_ORG_ADJ_PRC=1` (original/unadjusted price)

Each request must have one blank `tr_cont` header. Only `output2` is parsed;
`output1` and current/reference fields are ignored. Rows must be newest to
oldest, in request bounds, and unique. Multi-window rows are unioned by
`(session_date, symbol)`; overlap, missing symbols, empty responses, gaps, and
dates absent from the validated session list fail closed. Each selected session
must contain exactly the fixed 11 ETF rows.

The expected session list is not inferred from weekdays. The Rust loader embeds
and validates the checked-in XKRX dates-only artifact and manifest, including
artifact hash/size, range, sorted/disjoint sessions and non-sessions, complete
civil-date coverage, and the audit-only `source_schedule` shape. The approved
fixed universe is restricted to `2020-01-31..` through the checked-in artifact
end and carries the `kr-etf-core-v1` listing snapshot id plus the hash of the
checked-in listing snapshot. The source schedule's historical open/close/break
instants are audit evidence only; they are never flattened into the Rust
publication calendar.

Every output batch has a UUID-v5 identity containing normalizer version, source
batch id, source manifest hash, session date, calendar hash, and listing hash.
Its JSON is bars-only and carries exact source file/hash/size/query and row
lineage. Replays return the exact existing batch; concurrent writers converge
under Raw commit locking; orphan metadata is reconciled before reuse.

## Point-in-time limitation

KIS historical `output2` is a current vendor snapshot acquired at the request's
retrieval instant. KIS does not provide availability, revision, or knowledge-time
evidence for these historical rows. Stage4A records only `acquired_at`, with
explicit `strict=false` and all PIT evidence flags false; it never
backdates availability to the bar date and never claims strict historical PIT.
No adjusted values, corporate-action synthesis, holiday filling, or current
reference substitution is performed.

## Not yet approved

Production readiness still requires a separate range-aware canonical contract,
listing/coverage and gap approval, action mappings, strict lineage review, and
an explicit decision about the vendor-snapshot/PIT limitation. Stage4A alone
does not authorize a 2020-to-present canonical Curated backfill or a READY
dataset pin.
