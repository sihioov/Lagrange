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
node capture.mjs --from 2020-02-03 --to 2020-02-07 --out /path/to/staging \
  --confirm KIND_ETF_DISCLOSURE_CAPTURE
```

The exact confirmation value is an operator gate, not standing approval for a
new collection. Run the command only inside a separately approved date range
and request budget. It refuses to write into a non-empty directory, because one staging directory
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

## Historical volume observations; no current collection budget

Measured on 2020-02-03..2020-02-07: 473 disclosures, 32 pages at 15 rows per
page, and the last page correctly partial at 8 rows. That is roughly 95
disclosures per calendar day for the whole ETF universe, so ~6 days fills the
40-page bound.

Do not use that historical average as an operating window. A later approved
`2026-08-15..2026-08-19` pilot exceeded the 40-page bound and terminated
`page_bound_reached`, so even five calendar days is not a safe current-volume
bound. That one-shot pilot authorization is consumed. Any additional capture,
including a narrower window or full backfill, requires explicit approval of the
date range, request budget, and retention location. The capture still probes one
page past the configured stored-page bound: a duplicate probe proves an
exactly-40-page capture complete; a distinct probe writes `page_bound_reached`
and exits non-zero rather than silently truncating.

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

## Correction-evidence viewer (one acceptance)

This is a separate, operator-gated capture. Run exactly one acceptance number:

```sh
node capture-correction.mjs --from 2020-02-03 --to 2020-02-07 \
  --acceptance 20200207000058 --out /path/to/staging \
  --confirm KIND_CORRECTION_EVIDENCE_CAPTURE
```

`--out` must be a new path. Its existing parent must be a real, non-symlink
directory; an existing empty output directory is rejected. Capture atomically
reserves the final directory name with an exclusive `mkdir`, pins that directory
to a no-follow descriptor, and never replaces an existing path. A directory
without `capture.json` is an uncommitted capture and the Rust consumer rejects it.

The operator gate covers the low-volume D11 exception. The script uses the
ETF disclosure entry page and KIND's own date/search controls, then accepts an
exact `openDisclsViewer('<acceptance>','')` anchor only when it is unique in
the exact requested-range page-1 POST body has been read and validated as
strict UTF-8 and at most 1 MiB. Repeated nodes are admitted only when they
carry one identical raw handler value; a distinct handler for the same
acceptance fails closed. Default,
malformed, other-range, oversize, invalid-UTF-8, or still-pending response
bodies do not authorize DOM attribution. It never constructs or directly
navigates to a viewer URL, sends a reconstructed HTTP request, opens detailed
search or another page, or paginates. A missing or duplicate target is
incomplete and exits non-zero.

The anchor opens the viewer through the opener page's popup event. The
popup is admitted only for HTTPS `kind.krx.co.kr` path
`/common/disclsviewer.do` without userinfo, port, or hash; browser-generated
query semantics are opaque and are not recorded. Completion requires exactly
one rendered `select` whose `id` or `name` is `mainDoc` and which has at least
one option. An empty `mainDoc` is incomplete. The CLI rejects non-calendar
dates and a reversed range.

Complete staging contains `viewer.html` as the exact UTF-8 bytes of the
rendered DOM serialization, labelled `artifact_kind: rendered_dom_snapshot`;
these are not HTTP response bytes. The live anchor is matched by its exact
`onclick` string again in the same page task that clicks it; a live missing,
duplicate, or changed target fails closed. `capture.json` records the requested
range, acceptance, origin path, termination, and `file` only for
`viewer_loaded`. Complete viewer and metadata files are staged through
the pinned output descriptor. `viewer.html` is written first and `capture.json`
last as the consumer commit marker. Parent and output device/inode identities
are checked immediately before and after that marker; a metadata finalization
failure removes only this invocation's known files and its still-identical
reserved directory.
Missing target, duplicate target, no response, no popup, invalid viewer URL,
or missing/duplicate/empty `mainDoc` are retained as closed incomplete outcomes.

### Approved observation (2026-08-20)

The path was exercised for the single-day range `2020-02-07` with list-anchor
acceptance `20200207000058`. It uses only the ETF entry page and the site's own
controls: `fnSearch` runs initially and at most once more; the exact page-1 POST
response and handler are verified; readiness is read-only; the popup waiter is
page-scoped and installed only after readiness; and the final recheck plus click
is atomic. The viewer origin/path is exact-checked and the rendered DOM snapshot
and metadata are committed through the exclusive output directory described
above. No direct/reconstructed HTTP, direct viewer
goto, pagination, bulk/scheduled/full-history capture, query recording, or
body/provider-prose logging is allowed.

The 35 Node tests pass. The exact response was 12,852 bytes with 13 form fields;
the target handler occurred four times but had exactly one distinct raw value.
The rendered snapshot was 24,886 bytes. It contains exactly one
`mainDoc` select: option 0 has an explicit empty value (its rendered prompt is
not treated as evidence), and the sole real option has
raw value `20200207000081|Y`, acceptance token `20200207000081`, and label date
`2020.02.07`. The list anchor and
option acceptance must remain separate. This proves option-level acceptance
resolution and ordered membership shape only; it does not prove a correction
chain or any equality/join, predecessor, supersedes, withdrawal, time, or
timezone semantics. `|Y` is opaque beyond this exact observed shape. Dates remain
date-only and must never be derived from IDs. The prior non-ETF direct-viewer
sample is not an ETF implementation basis; `20251204000324` lacked ETF-list
provenance and was rejected.

Playwright initially lacked `libasound.so.2`; the existing user-space
`/home/l1nnx/tools/pwlibs` library was supplied through process-only
`LD_LIBRARY_PATH`, with no system installation or privilege change. Failed
captures were safe incomplete metadata-only outcomes. The persistent
`missing_target` cause was a response tracker that omitted the expected
acceptance; passing the CLI acceptance and validating the response expectation
fixed it. The resulting viewer HTML remains only under `/tmp` and must not enter
Git. Rust Raw ingest and ordered-membership normalization are implemented. The
list-response diagnostic size and rendered-viewer size are independently
bounded because they describe different artifacts; the real staging directory
passes the Rust CLI's read-only `--plan`, and a one-shot execute against a new
`/tmp` Raw root confirms the actual viewer passes the strict parser and atomic
Raw ingest. Rust staging reads are anchored to one opened directory/file
descriptor set; capture publication similarly anchors both the existing parent
and its exclusively reserved final directory, with `capture.json` written last
as the admission marker. Neither the viewer nor the
temporary Raw data enters Git.
