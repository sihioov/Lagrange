# Stage6 disclosure pipeline — review findings, 2026-08-20

Handoff document. Everything needed to act on these findings without the
conversation that produced them.

## Scope

```
8da8527..5b12e0f      31 commits, 39 files, +10,521 / -22
```

`8da8527` is the last commit **not** in scope. Note that the first in-scope
commit, `99fb8e9`, only committed a `docs/STATUS.md` edit that already existed
uncommitted in the working tree — its content was not authored by this change set.

Line numbers below are as of `5b12e0f` and will drift once edits begin.

Code under review, by weight:

| path | what |
|---|---|
| `crates/opendart-client/` | new crate: read-only OpenDART HTTP transport |
| `crates/market-data/src/providers/opendart.rs` | OpenDART Raw adapter |
| `crates/market-data/src/providers/kind.rs` | KIND capture ingest |
| `crates/market-data/src/kind_normalize.rs` | KIND HTML → typed observations |
| `data-pipelines/collectors/src/bin/opendart-raw.rs` | gated one-shot CLI |
| `data-pipelines/collectors/src/bin/kind-raw.rs` | staging → Raw CLI |
| `data-pipelines/kind-capture/` | Node/Playwright capture stage |

## How this review was produced

Two passes, deliberately different in kind.

**A mutation pass** (author-run) checked whether the change set's own tests can
fail. Each mutation had to compile — a mutation that breaks the build proves
nothing — and the paired test had to die. Eight passed:

| mutation | result |
|---|---|
| `redacted_metadata` records a live key instead of the placeholder | sentinel test died (+3 more) |
| table-`summary` contract validation always returns `Ok` | 2 tests died |
| sequence-monotonicity check removed | gap test died |
| acceptance-number bound widened from `14,14` to `1,64` | died |
| timezone venue `Krx` → `Nasdaq` | died |
| duplicate-page-bytes comparison made unreachable (`&& false`) | died |
| an error message made to contain the host name | leak test died |
| a new error variant added | **build fails** — `E0004 non-exhaustive patterns` |

The last is the strongest: the transport's leak test matches the error enum with
no wildcard, so a future variant cannot be added without also passing the leak
check.

**An independent review pass** (separate reviewer, stronger model) read the diff
with **commit messages withheld** — they state conclusions and would anchor
agreement — and was told to treat doc comments as claims, not evidence. It was
also told which properties the mutation pass had already pinned, so its effort
went to defects nobody had suspected. It used an out-of-tree probe crate at
`/tmp/kindprobe` path-depending on `crates/market-data` and `crates/domain`, so
the repo was never modified.

The mutation pass can only probe what its author suspected. Every finding below
came from the review pass except the two marked *author-verified*.

---

## Findings

### F1 — HIGH: the capture stage reports truncation as success, and ingest cannot tell

`data-pipelines/kind-capture/capture.mjs:145-166`, with
`crates/market-data/src/providers/kind.rs:259`

Three distinct failures each end the page walk, stage a contiguous prefix, print
a success line, and exit 0. Nothing downstream can distinguish "reached the last
page" from "stopped early", because the only legitimate terminal signal — a
response byte-identical to its predecessor, which is how KIND clamps `pageIndex`
past the end — is reached *only when advancing already worked*. The two `break`s
above it are guesses.

**(a) Latency.** Line 161, `if (captured.size === before || !captured.has(next)) break;`,
runs after a fixed `await page.waitForTimeout(4500)`. One response slower than
4.5 s ends the walk. If a 32-page window loses page 18, pages 1–17 are staged,
indices are contiguous from 1, `capture.json` is written, the script prints
"captured 17 page(s)" and exits 0. `kind-raw --execute` then sees 17 contiguous,
non-duplicate, `시간`-bearing pages and commits an immutable Raw batch missing 15
pages. The normalizer accepts it too: `번호` still descends by exactly 1 across the
surviving prefix.

**(b) Control.** Line 159, `if (!moved) break;`. If `fnPageGo` is renamed or the
numeric anchor text changes, `moved` is null on the first advance and a one-page
capture is staged, exit 0.

**(c) Bound.** `for (let next = 2; next <= maxPages; next += 1)` with `maxPages`
capped at 40 (`parseArgs:51-53`). A window with more than 40 pages exits the loop
normally and stages exactly 40. `kind.rs:259` tests `pages.len() > KIND_DISCLOSURE_MAX_PAGES`,
so exactly 40 is accepted. The measured rate is ~95 disclosures/day → ~32 pages
for 5 days, so a 7-day window (~44 pages) truncates silently.

