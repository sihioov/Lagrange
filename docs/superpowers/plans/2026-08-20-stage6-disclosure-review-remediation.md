# Stage6 Disclosure Review Remediation Plan

> Source review: `main@f78d033`,
> `docs/reviews/2026-08-20-stage6-disclosure-review.md`

**Goal:** Close F1-F8 and the four remaining low-severity findings without
weakening the read-only provider boundaries, Raw immutability, redaction, or
fail-closed behavior already verified by the review.

**Implementation baseline:** The review document is read from local
`main@f78d033`, while implementation remains in the already-created
`unspecified-task` worktree at baseline `5b12e0f`. Do not create or switch a
branch/worktree, and do not rewrite or reset either branch merely to copy the
review document into the implementation history.

**Safety boundary:** All verification in this plan is fixture-backed and
offline. Do not call KIND, OpenDART, KIS, KRX, KSD, or any other provider. Do
not print credentials, response bodies, provider messages, database URLs, or
account identifiers. The KIS read-only allowlist and every live-order guard are
out of scope and must remain unchanged.

---

## Decisions fixed by this plan

These choices remove the ambiguities left in the review before implementation
begins.

1. **All findings are in scope.** Implement F1-F8 and the four remaining low
   items. F6 is fixed now even though OpenDART has no ETF11 identity path,
   because it is a cheap completeness invariant on the already-shipped generic
   adapter.
2. **KIND's hard stored-page limit remains 40.** After reaching the configured
   stored-page bound (at most 40), capture makes one extra terminal probe. At
   the default bound, a byte-identical page 41 proves a complete 40-page
   result; a distinct page 41 produces `page_bound_reached`. Thus a complete
   40-page result remains valid while a truncated 40-page prefix does not.
3. **Every page-walk outcome after search begins is explicit.**
   `capture.json.termination` is required and is one of `clamped_duplicate`, `page_bound_reached`,
   `advance_control_missing`, or `no_response`. Only `clamped_duplicate` is
   ingestible. Incomplete captures are retained for diagnosis but the capture
   process exits non-zero and `kind-raw` rejects them before Raw storage.
4. **One retry means two bounded waits total.** A missing next-page response is
   retried once using the page's own control. A second miss becomes
   `no_response`; it is never treated as a terminal page.
5. **The KIND placeholder contract is structural.** The first `<tr>` must have
   no `<td>` or `<th>` cells. Anything else is a typed whole-batch failure. A
   valid placeholder remains excluded from the zero-based data-row index.
6. **F8 is corrected to the reachable contract.** Arbitrary invalid bytes cannot
   pass artifact attestation, so rename the test and positively assert that the
   integrity failure came from attestation. Do not weaken or bypass attestation
   merely to force the Parquet reader to run.
7. **Credential file formatting is canonicalized.** Trim leading and trailing
   whitespace, then reject an empty value. The secret itself must still never
   appear in `Debug`, `Display`, logs, or stored metadata.
8. **KIND form fields remain decoded ordered pairs.** Preserve current behavior
   and correct comments/docs that falsely call it byte-exact request encoding.

---

## Do-not-rework boundary

The source review already verified these areas. Change them only where a task
below explicitly requires it:

- OpenDART credential redaction and the exhaustive no-leak error test.
- `RawEnvelope` byte/SHA-256 integrity and `RawStore::store_batch` atomicity.
- Raw readback path canonicalization.
- KIND timestamp, KRX timezone, minute-granularity, and acceptance-number rules.
- OpenDART deny-by-default path allowlist, redirect policy, and bounded retries.
- KIND HTML nested/second-table fail-closed behavior.
- Existing manifest-path conventions at `paper_preview.rs` lines corresponding
  to the review's 379 and 698; F4 changes only the missing-close bars path.

---

## Execution graph and commit boundaries

```text
Task 0 baseline
    |
    v
Task 1 F1 termination contract
    |
    +--> Task 2 F2 normalizer ----+
    +--> Task 3 F3 tests ----------+--> Task 8 docs + final gate
    +--> Task 4 F4 QA path --------+
    +--> Task 5 F5 staging files --+
    +--> Task 6 F6 page identity --+
    +--> Task 7 F7/F8/low ---------+
```

Task 1 is one atomic cross-language change. After it lands, Tasks 2-7 may be
assigned independently where their file lists do not overlap. Keep each
numbered task in its own reviewable commit; Task 7 may be split by crate as
listed there.

