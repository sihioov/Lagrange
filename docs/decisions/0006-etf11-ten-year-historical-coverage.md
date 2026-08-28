# ADR-0006: ETF11 ten-year historical coverage

- **Status:** Accepted for collection; promotion remains evidence-gated
- **Date:** 2026-08-29
- **Decider:** Product owner
- **Range:** 2016-08-29 through 2026-08-28, inclusive
- **Audience:** owner only

## Decision

The owner has replaced the beta-era deferral of the ten-year SC-02 data goal
with an explicit request to extend the fixed ETF11 history to the most recent
ten completed calendar years and to begin the work when technically feasible.
The approved range is `2016-08-29..2026-08-28`: the first boundary is exactly
ten calendar years before the last completed XKRX session available on
2026-08-29.

The existing `2020-01-31..2026-08-19` Stage5 Raw batch and historical-price-v2
artifact remain immutable. The extension must be a fresh, full-range source
batch; it must not splice old Raw and a 2016--2020 increment into lineage that
claims a single capture. Collection uses only the existing KIS read-only daily
bar endpoint and process-owned token manager. It does not authorize accounts,
balances, orders, corrections, cancellations, executions, order WebSockets,
the Compose `live` profile, or any new provider surface.

The ten-year bars remain a retrieval-time vendor snapshot. Every downstream
surface must retain `vendor_snapshot=true`, `strict_pit=false`, owner-only
audience, and `PRICE_RETURN_ONLY` until separately proven otherwise. Raw
collection does not itself authorize Curated admission, publication, dataset
registration, recommendation cutover, or replacement of the currently
approved artifact.

Promotion requires a new historical-price-v3 contract with exact source-batch,
range, calendar, session, file, window, and content pins. KSD corporate-action
evidence must independently cover the same full range. Unsupported nonempty
actions, missing target observations, an incomplete page walk, a hash/count
mismatch, or absent independent approval stops promotion. The v2 artifact
continues serving until all v3 gates pass.

## Fixed-universe existence evidence

The coverage floor is a retrospective fixed-basket data scope, not historical
index membership and not a claim that the basket was selected with information
available in 2016. Official issuer/product material reviewed for this decision
places the latest directly dated ETF11 listing, KODEX KOSDAQ 150 (`229200`), on
2015-10-01, before the chosen floor. The reviewed sources were:

- Samsung Asset Management KODEX product pages for `069500`, `114260`,
  `132030`, and `153130`:
  `https://www.samsungfund.com/etf/product/view.do?id=2ETF01`,
  `https://m.samsungfund.com/etf/product/view.do?id=2ETF19`,
  `https://www.samsungfund.com/etf/product/view.do?id=2ETF24`, and
  `https://www.samsungfund.com/etf/product/view.do?id=2ETF35`.
- Mirae Asset official material for `102110`, `143850`, and `192090`:
  `https://securities.miraeasset.com/public/hks4412/040/ETFGUIDE.pdf`,
  `https://securities.miraeasset.com/bbs/download/2096174.pdf?attachmentId=2096174`,
  and `https://develop.investments.miraeasset.com/tigeretf/ko/introduce/history/content.do`.
  The issuer history identifies TIGER China CSI300 as a February 2014 listing;
  an exact day is not needed for, and is not asserted by, this boundary check.
- Kiwoom Asset Management's official `148070` product article:
  `https://www.kiwoometf.com/service/invest/KO04020102T?kijaGubun=10&kijaNo=169&schGubun2=0&schGubun4=1`.
- Official product pages reviewed for `133690`, `195930`, and `229200` place
  their listings in 2010, 2014, and 2015 respectively. Those page observations
  establish only that the products predate the coverage floor; the KIS Raw
  response remains the price-observation evidence.

Two source discrepancies are deliberately not normalized into invented facts.
Mirae Asset material shows `102110` dates of 2008-04-02 and 2008-04-03, and one
Kiwoom `148070` page shows 2011-10-19 and 2011-10-20. Both alternatives precede
2016-08-29, so the disagreement does not affect this range decision. The KIS
capture must still fail closed if any ETF lacks the requested first/last target
observations required by its response contract.

## Operational budget and durability

The regenerated calendar contains exactly 2,452 XKRX sessions. At the
provider's maximum 100 observations per daily-bar window, 25 windows per ETF
and 275 sequential GETs for ETF11 are expected, paced by the stricter project
ceiling of one request per second for this endpoint/TR channel. Retries remain
bounded and honor `Retry-After`; the existing token manager reuses its token.
The capture is run as a terminal-independent, root-owned transient service
with protected state, and the ordinary daily research worker must not issue
concurrent requests.

