# R1 same-commit hosted acceptance

Status: **HOSTED ACCEPTANCE PENDING — C2 passed Rust, DB, policy and functional smoke; the reviewed two-file Python/Web follow-up now passes local validation.**
Local validation and hosted validation are recorded separately. No production
rollout or R1 completion is claimed.

## Candidate C — 79da119e6e8b52be424ec8132ac2a52f65726471

The coordinator committed the 146 reviewed paths, verified a clean checkout,
pushed `orange-dugong`, and checked that the remote ref matched C before
dispatching both workflows. Both Actions runs report C as their head SHA:

- [CI 38258860613](https://github.com/sihioov/Lagrange/actions/runs/38258860613).
- [Research worker smoke 38258865310](https://github.com/sihioov/Lagrange/actions/runs/38258865310).

The first hosted candidate is not accepted. Completed-job logs were recovered
through the job-log API while the parent run remained active; inability to
retrieve them through `gh run view --log` was not treated as missing evidence.

| Job | Observed result | Follow-up |
| --- | --- | --- |
| format, clippy | Passed | Preserve the result as C evidence |
| policy | Missing `RESEARCH_ENTITLEMENT_SHA256` in the code-commit static check's Compose config invocation | Add the same scoped inactive hash used by the candidate static check; keep production interpolation required |
| web | Install, lint, typecheck, unit, build and 31 runner regressions passed; wrapper then reported `task_target_unavailable` before inventory/browser startup | Create the absent private `target/` in a fresh checkout, preserving existing symlink and non-directory rejection |
| python-tests | Runner shutdown signal during pytest, followed by operation cancellation; no final test result | Preserve the infrastructure interruption; a later same-candidate complete run is still required |
| postgres-integration-validation | Staged/full migration lanes passed; migration-contract reported 35 pass, 1 fail, 0 ignored | `identity_provisioning_is_atomic_and_role_scoped` observed `22P02` instead of the required `55000` rollback refusal; diagnosis pending |
| workspace-tests | Failed at `historical_price_only_artifact::tests::unsafe_writer_and_reader_do_not_write`; the library reported 195 pass, 1 fail, 0 ignored | Diagnose the temporary directory's permission precondition without weakening `UnsafePath` or no-write assertions |
| required | Failed after the failed prerequisite jobs | Candidate C is not accepted |
| research-smoke | Cancelled at the 40-minute job limit during the second serial Rust image build; no functional verdict | Increase the bounded hosted time allowance while retaining both cold builds and functional checks |

Retained job-log SHA-256 values: policy
`535084449fde14d75bd7c5f088179b27420afa3c2b220e02aea6c92ee5b90fbd`;
Python `c387cb1038986ab9dd8e47cb074a87c48855e163984185e0528161ae24cf70e1`.
Private receipts and bounded logs are under
`target/r1-runtime-fxi6ltmv/hosted-79da119/`.

## Local results supporting the first candidate

- [Rust workspace](2026-10-12-r1-workspace-validation.md): 2,833 outer binary
  passes, 1 nested child pass and 52 doc-test passes; no failures, 6 existing
  ignored cases and 180 filtered. Six strict Clippy selections passed.
- [DB validator](2026-10-12-r1-postgres.md): 122 pass, no failure/ignored/SKIP;
  staged upgrade and the 59-migration full lane with matching no-op rerun.
- [Web](2026-10-12-r1-web-validation.md): 135/135 browser executions across
  134 unique synthetic identities, 31 runner regressions, no skip/retry/flaky.
  [Actual API](2026-10-12-r1-actual-api-validation.md) is a separate two-phase lane.
- [Python](2026-10-12-r1-runner.md): 450 pass, 10 documented existing skips;
  backup policy 6 pass.
- [Functional smoke](2026-10-12-r1-smoke-validation.md): script and supervisor
  exit 0, functional PASS, no SKIP; 933 unchanged inputs and owned cleanup.

These local results do not replace the failed or unfinished hosted gates.
Any source repair produces a new candidate; both required workflows must then
complete on that same new commit.

## Bounded follow-up validation

The Compose checker now supplies an inactive synthetic entitlement hash only to
its two `config` calls; production interpolation remains required. The Owner V2
static checker now matches the existing Dockerfile build's explicit
`market-stream-provisioning` feature. The Web runner creates a missing private
`target/` and retains rejection of existing symlinks and non-directories.
Independent source review found no blocking issue in these four changed files.

The local policy sequence passed 14 of 16 commands initially. The remaining
operations and secrets checks subsequently passed in fresh detached C checkouts
created under umask 022, with the corrected operations marker applied where
needed. Git-created executable modes were 0755; the original checkout's 0775
modes were left unchanged. The secrets check's first sandbox-only fakeroot
attempt failed; its single native continuation passed the synthetic ownership
fixture. These results cover the individual policy commands; a new complete
hosted policy job remains required.

The modified runner's 31 regressions passed with zero skips in the same clean
checkout setting. Temporary worktrees were verified clean after restoring only
their owned changes and removed without force. No production secrets, services,
or file modes were changed. Private command/log/hash receipts are retained under
`hosted-79da119/clean-policy-runner/` within the evidence directory above.

The complete workspace log contains exactly one failed target and one failed
test. Source review established that the unsafe-root test relied on tempfile's
umask-dependent default mode. Its bounded repair explicitly sets the owned Unix
root to 0770, preserves exact `UnsafePath` and no-write assertions, and aligns
both non-Unix API expectations with `UnsupportedPlatform`. Only that existing
test changed. Independent source review found no issue. The exact case passed
under launch umasks 022 and 077, the full library passed 196 tests with zero
failures/ignored, and all-target/all-feature package Clippy passed with
`-D warnings`. Source and lock hashes stayed unchanged during validation.
Non-Unix execution remains unverified. The original hosted return variant was
not logged and is not inferred as an observed value.

All C checkout jobs were confirmed at C. The workspace command has 235 passing
or failing summary rows, summing to 2,885 passes, 1 failure, 6 ignored and 180
filtered. The separate chart HTTP gate adds 6 passes; these aggregate summary
counts are not a deduplicated test count. The independent source inventory
confirmed exactly one failed target and test.

The final smoke check annotation explicitly states that the 40-minute maximum
was exceeded. The research-worker image finished at 14:19:57 UTC after roughly
25 minutes of Rust compilation; cancellation occurred at 14:33:36 during the
candidate-runner image build. The workflow now allows 90 minutes for both serial
cold builds and the remaining functional work. Concurrency, Cargo jobs, cold
cache requirements and functional assertions are unchanged; the longer allowance
does not itself establish a successful run. Annotation receipt SHA-256:
`545e6574d20277c0237a55a280ab4824f03edf23b47f2cc2481273e18b6bdb96`.

The smoke timeout change passed independent source review and the existing CI
contract suite (10 pass, no failures or skips).

The single disposable DB diagnostic completed all six probes: both predicates
failed with `22P02` only for an EMPTY actor, while MISSING and a valid reserved
actor succeeded. The original test's pinned connection started MISSING, reached
the intended migration 40 `55000` refusal, and was EMPTY after the earlier down
transactions. The test itself passed; this proves the session-state hazard,
not the exact historical hosted backend state.

The coordinator adopted a test-fixture repair: both rollback calls now use a
dedicated migration-owner connection pool initialized with the typed, validated
scheduler actor. The same-backend advisory unlock, exact `55000`, invitation and
audit cleanup, function removal and replay remain. An additional assertion
requires the refusal to originate from migration 40. No migration or production
role/policy is changed. Independent source review found no issue. Diagnostic receipt SHA-256:
`ad33c5fe15afd9ec5281b264f1e607113dc7af8af2a0d1f89717d1a5a88eb333`.

The full validator then exited 0 with `APPROVED`: all ten selections completed,
126 passed, zero failed or ignored, and zero `SKIP:` markers across 87 runtime
logs. This count includes four matching auth library tests as well as the auth
integration tests; earlier reports' 122-count receipts remain historical.
The migration-contract suite passed all 36 tests, including the repaired exact
rollback guard. Staged 0038–0041 checks, the empty full-release 59-entry ledger,
matching checksums and the no-op rerun passed. Formatting and strict all-target,
all-feature migration-contract Clippy also exited 0.

All 1,037 captured source inputs stayed unchanged. The coordinator verified the
102-entry evidence manifest and original exit receipts; owned containers,
networks, volumes, temporary directories and child processes were removed and
the reserved ports released. Evidence manifest SHA-256:
`23c72b1dc6b32ee42daed1654530335a0db320544839139a02495b3a7728400f`;
validator evidence SHA-256:
`c4a6be026b37c67c2b48f1d6127133fb23e1aa4a9e3906bd5923e91a4bd90dae`.
Private report: `hosted-79da119/db-followup-validation.md`; evidence directory:
`target/r1-runtime-fxi6ltmv/evidence-hosted-db-followup.x5k0q1n3/`.

These results accept the seven-file follow-up locally. Candidate C2 is the
commit adding this record and the reviewed repairs; its new hosted CI and smoke
results must be recorded separately before R1-08 can be checked. Release images
and an immutable production manifest are still outside this acceptance.

The retained workspace job log has SHA-256
`f3dbb7c6c15512c49703a9818ffd8503c9735802ec0faa3a059041ce50d65266`.

## Candidate C2 — f5dff910c53be55c11ae5a69a640052f653c352d

Recovered 2026-10-13 KST. [CI 38266715994](https://github.com/sihioov/Lagrange/actions/runs/38266715994)
completed with failure. [Research smoke 38266740196](https://github.com/sihioov/Lagrange/actions/runs/38266740196)
completed successfully on the same C2 commit. Every checkout job matched C2.
No run was retried or cancelled during recovery.

| Gate | C2 result | Evidence and limits |
| --- | --- | --- |
| format / Clippy | Success | Both hosted jobs completed on C2. |
| policy | Success | All 26 steps succeeded, including the ten-test CI contract suite. |
| workspace-tests | Success | Chart acceptance: 6 passes, separate from the workspace command. Workspace: 235 result summaries totaling 2,886 passes, zero failures, six ignored and 180 filtered. The log separates 2,834 binary-summary passes and 52 rustdoc passes; this is not a deduplicated test-identity count. |
| PostgreSQL | Success, APPROVED | All ten selections and the runtime no-SKIP gate succeeded. Full 59-migration ledger/checksum comparison and no-op rerun passed. Hosted output does not expose per-selection test counts, so the local 126 is not attributed to this run. |
| Python | Failure; pytest cancelled | Dependency and Phase0 setup succeeded. Pytest reached the 73% progress line before a runner shutdown message; no final pytest count exists. The backup-policy step was not reached. |
| Web | Failure before browser execution | OpenAPI, lint, typecheck, 61 Vitest files / 487 tests and build passed. Two subcases of one cleanup regression failed among 31 unittest methods. Browser inventory and the required 135 executions were not reached. |
| research-smoke | Success; functional PASS | The job ran approximately 52 minutes and emitted static and functional PASS without a functional skip. Its normal-exit trap suppresses teardown errors; no independent hosted resource-removal receipt exists. |
| required | Failure | Python and Web prevent C2 acceptance. |

Detailed bounded receipts are retained in
`target/r1-runtime-fxi6ltmv/hosted-f5dff91/`. Exact job-log SHA-256 values:

- Workspace: `bc08f2e580d32cc6cfa054acbf1d9a5af279db8f46eede20f6076197b0132ad6`.
- PostgreSQL: `f341387d732b1dcf20d56763daa92d0e01380a14b08c7da816726a283bed2738`.
- Python: `faa7c6c6e582960053eddca2ddd28ff1a468d65e1e08c8c9d8da5d449c5b20a5`.
- Web: `a2131930e6d1020f0e62a656bedfba55e2751ae2cf463fd34036ebe2b5b54387`.
- Smoke: `9001483ec3db6e6d4d70ddd2638649d0cb2c25ed88a7fb41dbfe7be345d29d85`.

### Python ownership counterexample and bounded repair

Source analysis found that an unrelated direct child could be mistaken for an
adopted descendant. Its caller session then authorized registering and signaling
the pytest process itself. A multiprocessing resource tracker supplies this
precondition. The original hosted sender, recipient and session topology were
not observed; the earlier shutdown entries describe symptoms, not an established
infrastructure-only cause.

One independently reviewed, signal-denying diagnostic ran against an exact C2
archive with CPython 3.12.13. Ownership masks progressed 0 → 2 → 3: first the
resource tracker, then the supervisor itself. One self-directed TERM reached the
denying hook. Actual signal deliveries were empty. Controller, leaf and tracker
exited 0, the tracker was reaped, and both child identities were absent afterward.
All 1,828 source entries were unchanged. This establishes the local defect, not
retrospective hosted attribution or equivalence to hosted CPython 3.12.15.

The exclusive one-shot marker and original receipts remain in
`hosted-f5dff91-root/python-ownership-proof/` within the evidence root.
Result SHA-256: `ed489e76abf336ee53e62dfca6ec533b87c895c62a1a1b642a303d927b3ceb80`.
Both source manifest SHA-256 values:
`a73a90bb9a208c3d0e97f7f5c1da24217f47c455d1ede26cd6aa146ab28f9668`.
No diagnostic repetition is needed.

The two-file repair excludes the caller PID/session and pre-existing direct-child
generations from ownership authority, registration and final PID/group signaling.
The regression class restores the prior subreaper flag and requires its exact
baseline child generations after each test. A mocked graph test covers rejected
caller/pre-existing identities, contaminated authority and valid detached/root
signaling. All prior 31 methods remain, plus this one new method.

Separately, the cleanup regression expected a cooperative Python TERM handler's
file write within 50 ms. Its reviewed test-only repair uses the unchanged
production 5-second TERM/reap limits and checks writer/root reap plus the exact
write marker at raw-file deletion. Production deadlines and cleanup requirements
are unchanged.

The new mocked graph test passed once (1 test, exit 0). Independent source review
accepted every changed hunk with no findings; all 31 old method names and the 30
unchanged old bodies were verified. The standalone suite passed all 32 methods
with no skips on the native continuation (exit 0, 23.456 seconds including launch).
Its first sandbox attempt could not allocate a loopback port and ran zero tests.
The shared-interpreter pytest command then stopped before collection because the
sandbox made uv's existing cache read-only. Its exact native continuation passed
all 34 selected tests (two multiprocessing cases followed by the 32 runner methods
in one interpreter). Full repository pytest then passed 482 tests, with ten skips
and seven warnings, and backup policy passed 6/6. All three commands exited 0.
The only full-suite selection addition was `PYTEST_ADDOPTS=--ignore=target`, which
excludes ignored local archives/caches absent from a clean CI checkout. No source
tests were excluded. No install, sync, dependency or source change occurred.

The quiet full-run progress contains 492 outcomes. Its ten skip positions map to
the unchanged static inventory: two Windows-only cases and eight root-only
metadata fixtures. All 30 source files preceding the last skip match that
inventory; the new runner test is later. Reasons are inferred from those
unchanged clauses and the earlier `-ra` receipt, not emitted anew by this quiet
run. Mapping receipt SHA-256:
`ac7335d3564ae23ef10905cedb1fd10c5b2a00976c8c33dcc2ebc31f63c68f66`.
The Windows Job Object/secure-handle and root metadata behavior remains untested
in this non-root Linux run. The earlier 450-pass result remains historical.

The current browser wrapper subsequently passed, as recorded below. No NT,
workflow, dependency or provider change is part of this repair. R1-08 remains open.

Standalone log SHA-256:
`2be0de9dc1e4f20a1fdcd5a9e67787b61a675efa0ee7cdc88c72ec86e4f2fe2b`.
Both source pins and the 25 Phase0 files were unchanged during this first stage;
that stage did not retain a full repository Python manifest. The continuation
captured all 124 tracked Python files, both NT dependency manifests and the 25
Phase0 files; its before/after receipts were identical. Manifest SHA-256:
`5cb0d36b2abc337e43fc4a48e0da9a361d55a44ca393fe1ae59aaa1a54f3ef2e`.
The coordinator verified each command exit and log digest. All command children
were reaped. Shared-interpreter log SHA-256:
`b094d585f7a506cb847b3db07b80f598faaf6f7cba5c00bd9ea2f51f8f960963`;
full pytest: `8b5bcf7a20de2a7b0acc9a4f6e3c6535772f51603ff6cb294e1f7a560c94186a`;
backup: `f03d76e3c1728f6dda85bed4b03b99a1b99e8cd329e0f86fff239db4958cc4a2`.
Private reports are
`hosted-f5dff91-root/ownership-repair-review.md` and
`hosted-f5dff91-root/ownership-python-validation.md` within the evidence root.

### Candidate C3 local acceptance

C3 is the commit adding this record and the reviewed two-file repair. The official
wrapper ran once with `--web-port 33020 --api-port 38182`, exited 0 in 300.205 seconds,
and passed its mandatory 32 regressions before completing browser inventory and
all 135 executions: off 121, market-stream 13, separate owner-empty 1. There are
134 unique synthetic identities; owner-empty is the sole cross-phase duplicate.
The dedicated `@actual` test was excluded as required. No failure, skip, retry or
flaky attempt occurred.

The coordinator verified all 12 artifact-manifest entries and the three result
file counts, exit receipts, coverage, source equality and cleanup evidence. All
30 captured source/config/lock/runtime inputs matched before and after. Before
and after manifest files have different enclosing metadata, but identical input
maps. Both reviewed Python pins and the intraday spec pin remained unchanged.
All 345 recorded process generations were absent afterward; the wrapper child
was reaped and the three assigned ports had no TCP rows.

Evidence: `target/r1-web-e2e-1791822453272083578-3495d2b1a987/` and
`target/r1-runtime-fxi6ltmv/hosted-f5dff91-root/ownership-web-validation/`.
Run receipt SHA-256: `2f97cd5c2cce972d41c6844fed9ae2d72f15afdcaea54f4d465e8a0d4ec1da5f`;
artifact manifest: `624a059a72c55dbe935251470bbf507096efb81127e1c7143a6f68a73f34ba3b`;
wrapper log: `756e12591f1562db9e06f8d3de2011b90be77a06f054c480fccf534ab656455a`.

Independent source review and the focused/shared/full Python, backup and Web
results accept this bounded follow-up locally. No unresolved local defect remains.
The coordinator confirmed the review and validation workers were actually idle
with no pending permissions before candidate preparation. Both hosted workflows
must still pass on C3; C2's successes do not establish C3 acceptance. The separate
actual API lane was not repeated because its source was unchanged. Production,
Windows parity and the root-only metadata fixtures remain outside these results.

### Deployment preparation limits

C2's official image builder plan and preflight both exited 0; an independent
source audit found no new release-source defect and matched all 59 up-migration
checksums. No actual release images or immutable manifest were created.

The exact noninteractive root access probe required interactive authentication.
The observed production research-worker was exited 2/unhealthy while Owner V2
was running; the ordinary build-health predicate was not satisfied. No service
change, drain, exception or activation is authorized by these preparation records.
The current-link observation alone does not establish database compatibility,
backup integrity or restore readiness.

R1-09 stays BLOCKED until the final candidate passes CI and a root-capable
operator/session can satisfy the documented build-health prerequisites. The
coordinator will refresh the concrete official build plan for that candidate
before requesting external help. C2's retained proposal is
`target/r1-runtime-fxi6ltmv/release-readonly-f5dff91/candidate-build-handoff.md`;
it must not be reused as proof for a later commit. Production activation remains
outside this R1 task.