---

## Task 0: Establish an offline baseline

**Files:** none

- [ ] Confirm the implementation worktree remains on its existing branch and
  inspect the review from local `main` without changing branches:

  ```sh
  git branch --show-current
  git rev-parse --short HEAD
  git show main:docs/reviews/2026-08-20-stage6-disclosure-review.md >/dev/null
  git status --short
  ```

  Expected: branch `unspecified-task`, baseline `5b12e0f`, the review is
  readable, and the worktree has no unexpected changes outside this remediation
  set. Do not discard user changes if it is dirty.

- [ ] Run the focused baseline without network access:

  ```sh
  cargo test -p opendart-client --locked
  cargo test -p market-data --locked --test kind_raw_ingestion --test kind_normalization --test opendart_raw_ingestion
  cargo test -p collectors --locked --bin kind-raw --bin opendart-raw
  ```

- [ ] Record environmental prerequisites rather than calling a skip a pass:

  - `recommendation_compute` needs the CI-pinned `pyarrow==25.0.0` and
    `uv==0.12.1`.
  - F4's full worker test needs the approved QA PostgreSQL lane and a redacted
    `DATABASE_URL` supplied by that lane.
  - The new pure Node tests do not need a browser or network.

**Acceptance:** Baseline failures are classified as code failures or missing
environment prerequisites. No live provider call is made.

---

## Task 1: Make KIND capture completeness an end-to-end contract (F1)

**Files:**

- Create: `data-pipelines/kind-capture/capture-logic.mjs`
- Create: `data-pipelines/kind-capture/capture-logic.test.mjs`
- Modify: `data-pipelines/kind-capture/capture.mjs`
- Modify: `data-pipelines/kind-capture/package.json`
- Modify: `data-pipelines/collectors/src/bin/kind-raw.rs`
- Modify: `crates/market-data/src/providers/kind.rs`
- Modify: `crates/market-data/tests/kind_raw_ingestion.rs`
- Modify: `data-pipelines/kind-capture/README.md`

### Step 1: Add RED tests for the pagination state machine

- [ ] Extract only pure decisions and bounded-wait orchestration into an
  importable helper. Keep Playwright/browser launch in `capture.mjs`.
- [ ] Use Node's built-in `node:test` and add a package `test` script. Cover:

  - first wait misses, retry obtains the requested page;
  - both waits miss -> `no_response`, non-zero result;
  - page control missing -> `advance_control_missing`, non-zero result;
  - next response equals the previous bytes -> `clamped_duplicate`;
  - distinct page 41 after 40 stored pages -> `page_bound_reached`;
  - duplicate page 41 after 40 stored pages -> clean termination with exactly
    40 stored pages.

  ```sh
  (cd data-pipelines/kind-capture && npm test)
  ```

  Expected before implementation: tests fail because there is no explicit
  termination state machine.

### Step 2: Record every termination reason and set the exit status

- [ ] Write `capture.json` through one function for both complete and
  incomplete outcomes. Include the required string `termination` field.
- [ ] For `no_response`, `advance_control_missing`, and
  `page_bound_reached`, retain any captured prefix for diagnosis, print a
  bounded reason without response bytes, and exit non-zero.
- [ ] Only `clamped_duplicate` may print the existing success summary and exit
  zero.
- [ ] Do not infer a total from the leading `번호` cell.
- [ ] In the capture README, document termination/retry and describe
  `form_fields` as ordered URL-decoded pairs that preserve duplicate names;
  remove the false byte-exact POST-encoding claim.

### Step 3: Make both Rust boundaries reject incomplete capture

- [ ] Add a public, closed termination enum in `providers/kind.rs` and require
  it in `ingest_disclosure_capture`. Add a typed incomplete-capture error with
  no page bytes or free-form provider text.
- [ ] Keep `KIND_DISCLOSURE_MAX_PAGES = 40` and the `> 40` rejection. The extra
  probe proves whether exactly 40 stored pages are complete.
- [ ] Add required `termination` deserialization to `kind-raw` and reject a
  missing, unknown, or non-clean value before `build_captured_pages` and before
  a `RawStore` can publish anything.
- [ ] Update every fixture/call site so completeness cannot be omitted at
  compile time.