Two documented claims are false as written:

- `data-pipelines/kind-capture/README.md:66-68` — "The bound is deliberate: it is
  better to stop and be told than to silently truncate a result set." Nothing
  tells: no error, no non-zero exit, no marker in `capture.json`.
- `README.md:41-44` — "the terminal condition is observed, not guessed."
- `kind.rs:60-64` claims the bound "mirrors the OpenDART walk bound for the same
  reason". It does not: `providers/opendart.rs:513-515` returns
  `PaginationBoundExceeded` when the bound is reached without a terminal page.
  KIND's ingest has no equivalent because the capture stage sends it no
  total-page claim to check.

**Proposed fix.** Make the terminal condition explicit and carry it across the
boundary.

1. In `capture.mjs`, record *why* the walk ended in `capture.json` — a
   `termination` field with one of `clamped_duplicate` (the only clean end),
   `page_bound_reached`, `advance_control_missing`, `no_response`. Exit non-zero
   for every value except `clamped_duplicate`.
2. Retry a page once on timeout before concluding, and distinguish "no response"
   from "same response".
3. In `kind.rs`, require the termination reason on ingest and reject anything
   other than the clean value, so a truncated staging directory cannot become a
   Raw batch. Change `>` to `>=` on the page bound, or drop the bound check in
   favour of the explicit reason.
4. Consider recording the site's own total-count if a reliable one can be found;
   note that the leading `번호` cell is **not** a reliable total on the 상세검색
   surface (see `docs/runbooks/stage6-source-contracts.md`, the retracted-volume
   section).

---

### F2 — MEDIUM: the normalizer discards the first row of every page unconditionally

`crates/market-data/src/kind_normalize.rs:874-880`

`let mut rows_iter = rows.into_iter(); rows_iter.next();` drops the first `<tr>`
with the comment "It is simply discarded here, never inspected" — and never
checks that it is in fact the empty `<thead>` placeholder.

For the *first* page in stored file-name order, with ≥2 data rows and no
placeholder row, the newest disclosure (highest `번호`) is dropped and the batch is
accepted. `validate_sequence` cannot catch it: the surviving rows still descend by
exactly 1. Measured with the probe crate:

- one page, 3 data rows, no placeholder → parsed OK with **2** observations,
  seq 472, 471 — 473 gone
- two pages, 15 + 3 rows, page 1 lacking the placeholder → OK with **17**,
  `[489 … 473]` — 490 gone, no error

A *later* page lacking the placeholder does fail closed
(`SequenceNumberOutOfOrder`), and a single-row page with no placeholder yields
`EmptyBatch`. So the silent window is narrow — but it drops the newest row, which
is the one a point-in-time consumer cares about most.

This contradicts the module's own doc at `kind_normalize.rs:105-111`: "A single bad
row fails the whole batch … rather than silently under-reporting disclosure
evidence by skipping it."

The discard is untested: every fixture is built by `page_html`
(`tests/kind_normalization.rs:166`), which always emits the placeholder.

Side effect: in the surviving case `source_row_index` is off by one against the
page's real data-row position, degrading the traceability that field exists for.

**Proposed fix.** Validate the discarded row instead of assuming it: require that
the first `<tr>` contains no `<td>`, and return a typed error otherwise. Add a
fixture without the placeholder row.

---

### F3 — MEDIUM: a vacuous sentinel sweep

`crates/market-data/tests/kind_raw_ingestion.rs:564-572`

In `credential_like_form_field_name_fails_closed_and_sentinel_never_persists`, the
loop `for bytes in &scanned { assert!(!text.contains(SENTINEL)) }` executes **zero
times** for all five field names: `ingest_disclosure_capture` rejects at
`validate_form_fields` before `store_batch`, so nothing is written under the store
root and `all_file_bytes_under` returns empty.

It is also strictly redundant — any state where the sweep could fire is one where
the immediately preceding `assert_nothing_written(&store)` has already failed.

The correct pattern exists in the same change set and was not applied here:
`tests/opendart_raw_ingestion.rs:557-567` asserts non-vacuity first
(`assert_eq!(names.len(), 3)`, plus "the sweep must cover batch.json") before
concluding.

