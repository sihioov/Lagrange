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