### Step 4: Prove fail-closed storage behavior

- [ ] In `kind-raw` unit tests, cover missing/unknown/all three incomplete
  values and one clean value.
- [ ] In `kind_raw_ingestion.rs`, assert each incomplete value leaves no
  provider directory, batch directory, or manifest row.
- [ ] Assert clean termination with 40 pages succeeds and 41 stored pages still
  fails with `TooManyPages`.

  ```sh
  cargo test -p collectors --locked --bin kind-raw
  cargo test -p market-data --locked --test kind_raw_ingestion
  ```

**Acceptance:** A timeout, missing control, or result larger than 40 pages can
never produce exit 0 or a visible Raw batch. A byte-duplicate terminal response
is the only clean end. Existing termination-less staging is intentionally
rejected and must be recaptured; already immutable Raw batches are not edited.

**Suggested commit:** `fix(kind): require proven capture termination`

---

## Task 2: Validate the KIND placeholder row (F2)

**Files:**

- Modify: `crates/market-data/src/kind_normalize.rs`
- Modify: `crates/market-data/tests/kind_normalization.rs`

- [ ] Add a fixture builder that can omit the placeholder without changing the
  production parser.
- [ ] Add RED cases where the first row is a valid-looking data row, contains a
  `<td>`, or contains a `<th>`. Assert a typed placeholder error and no
  normalized output.
- [ ] Replace the unconditional `rows_iter.next()` with validation that the
  first row has no data/header cells. Do not include the row HTML in the error.
- [ ] Preserve the existing zero-based `source_row_index` for real data rows and
  assert normal fixtures retain their count and order.

  ```sh
  cargo test -p market-data --locked --test kind_normalization
  ```

**Acceptance:** No first data row can be silently discarded on any page. A
valid empty placeholder remains accepted without changing row ordering.

**Suggested commit:** `fix(kind): validate disclosure placeholder rows`

---

## Task 3: Replace vacuous KIND ingestion assertions (F3)

**Files:**

- Modify: `crates/market-data/tests/kind_raw_ingestion.rs`

- [ ] In the byte/hash test, assert `pages.len() == entry.files.len()` before
  `zip`, then retain the byte-exactness and independently computed SHA-256
  assertions for every pair.
- [ ] In the credential-like field test, remove the empty filesystem sweep.
  Rename it to describe credential-like **field-name rejection**, assert the
  exact `CredentialLikeFormField { page_index, field_name }` variant, and keep
  `assert_nothing_written`. Do not claim that this test proves value redaction:
  the error type never carries the submitted value, so a sentinel-absence
  assertion over its rendering would also be vacuous.
- [ ] Rename comments/test names so they describe the executed properties, not
  a zero-iteration sweep.

  ```sh
  cargo test -p market-data --locked --test kind_raw_ingestion
  ```

**Acceptance:** Every load-bearing loop proves it has the expected cardinality,
and the credential-like field test fails if validation stops rejecting the
field or if any storage becomes visible.

**Suggested commit:** `test(kind): make raw integrity checks non-vacuous`

---

## Task 4: Fix and expose the Paper preview path regression (F4)

**Files:**

- Modify: `crates/job-queue/tests/paper_preview.rs`

- [ ] Change only the missing-close deletion target to
  `CurateStore::new(&fixture.dataset_root)`.
- [ ] Assert the selected path is an existing file immediately before deleting
  it, so an ENOENT cannot masquerade as the intended business failure.
- [ ] Extract a test-only deletion-path helper used by the DB test and add a
  non-DB unit test proving the writer and deletion target agree. Assert the
  doubled `curated/curated` path does not exist.
- [ ] Do not change the separate manifest-path conventions identified at the
  review's lines 379 and 698.

  ```sh
  cargo test -p job-queue --locked --test paper_preview missing_close_deletion_path_matches_fixture_writer_without_db
  ```

- [ ] In the approved QA PostgreSQL lane, run the full test and require it to
  reach the `PAPER_PREVIEW_CLOSE_MISSING`, permanent-failure, and zero-output
  assertions. A `DATABASE_URL` skip is a failure of the gate, not a pass.

  ```sh
  cargo test -p job-queue --locked --test paper_preview missing_close_fails_preview_permanently_without_outputs -- --nocapture
  ```

**Acceptance:** The regression is caught without a database, and the complete
business path passes with the QA database without ENOENT.