Same class, secondary: `tests/kind_raw_ingestion.rs:241`,
`for (page, file) in pages.iter().zip(entry.files.iter())` has no length
assertion, so an empty `entry.files` would skip every byte-exactness and
independent-SHA-256 assertion and the test would pass.

**Proposed fix.** Either assert non-vacuity before sweeping, or replace the sweep
with a positive assertion that can fail — e.g. a successful ingest whose recorded
metadata is then checked for the sentinel. Add `assert_eq!` on collection lengths
before any `for`/`zip` that carries the load-bearing assertions.

---

### F4 — MEDIUM *(author-verified)*: `paper_preview.rs:1020` will panic on a host with the QA database

`crates/job-queue/tests/paper_preview.rs:1020`

This change set corrected `write_preview_bars` to `CurateStore::new(root)`
(line 229), matching `load_recommendation_closes`
(`crates/job-queue/src/paper_preview.rs:237`, unchanged). Line 1020 still reads
`CurateStore::new(fixture.dataset_root.join("curated"))`.

`seed_worker_fixture` calls `write_preview_bars(directory.path(), …)` and sets
`dataset_root = directory.path()`, so for root `/tmp/FIXTURE`:

```
writer (line 229)          -> /tmp/FIXTURE/curated/bars/.../bars.parquet
reader (line 1020)         -> /tmp/FIXTURE/curated/curated/bars/.../bars.parquet
```

The second path is never created, so `std::fs::remove_file(path).unwrap()` panics
with ENOENT in `missing_close_fails_preview_permanently_without_outputs`.

It passes here only because `ScratchDb::create()` returns `None` without a
database and the test returns early — it completes in the 0.01 s batch, never
reaching the path.

**This corrects an earlier author judgement.** The change set's own record claimed
lines 379/698/1020 were self-consistent because "both write and read double-join".
That is false for 1020: the write side was changed to single-join and the read side
was not. Verified by path arithmetic, not by execution.

**Proposed fix.** `CurateStore::new(&fixture.dataset_root)` at line 1020. Lines 379
and 698 use the doubled convention for *manifest* paths and are pre-existing —
check them against their own writers before touching.

---

### F5 — MEDIUM: unbounded read and unconsidered symlinks at an explicitly untrusted boundary

`data-pipelines/collectors/src/bin/kind-raw.rs:406`

`build_captured_pages` does `std::fs::read` per referenced page with no size bound.
`validate_page_file_name` (line 305) checks only for `/`, `\`, `..`, and
absoluteness. `check_no_stray_html_files` (line 336) skips non-`is_file()` entries,
so a character device is neither rejected as a stray nor rejected as a name.

Make `page-0001.html` a symlink to `/dev/zero` and the process allocates until
OOM-killed. A 50 GB regular file does the same, up to 40 times. There is no size
bound in `CapturedPage` or in `ingest_disclosure_capture` either —
`validate_page_body` rejects only *empty* bytes.

The module's own doc at `kind-raw.rs:37-48` states "The staging directory is
untrusted input".

**Proposed fix.** Reject symlinks explicitly (`symlink_metadata` + `file_type().is_symlink()`),
require a regular file, and impose a per-page size ceiling — the observed pages are
~11–13 KB, so a 1 MB bound is generous — with a typed error above it.

---

### F6 — MEDIUM: the OpenDART walk parses the response's own `page_no` and throws it away

`crates/market-data/src/providers/opendart.rs:665`

`documented_envelope_u64(object, "page_no")?;` validates presence and type, then
discards the value instead of comparing it to the page that was requested. The doc
two lines above admits it: "Presence/type of `page_no` and `page_count` is
validated even though this adapter tracks the *requested* page itself".

A server that ignores `page_no` and returns page 1 every time, with any per-response
jitter (a timestamp or request id in `message`, a differing key order), passes every
guard: `total_count`/`total_page` are consistent by construction, and the
duplicate-bytes check (lines 464-474) compares whole bodies, so jittered duplicates
are not detected. The result is a stored batch whose pages all carry the same rows,
presented as N distinct pages, with a manifest asserting completeness.

**Proposed fix.** Compare the parsed `page_no` against the requested index and fail
closed on mismatch. Cheap, and it is the one field that directly proves page
identity.

---

### F7 — MEDIUM: an assertion the type system already guarantees

`data-pipelines/collectors/src/bin/opendart-raw.rs:698`

`Surface::query_parameter_names` (line 148) returns `Vec<&'static str>` built only
from string literals. A caller-supplied value lives in
`ParsedArgs::corp_code: Option<String>`, and a `&str` borrowed from it is not
`'static`, so it cannot enter the returned vector. No mutation of that function —
short of changing its return type and rewriting every push — makes
`assert!(!names.contains(&"00126380"))` fire.

