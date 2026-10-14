# WP-4/I0b-I1b-S1-F-R — Wave18 caller integration source review

2026-10-14, Asia/Seoul. R2-04/09, direct review in
`/data/worktrees/3puw275b/excited-dolphin`, branch excited-dolphin.
Only this report was written. No delegation was used. Coordinator owns acceptance
and subsequent exact-candidate hosted CI.

**No concrete new defect was identified in the scoped three-file delta.**
The explicit classifications, static diagnostic mapping and bootstrap count agree
with the coordinator-fixed contract. The new regression exercises all eleven
variants and both recursive wrappers through the required paths. These are source
conclusions, not compiler/lint/test PASS or acceptance of the broader R2 work.

## Frozen inputs, comparison and failure evidence

Read root AGENTS.md, ROADMAP R2-04/09 and complete Wave17/18 briefs/recovery in the
implementation plan. Read the two fixed CI reports, private input inventory and
relevant current type/reexport/call paths. Compared the complete three-file delta
against `f2dc593`, the documentation baseline with unchanged f9 candidate product
bytes. Entry HEAD was `8de8cff6e451383533f512fe7b0d4cdecb9b60fe`.
Hashes identify the review even if documentation HEAD advances.

| Target | Lines | SHA-256 |
| --- | ---: | --- |
| [pipeline.rs](../../data-pipelines/collectors/src/pipeline.rs) | 1226 | `b31d2e830e10f17212826cad1f990203a437575e0d4cddc23aab2ec9954c2633` |
| [worker.rs](../../data-pipelines/collectors/src/worker.rs) | 6797 | `9b03eef5aa5f3966d9d06c64bd0a2f42fc1e665457e3162e44d225cf7a3cf15a` |
| [migrate-static-check.sh](../../deploy/db/migrate-static-check.sh) | 264 | `1a5c8411b7496c40720ed0691097889fb2b6ec6be82178eac7da230bde747589` |

All target pins matched at entry and final. Baseline Git blobs were checked afresh:
pipeline1204 lines/`fa01d39af3df15a7cdd5c206adc2f1baee712f39626f923ec7ba4024aed57ffd`;
worker6701 lines/`de565faa0adf5bb1ba31ec5bf76784875a54bf12ae806e9864050b14e9c09e09`;
shell264 lines/`d65f82ea5482dd5725d211eacd9a83641a3f816b4acff8633dd46ee96e32ff8e`.
The input inventory is137 lines, SHA-256
`a608e8ede340336decec003cca48c66a7893ea9d0e7994d45cdc5120bc6d250e`, at
target/r2-daily-beta/wp-4-integration/source-wave-18-inputs.json.

All25 read-only reference files matched their inventory line/hash pairs and were
byte-identical to the baseline. Relevant content inspection was restricted to
storage/normalizer types, collectors reexports, provisioner/Compose source entries
and the target functions/tests. Other pinned references were checked by hash/bytes,
not re-reviewed for architecture. Important compatibility pins are:

| Reference | Lines | SHA-256 |
| --- | ---: | --- |
| crates/market-data/src/storage.rs | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` |
| crates/market-data/src/normalize.rs | 2019 | `be5b4d3444b355c40295ca33bdc9d40e3aa1e359146fcaca293b4a1f745cd6b9` |
| data-pipelines/collectors/src/lib.rs | 55 | `be672270c0f88f6d11ad2ac9381c0b52d9bf12f6d7e4048fc5c1c151b5a406fa` |
| deploy/secrets/provision-runtime-secrets.sh | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` |
| deploy/compose/compose.yml | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` |

All four pinned failure artifacts also matched:

| Evidence | Lines | SHA-256 |
| --- | ---: | --- |
| [role/Raw CI report](2026-10-14-r2-role-raw-ci.md) | 27 | `fbe0d8930036cbc41e9a9e73c0e928c8df32f73327027b5ae25a90229f9f61d4` |
| [Diagnostic supplement](2026-10-14-r2-role-raw-ci-diagnostics.md) | 15 | `16eca425449e48d3a39e370e6d5c456bf5d8c329bf318b3902df4b973be4fcb0` |
| target/r2-daily-beta/ci/f9dfffb/ci-evidence.json | 1704 | `ac8e2b13f8ab2412ffbe150bfce702120da6da646d41ae6ac2117e0692761261` |
| target/r2-daily-beta/ci/f9dfffb/diagnostics/diagnostic-evidence.json | 201 | `0a1d2394cd1ecc350a76a707b29d452e5aab5605a9fd3b8431d49db2f92a9d46` |

The supplement establishes the same three primary E0004 errors in Clippy, chart
compilation and PG's retained api-tenancy-rls log. PG outer exit1 is emitted; nested
Cargo exit is unknown. The prior run38530582876 for
`f9dfffb9a5d4bfc38a9df347af0de154dbe6d11b` remains terminal failure. Its30 wire,
18 Linux Raw and chart tests did not execute. The policy failure was the expected7
bootstrap count against existing8 source entries. Supplemented error-count/chart/
inventory-link fields supersede those specific original receipt omissions; the
original report/receipt were not modified. No new run or compiler output was queried.

## Integration dispositions

| Integration | Source disposition | Exact anchors and consequences |
| --- | --- | --- |
| Pipeline classification | Matches fixed contract; execution pending | [store_failure_class:279](../../data-pipelines/collectors/src/pipeline.rs#L279) and [normalize_store_failure_class:347](../../data-pipelines/collectors/src/pipeline.rs#L347) each explicitly add all11 variants to Permanent. New manifest limits/metadata failures cannot select Retryable through those paths. |
| Worker diagnostic and regression | Matches fixed contract; execution pending | [normalize_store_error_code:581](../../data-pipelines/collectors/src/worker.rs#L581) adds the same11 variants to existing KIS_NORMALIZE_INTEGRITY_FAILURE. [New test:6401](../../data-pipelines/collectors/src/worker.rs#L6401) checks both public pipeline classifiers and normalize_diagnostic for eleven owned values plus two representative wrappers. |
| Static bootstrap cardinality | Matches fixed source contract; checker execution pending | [Shell:46](../../deploy/db/migrate-static-check.sh#L46) changes only bootstrap expected_count7→8. db-migrate remains1; UID/GID/mode/root prohibitions and all other checks remain byte-identical. |

There are no prioritized new findings or severity assignments in this bounded delta.
No wildcard match, lint allowance, public API/error code or reader activation was
introduced. Explicit coverage removes the cited missing-match source conditions;
only fresh compilation can establish that the candidate builds successfully.

## Classification, diagnostic and test semantics

The required eleven variants occur once each in all three relevant match functions:

| StoreError variant | Written constructor in new test |
| --- | --- |
| InvalidCommittedManifestReadLimits | Static harmless reason marker |
| CommittedManifestReadLimitOverflow | Unit variant |
| CommittedManifestReadAllocationFailed | requested_bytes=usize::MAX metadata only |
| CommittedManifestReadByteCountOverflow | Unit variant |
| CommittedManifestReadLineCountOverflow | Unit variant |
| CommittedManifestReadRecordCountOverflow | Unit variant |
| CommittedManifestByteLimitExceeded | max_bytes=128 |
| CommittedManifestRecordLimitExceeded | max_records=8 |
| CommittedManifestRecordScopeMismatch | Harmless path marker and line9 |
| InvalidCommittedManifestRecord | Harmless path marker and line10 |
| CommittedManifestRecordConflict | Harmless path marker and line11 |

The allocation-failure case constructs an enum field; it does not request an
allocation of usize::MAX. This regression checks classification/diagnostic behavior,
not the reader's allocation or provider/storage implementation.

All existing distinctions are preserved exactly: StoreError::Io remains Retryable
in both classifiers; FileExists remains Permanent at store_failure_class and
Retryable at the normalization boundary. The existing diagnostic codes for Io and
FileExists remain KIS_NORMALIZE_STORE_IO and KIS_NORMALIZE_STORE_FILE_EXISTS.
CleanupFailed follows original; IndeterminateBatchCommit follows source. Both
normalization paths recursively select the inner class/code, without allowing the
cleanup I/O or outer manifest entry to override it. No retry loop was added.

The shared assertion closure first borrows &StoreError for store_failure_class,
then moves it once into NormalizeError::Store. The frozen
[NormalizeError:124](../../crates/market-data/src/normalize.rs#L124) holds StoreError
by value. It then borrows that owned normalize_error for normalize_failure_class
and normalize_diagnostic, retaining it until all diagnostic assertions finish.
There is no use-after-move or diagnostic reference outliving its owner.

The array has homogeneous StoreError elements with valid field types. IntoIterator
moves each non-Copy value once; the closure has no consumed capture and can be called
again for wrappers. CleanupFailed uses the exact boxed original and io::Error cleanup
fields; IndeterminateBatchCommit supplies a complete ManifestEntry and boxed source,
matching frozen [storage types](../../crates/market-data/src/storage.rs#L190).
The synthetic entry is metadata only: UUID/date/time construction, strings, mode
and an empty files vector do not activate a provider, entitlement or reader.

[FailureClass:20](../../data-pipelines/collectors/src/pipeline.rs#L20) implements
Debug/PartialEq/Eq; [lib.rs:25](../../data-pipelines/collectors/src/lib.rs#L25) reexports
it and both public classifier functions. Internal process_tests can access the crate
pipeline module and its parent's private normalize_diagnostic. WorkerDiagnostic has
Debug and the four optional fields asserted by the test. No requirement for cloning
StoreError or deriving Debug for Raw evidence records was introduced.

For each of the13 values (11 direct plus both wrappers), assertions require:
Permanent through both classifiers; exactly KIS_NORMALIZE_INTEGRITY_FAILURE;
endpoint/http_status/response_kind/file_name all None; and absence of harmless reason,
path and cleanup markers from formatted WorkerDiagnostic. Only the bounded diagnostic
is formatted, not StoreError/NormalizeError. The Store normalize_diagnostic branch
returns only the selected static code and empty optional fields; outer metadata and
free-form reasons cannot be copied by this unchanged branch.

The regression is meaningful: omitting/misclassifying a variant, altering wrapper
recursion, or forwarding an optional diagnostic field changes its assertions. It is
not a test of actual reader failures, worker retries or full event delivery. Existing
Io/FileExists/wrapper and other tests were preserved; no existing test was weakened.
No concrete new strict-lint/type issue was identified from this source inspection;
mechanical formatting and source reasoning are not compilation or Clippy evidence.

## Shell count compatibility and preservation

The sole shell replacement is expected_count7→8 for db-role-bootstrap. Inert source
counting confirms eight add_copy bootstrap entries and one migration entry at
[provisioner:135](../../deploy/secrets/provision-runtime-secrets.sh#L135), each with
999/999/0400. The corresponding frozen Compose source has eight bootstrap long-syntax
mounts and one migration mount, each with uid/gid999 and mode0400, starting at
[Compose:287](../../deploy/compose/compose.yml#L287) and
[370](../../deploy/compose/compose.yml#L370). No credential fixture value was read
or printed for this comparison.

The checker retains its Dockerfile USER999:999 assertion, prohibition on root-owned
mounts, per-service uid/gid/mode counts, distinct bootstrap/migration credential checks
and all other existing assertions. These services inherit the image user; the source
does not need a Compose user override. No Dockerfile/image/runtime verification is
claimed or added to the review scope. No script was executed beyond bash syntax check.

Removing only the11 added arm entries from each affected Rust match and the single
new test yields byte-identical baseline collector files. Thus all other code,
diagnostics, public signatures and tests are preserved. The shell is byte-identical
after reversing its one expected_count replacement. The entire pinned read-only
set, including storage/normalize/role/secret/Compose/wire/workflow sources, is unchanged.

## Actual checks and unresolved verification

Actual checks: complete exact delta/source inspection; three target hash/count and
baseline-blob checks; all25 reference hash/count/baseline-byte checks; four failure
artifact pins; enum-arm inventory and new-test constructor/type/borrow inspection;
byte-preservation comparison; inert provisioner/Compose source cardinality checks;
scoped edition2024 rustfmt --check with skip_children=true on both Rust files (exit0);
bash -n on the shell target (exit0); scoped diff --check against f2dc593 (exit0);
report local-link/line-bound/whitespace and final frozen-input checks.

One initial inert assertion incorrectly expected a Compose user override. Source
inspection showed the checker enforces the image USER999:999 instead. Correcting
the review assertion produced matching8/1 source entry and mount UID/GID/mode counts;
no product source defect or execution failure was inferred from that checker mistake.

Own memory context returned MCP_ERROR/no response, so no context result was used.
Own finish returned SKIPPED, saved=false, incomplete=false with no_durable_change.
Only the supplied hook performed loopback/operational-receipt work; no stale envelope,
other session or Basic Memory was used.

Unresolved source findings: none identified. Follow-up: coordinator decides source
acceptance, then prepares one fresh exact-candidate hosted CI after existing-run
query. The old failure remains preserved. The written new unit test needs a direct
result row alongside the prior30 wire/18 Linux Raw names and chart tests; nothing
here proves any of those executed on the corrected candidate.

Deviations from authorized task scope: none. Required inputs not found: none.
Missing/conflicting requirements requiring invented policy: none identified.
Other unreported missing evidence: none. Not verified: current compiler/Clippy/tests,
actual shell checker execution, updated hosted CI, full event/retry behavior and
coordinator acceptance. The old PG nested Cargo exit remains unknown. Broader R2/I0,
migration0060, pool/GUC, full snapshot, runtime/provider/production and Owner acceptance
are outside scope and unverified.

Only the assigned report was written. No source/other-document/private-receipt/Git
write, parser/checker/validator/provisioner/self-test execution, Cargo/rustc/Clippy/
tests/build, SQL/DB/root/Docker/provider/network/production/workflow/lifecycle action,
dependency install or delegation was performed. No auto-review denial occurred and
no permission mode was changed. Background work and pending permissions: none.
Further decomposition needed: none. Exact report line range/hash are in the handoff.
