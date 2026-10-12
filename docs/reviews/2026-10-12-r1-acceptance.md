# R1 same-commit hosted acceptance

Status: **HOSTED ACCEPTANCE PENDING — first candidate failed; reviewed follow-up passed local validation.**
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