The presence/absence assertions at lines 694-697 are falsifiable; only the leak
assertion is vacuous. Compounding: `run_plan` (lines 273-286), the real output
surface that prints the joined names plus `raw_root` and `entitlement_reference`,
has no test at all.

**Proposed fix.** Drop the vacuous assertion or move it to where a value could
actually appear, and add a test that captures `run_plan`'s stdout and asserts no
value appears in it.

---

### F8 — LOW: a test no longer exercises the property in its name

`crates/job-queue/tests/recommendation_compute.rs:699-712`

`malformed_parquet_is_integrity_not_a_retryable_store_error` writes `b"not parquet"`
and deliberately skips `resync_manifest_artifacts`, so `verify_artifacts` fails on
size/hash/schema before the Parquet reader is entered. `ErrorClass::Integrity` is now
satisfied by the attestation layer. If `read_bars` began classifying malformed bytes
as a retryable store error, the test would still pass.

Its sibling `semantically_invalid_parquet_value_is_integrity` (line 742) was given
the guard that catches exactly this — `assert!(!rendered.contains("artifact attestation"))`
— and this one was not. A comment at lines 704-709 states the situation without
restoring coverage.

**Proposed fix.** Either accept the change in meaning and rename the test, or find
a corruption that passes attestation and still fails the reader. Note that
arbitrary bytes cannot: they fail size, hash, and schema, so the reader is
unreachable through an attested path for this input class.

---

### Remaining low-severity items

- `crates/opendart-client/src/transport.rs:115-119` — every non-`is_connect()`
  `reqwest::Error` (TLS handshake, redirect policy, request builder, body decode) is
  classified `Failure::TimedOut`. Fail-closed and leak-free, so this is a
  diagnosis/labelling defect: `TimedOut` gets reported for failures that never left
  the process.
- `crates/opendart-client/src/credential.rs:127` — `contents.trim_end()` only, so a
  key file written as `" abc\n"` sends a leading space in `crtfc_key`. Causes an
  opaque auth failure rather than a leak. Behaviour matches the comment at line 123
  but probably not the intent.
- `crates/market-data/src/providers/opendart.rs:338-341` — the `Debug` impl comment
  asserts "`OpenDartRead` carries no `Debug` supertrait", but line 100 declares
  `pub trait OpenDartRead: std::fmt::Debug + Send + Sync`. Stale comment; no leak
  follows, since the impl never prints the reader.
- `data-pipelines/kind-capture/capture.mjs:59-75` — `parseFormFields` URL-decodes
  names and values, so what lands in `RequestMetadata::query` is the decoded form,
  not "exactly as the page sent them" as the comment claims. Provenance fidelity
  only.

---

## Verified sound — do not re-derive these

Checked concretely by the review pass; listed so effort goes elsewhere.

- **No credential path to disk or output in `opendart-client`.**
  `OpenDartTransportError` has no `String` or bytes field on any variant
  (`error.rs:31-77`). The leak test matches both enums with no wildcard, so a new
  variant breaks the build. Every `println!`/`eprintln!`/`dbg!`/`panic!`/`unwrap`/
  `expect` in the crate (23 hits) is inside `#[cfg(test)]`. `reqwest::Error` is
  discarded at `transport.rs:95, 115, 125` without being formatted or stored.
  `Secret`'s `Debug` is hand-written and there is no `Display`.
- **`market-data` never holds a key.** `redacted_metadata` (`opendart.rs:352-374`) is
  the only `RequestMetadata` constructor in the module, always appends the fixed
  placeholder, and rejects a caller-supplied `crtfc_key` by name. Every `visible`
  query is built from hardcoded names, so the guard is unreachable from the public
  API.
- **Byte and hash integrity.** `RawEnvelope::new` (`contract.rs:291-302`) hashes
  exactly the bytes it stores; `ContentHash::from_bytes` is real SHA-256.
  `CapturedPage` has no hash field. Nothing transforms bytes en route —
  `validate_page_body`'s `from_utf8_lossy` result is discarded. On read,
  `RawStore::read_batch_bytes` (`storage.rs:1041-1048`) recomputes the hash and
  rejects a mismatch, and canonicalizes each path against the batch dir so a
  traversal cannot be read back.