This request budget is an operating estimate, not permission to use the code's
much larger absolute safety bound. The actual source-file/window count is
recorded after the provider response is verified; it may not be asserted from
the request estimate alone.

## Execution evidence (2026-08-29)

The daily-price collection completed as immutable KIS Raw batch
`d746ef9f-7eed-5333-97db-cb064331bd06`. It contains 275 payload files, exactly
25 windows for each of the eleven symbols. Range normalization verified all
2,452 approved XKRX sessions from `2016-08-29` through `2026-08-28`. The source
`batch.json` hash is
`sha256:1673cdc3f29ecd13cc5117ce15d1d2e26a22db4328fc8b49926608721a67d5e6`.
This result remains Raw/intermediate evidence with `vendor_snapshot=true`,
`strict_pit=false`, and no publication or recommendation cutover.

The price batch predates persistence of `response_continuation`: all 275
`FileEntry` records omit that optional key. Replay must not rewrite the absence
as a recorded blank marker. The exact capture commit is
`23a01b49114943f93b3c8b240843d360c7485e94`; its daily-range contract and
focused tests already rejected every non-empty response-header marker
(`M`, `N`, `F`, unknown, or whitespace), every body cursor, and every request
continuation before Raw visibility. V3 therefore labels this exact pinned input
`UNRECORDED_CAPTURE_CONTRACT_REJECTED_NONEMPTY_V1`. A replacement capture was
attempted only through the approved isolated wrapper, but the pre-provider
duplicate-source gate correctly returned `KIS_RANGE_BATCH_CONFLICT`; no second
source batch or KIS request was created and the ordinary worker was restored.

Before collecting the matching action evidence, the Raw contract was extended
with an optional, backward-compatible `response_continuation` field. It records
the provider's non-secret response `tr_cont` header separately from the request
header, is omitted for legacy `None` values, and survives `batch.json`, the
append-only manifest, and orphan recovery. This closes the audit gap where the
live collector could validate an `M -> N` walk but a later verifier could not
prove from immutable metadata that the last retained page was terminal. The
contract and isolated action runtime are commit
`8c42a091cedd1031538cb9f3d97ccfeb3a17905c`.

The fixed-ETF11 KSD collection then completed as immutable Raw batch
`fbec8b5d-d87a-4d62-86fa-7af8ebce982b`, retrieved at
`2026-08-28T18:54:49Z`. It contains exactly 77 payloads: eleven symbols times
the seven logical action classes. Every group terminated on page 1, every
request `tr_cont` was blank, and every stored response marker was `E`, which is
a non-`M` terminal marker under the approved KSD policy. The source
`batch.json` hash is
`sha256:73a6c3e18b4cd90ea8aa2daa5a13a6c7572adc6ceed8cbe074e61bc6b5580cf2`;
the exact append-only manifest line hash including its newline is
`sha256:080d38142a6506f741114eb75a77a36c23c2554a0d39eba8843ff62bfb484550`.

Metadata-only QA verified the exact range, endpoint/TR-ID pairs, ETF11 matrix,
redacted credential headers, content hashes and sizes. Body values were not
printed. A local aggregate classification found 157 documented dividend rows,
all with zero stock-dividend rate; the other six classes and positive stock
dividends had zero rows. Numeric parsing, negative-value, and zero-rate
stock/odd-lot payment consistency checks found no error. This classification
is not by itself v3 admission: a committed v3 Raw replay verifier, deterministic
artifact, independent approval record, dual v2/v3 resolver, and immutable
release QA remain required before the ten-year data can serve users.

The committed V3 price/action replay code was also run against the actual
production Raw tree without provider access. Price verification admitted 275
files, 2,452 observed dates, and 26,972 bars with deterministic commitment
`sha256:20c750f0ca415073da37650ae2bb0c942a181b4c86f167defe95895e4499dcf2`.
Action verification admitted 77 files and 157 cash-only dividend rows, produced
zero normalized price actions, and committed those rows as
`sha256:b22a5c9808a8a1a2c892aa3ff46d529672c909620a2c45c0e46d48d0538d17e8`.
Both sides agree on `vendor_snapshot=true`, `strict_pit=false`, and
`PRICE_RETURN_ONLY`; no artifact, approval, registration, or publication was
created by this check.