**Suggested commit:** `test(paper): use the fixture dataset root for missing close`

---

## Task 5: Bound and type-check KIND staging page reads (F5)

**Files:**

- Modify: `data-pipelines/collectors/src/bin/kind-raw.rs`

- [ ] Add a `1 MiB` per-page constant and typed errors for symlink,
  non-regular file, and oversize file.
- [ ] In the execute-only read helper, call `symlink_metadata`, reject symlinks,
  require a regular file, and compare metadata length before allocating/reading.
  Preserve `--plan`'s contract that it does not open HTML bodies.
- [ ] Add tests for:

  - symlink to a regular file (`#[cfg(unix)]` where required);
  - referenced directory/non-regular entry;
  - sparse file one byte above the limit;
  - file exactly at the limit;
  - the existing normal two-page fixture.

  ```sh
  cargo test -p collectors --locked --bin kind-raw
  ```

**Acceptance:** Static untrusted staging input cannot trigger an unbounded read;
all rejection happens before Raw storage. Document that concurrent replacement
of staging entries is outside this patch's threat model; if it becomes in
scope, follow with fd-based `O_NOFOLLOW`/bounded-read work for each platform.

**Suggested commit:** `fix(kind): bound untrusted staging page reads`

---

## Task 6: Verify OpenDART response page identity (F6)

**Files:**

- Modify: `crates/market-data/src/providers/opendart.rs`
- Modify: `crates/market-data/tests/opendart_raw_ingestion.rs`

- [ ] Add a typed numeric-only `ResponsePageMismatch { requested, response }`
  error.
- [ ] Preserve the existing documented number-or-digit-string parsing rule,
  convert safely to `u32`, and compare response `page_no` with the requested
  page before accepting rows.
- [ ] Add a two-page fixture whose second response claims page 1 and differs in
  bytes from the first. Assert mismatch and no Raw/manifest publication.
- [ ] Retain the total-count, total-page, bound, and duplicate-byte tests.
- [ ] Correct the stale provider `Debug` comment while this file is open; do not
  change the opaque formatting implementation.

  ```sh
  cargo test -p market-data --locked --test opendart_raw_ingestion
  cargo test -p market-data --locked providers::opendart
  ```

**Acceptance:** Jittered copies of a logical prior page cannot be represented as
distinct complete pages. No error carries response bytes or provider prose.

**Suggested commit:** `fix(opendart): verify disclosure response page identity`

---

## Task 7: Make the remaining tests and diagnostics truthful (F7, F8, low)

### Task 7A: Test actual OpenDART `--plan` output (F7)

**Files:**

- Modify: `data-pipelines/collectors/src/bin/opendart-raw.rs`

- [ ] Extract a pure `render_plan` used by `run_plan`; retain current stdout
  content and no-network behavior.
- [ ] Remove only the type-system-guaranteed sentinel assertion from
  `query_parameter_names_never_include_values`.
- [ ] Render a plan using sentinel `corp_code`, `bgn_de`, and `end_de` values.
  Assert expected parameter names and safe configuration fields are present,
  while every supplied value is absent from the complete rendered text.

  ```sh
  cargo test -p collectors --locked --bin opendart-raw
  ```

**Suggested commit:** `test(opendart): cover rendered plan redaction`

### Task 7B: Align the malformed-Parquet test with attestation (F8)

**Files:**

- Modify: `crates/job-queue/tests/recommendation_compute.rs`

- [ ] Rename the test to
  `malformed_parquet_fails_artifact_attestation_as_integrity`.
- [ ] Retain the stale manifest and arbitrary invalid bytes.
- [ ] Assert both `ErrorClass::Integrity` and a positive, stable
  artifact-attestation marker. Do not call `resync_manifest_artifacts` and do
  not bypass `verify_artifacts`.

  In the existing CI image or another isolated environment containing the
  pinned `pyarrow==25.0.0` and `uv==0.12.1`, run:

  ```sh
  cargo test -p job-queue --locked --test recommendation_compute malformed_parquet_fails_artifact_attestation_as_integrity
  ```

**Suggested commit:** `test(recommendations): name the malformed artifact boundary`

### Task 7C: Correct OpenDART transport classification (low)

**Files:**

