# KIND ETF disclosure capture

KIND (`kind.krx.co.kr`) has no API. Its search runs only in a browser: the form
page serves real HTML, but the search POST is refused outside the page because
the endpoint depends on state the page's JavaScript produces. ADR-0004 D11
therefore fixes the mechanism — drive the site's **own** controls in a browser
engine, never a reconstructed request — and this directory is that stage.

## Why it is split in two

A browser cannot be trusted to report what it retrieved. So this stage records
only the interaction and the bytes it received; the `kind-raw` Rust path
recomputes every content hash from those bytes before anything enters the
immutable Raw zone. `CapturedPage` in `crates/market-data/src/providers/kind.rs`
has no hash field at all, so there is no code path for this stage to assert one.

## Not an npm workspace member

The root `package.json` lists only `apps/*`, so a normal root `npm install` does
not pull Playwright or its ~115 MB browser download into dev or CI. Install here
on demand:

```sh
cd data-pipelines/kind-capture
npm install
npx playwright install chromium
```

On a host without the usual desktop libraries, `chrome-headless-shell` may fail
to start on a missing `libasound.so.2`. `playwright install --with-deps` fixes it
but needs root; without root, extract the library from its distribution package
into a user directory and point `LD_LIBRARY_PATH` at it.

## Capture

```sh
node capture.mjs --from 2020-02-03 --to 2020-02-07 --out /path/to/staging
```

It refuses to write into a non-empty directory, because one staging directory
maps 1:1 onto one Raw batch and mixing two captures would corrupt that
correspondence. Pagination uses the page's own `fnPageGo`. It accepts only one
terminal condition as complete: after advancing, KIND returns bytes identical
to the preceding page because it clamped `pageIndex` past the final page. Each
requested page gets one bounded retry; a missing control, two missed responses,
or a distinct response beyond the configured page bound leaves diagnostic
staging behind but exits non-zero. Page indices must come out contiguous from
1; the Rust side enforces that too.

Output:

```
staging/
  capture.json      # source, range, required termination, and per-page metadata
  page-0001.html    # exact response bytes, unmodified
  page-0002.html
```

`capture.json` has a required `termination` value: `clamped_duplicate`,
`page_bound_reached`, `advance_control_missing`, or `no_response`.
Only `clamped_duplicate` is complete and accepted by `kind-raw`; every other
value is retained only as incomplete staging and is rejected before Raw storage.
It also records, per page, the `page_index`, file name, `retrieved_at` instant,
and `form_fields`: ordered, URL-decoded pairs from the page's POST body. Pairs
rather than an object preserve repeated field names and their order.

## Chunk the date range

Measured on 2020-02-03..2020-02-07: 473 disclosures, 32 pages at 15 rows per
page, and the last page correctly partial at 8 rows. That is roughly 95
disclosures per calendar day for the whole ETF universe, so ~6 days fills the
40-page bound.

Capture in windows of about **5 days** to stay under it. A full
2020-01-31..present backfill is therefore a few hundred captures, each its own
staging directory and its own Raw batch. The bound is deliberate: the capture
probes one page past the configured stored-page bound. A duplicate probe proves
an exactly-40-page capture complete; a distinct probe writes
`page_bound_reached` and exits non-zero rather than silently truncating.

Past the last page KIND clamps `pageIndex` and re-serves the final page, so the
capture stops only when a response is byte-identical to its predecessor and
discards that duplicate. The comparison is on bytes alone — this stage never
reads the body.

Verified byte-stable across independent runs: two separate captures of the same
window produced **32 of 32 pages byte-identical**, so the artifact hashes
reproducibly and a re-capture can be checked against an earlier batch.

## Scope

Raw stores the response **unfiltered**. Selection by `종목명` belongs at
normalization, where it is reversible; Raw holds provider bytes rather than a
projection of them. Per-issue filtering at the source was attempted three times
and does not work without popup state the site supplies — see checklist item 17
in `docs/runbooks/stage6-source-contracts.md`.

Collection stays deliberately modest: the site's own control, low request volume,
no parameter probing. ADR-0004 D11 records the accepted risk — whether the KRX
Data Marketplace anti-automation clause reaches KIND is unresolved, since KIND's
own legal notice does not carry one.