- **Store atomicity.** `store_batch` (`storage.rs:398-520`) validates scope and every
  file name before creating anything, refuses a pre-existing batch dir, routes every
  later error through a cleanup that removes the partial dir, and publishes the
  manifest row under an exclusive commit lock. Every ingest path calls it exactly
  once, after validation.
- **Timestamp correctness.** `posted_at_instant` is only produced via
  `VenueTimestamp::from_naive_local(Venue::Krx, …)` with `timezone_assumption` set in
  the same struct literal (`kind_normalize.rs:933-957`); there is no other
  constructor. `domain/time.rs:162-175` rejects ambiguous and nonexistent local times
  as typed errors. Minute granularity is enforced rather than widened — feeding
  `"2020-02-07 14:46:00"` yields `InvalidTimestamp`. No date is derived from
  `rcept_no` or from the 14-digit acceptance number.
- **Retry and redirect policy.** Redirects disabled (`transport.rs:89`); 3xx terminal
  and never retried (`client.rs:148-152`); only never-sent and 5xx retry, capped at
  `MAX_RETRIES = 2`. `single_flight_serializes_concurrent_callers` is genuinely
  falsifiable — the fake transport's 20 ms sleep yields on the current-thread
  runtime, so removing the mutex would let concurrency reach 2.
- **Path allowlist is deny-by-default**, returns `&'static str`, and rejects `""`,
  `/api/list.json?`, and `/api/list.json/../../etc/passwd`.
- **HTML scanner fails closed on constructed edge shapes** — a nested `<table>`
  inside a data cell yields `RowCellCountMismatch`; a second table after `</table>`
  parses correctly; a `DetailEtf` batch fed to the normalizer fails closed.
- **Suites and lints green** — `market-data` (20 binaries), `opendart-client` (30),
  `collectors --bin opendart-raw --bin kind-raw` (27), `job-queue --test paper_preview`
  (21); clippy clean on the new crates and bins.
- **The credential-marker false-positive risk is resolved** *(author-verified)*.
  `CREDENTIAL_FIELD_NAME_MARKERS` (`kind.rs:120`) matches substrings
  case-insensitively, so a real field named e.g. `searchKeyword` would reject every
  capture from that surface. Checked against the 13 field names in the preserved
  real capture — `method`, `forward`, `currentPageSize`, `pageIndex`, `orderMode`,
  `orderStat`, `etfIsuSrtCd`, `reportCd`, `reportTmp`, `etfIsuSrtNm`, `reportNm`,
  `fromDate`, `toDate` — **none matches**. Re-check if another surface is added.

## Unchecked — gaps, stated rather than implied absent

- **All live behaviour.** No network was used in the review. The `corpCode.xml`
  XML-error shape, the `020`/`021` patterns, KIND's `pageIndex` clamping, the
  13,085-byte byte-stability figure, and the 5-day/32-page volume numbers come from
  fixtures and `docs/**`.
- **DB-gated tests.** No PostgreSQL is available here, so the `job-queue`
  preview-worker tests and `collectors --test research_worker` did not execute. F4 is
  proven by path arithmetic, not by a failing run.
- **`--plan` stdout for both CLIs** is untested by anything in the change set; it was
  read, not executed.
- **`docs/**`** skimmed only, for the specific claims cited.
- **`storage.rs` beyond `store_batch`/`read_batch_bytes`** — in particular whether a
  crash between `sync_published_metadata` and the manifest append can leave a batch
  dir with no manifest row (`IndeterminateBatchCommit` recovery) was not traced.

## Suggested order of work

1. **F1** — it is the only finding that can put wrong data into an immutable store
   while reporting success, and it invalidates two documented claims. Fixing it
   touches both the Node stage and `kind.rs`.
2. **F2**, **F3**, **F4** — each is small and each closes a hole in something that
   currently claims to be checked.
3. **F5**, **F6** — hardening at boundaries already declared untrusted.
4. **F7**, **F8**, and the low items — cosmetic or documentation-truth fixes.

`docs/runbooks/stage6-source-contracts.md` and `docs/decisions/0004-*.md` will need
amending once F1 lands, since both describe the bound as a stop-and-tell.