- Modify: `crates/opendart-client/src/transport.rs`
- Modify: `crates/opendart-client/src/error.rs`
- Modify: `crates/opendart-client/src/status.rs`
- Modify: `crates/opendart-client/src/client.rs`

- [ ] Classify `is_connect()` as `NeverSent`, then genuine `is_timeout()` as
  `TimedOut`, and all other send errors as a new terminal coarse failure such
  as `Indeterminate`. Discard the original `reqwest::Error` without formatting.
- [ ] Add a corresponding public error variant with no string/bytes payload.
  Update the exhaustive no-leak sample/match so adding future variants still
  fails compilation until audited.
- [ ] Assert the new class is not retryable and a fake client attempts it only
  once.

  ```sh
  cargo test -p opendart-client --locked
  ```

**Suggested commit:** `fix(opendart): distinguish indeterminate send failures`

### Task 7D: Canonicalize credential-file whitespace (low)

**Files:**

- Modify: `crates/opendart-client/src/credential.rs`

- [ ] Replace trailing-only trimming with surrounding whitespace trimming.
  Keep empty-after-trim as `FileEmpty`.
- [ ] Add fixtures for leading/trailing whitespace and whitespace-only files.
  Assert only the canonical secret reaches the secret wrapper; never render the
  sentinel.

  ```sh
  cargo test -p opendart-client --locked credential
  ```

**Suggested commit:** `fix(opendart): normalize credential file whitespace`

## Task 8: Reconcile documentation and run the closure gate

**Files:**

- Modify: `docs/runbooks/stage6-source-contracts.md`
- Modify: `docs/decisions/0004-stage6-official-source-contracts.md`
- Modify: `docs/STATUS.md`
- Review: `data-pipelines/kind-capture/README.md`
- Review: `docs/reviews/2026-08-20-stage6-disclosure-review.md`

- [ ] Update the KIND contract in the runbook, ADR, and STATUS, and verify Task
  1 already made the capture README consistent:

  - four termination values and the single clean value;
  - two bounded waits per page;
  - 40 stored pages plus one terminal probe;
  - non-zero incomplete capture and ingest rejection;
  - recapture requirement for old termination-less staging;
  - no claim that `번호` is a total count;
  - ordered decoded form-field semantics.

- [ ] Add a concise F1-F8 resolution table to STATUS or a dated resolution
  appendix. Keep the original review findings intact as historical evidence.
- [ ] Do not claim live verification, DB verification, or full workspace green
  unless the corresponding command actually ran without a skip.

### Focused closure gate

```sh
(cd data-pipelines/kind-capture && npm test)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p opendart-client --locked
cargo test -p market-data --locked --test kind_raw_ingestion --test kind_normalization --test opendart_raw_ingestion
cargo test -p collectors --locked --bin kind-raw --bin opendart-raw
cargo test -p job-queue --locked --test recommendation_compute
```

### Environment-backed closure gate

- [ ] With `pyarrow==25.0.0` and `uv==0.12.1`, run the complete recommendation
  suite.
- [ ] Through the approved QA DB lane, run `paper_preview` and fail if it emits
  a skip marker.
- [ ] When all documented prerequisites are present, run:

  ```sh
  cargo test --workspace --locked --no-fail-fast
  ```

  Classify unrelated pre-existing/environment failures explicitly; do not hide
  them by changing Stage6 code.

### Mutation checklist

Temporarily apply one mutation at a time, require it to compile, and require the
paired test to fail:

- accept `page_bound_reached` as clean;
- remove the page-41 probe distinction;
- restore unconditional first-row discard;
- temporarily make manifest construction omit one file entry so the
  pages/files cardinality assertion must fail;
- restore the doubled F4 deletion root;
- skip the staging size/symlink check;
- discard response `page_no` again;
- append one query value to rendered plan output;
- relabel indeterminate send failure as timeout.

Revert each mutation through a normal patch before continuing. Do not use a
destructive worktree reset.

**Final acceptance:**

- F1-F8 and all four low items have a linked code/test or documentation change.
- The HIGH path cannot publish a truncated KIND prefix.
- Every new failure is typed, fail-closed, and secret/body-free.
- Focused offline suites, format, and clippy are green.
- DB and Python-dependent results are reported honestly as PASS or explicit
  environment blocker; no skipped suite is presented as proof.
- No live provider or order-capable path was called or enabled.

**Suggested commit:** `docs(stage6): record disclosure review remediation`
