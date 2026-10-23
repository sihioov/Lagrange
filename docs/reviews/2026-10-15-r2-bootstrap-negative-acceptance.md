# R2 I0d bootstrap and rollback bounded acceptance

The coordinator accepts **WP-4/I0d and its C01 test-fixture correction** on
2026-10-15, candidate `57ca4d1390b025e7efbe9ac452c614a8b5618889`.
[CI 38672658570](https://github.com/sihioov/Lagrange/actions/runs/38672658570),
attempt 1, completed **SUCCESS at 03:04:41 UTC**. All eight jobs succeeded.
This closes the bounded bootstrap negative/transaction-rollback verification
package, not all I0/runtime work or final R2 acceptance.

## Candidate and complete evidence

Repository sihioov/Lagrange, excited-dolphin, CI/ci.yml, workflow_dispatch.
Remote SHA and an all-event exact-head query returning zero preceded one dispatch
at 02:45:18 UTC. There was no rerun, cancellation or duplicate dispatch. Later
documentation commits do not change this candidate and need no extra CI dispatch.

All eight jobs, 84 step outcomes (83 success, one conditional PG failure-log step
skipped), eight complete logs and seven actual checkout SHAs were recovered.
The required aggregate has no checkout. All 85 input pins and 60 tracked candidate
blobs matched. Each completed log was read once; raw logs remained in memory and
were cleared after sanitized receipts were persisted. The read-only watch exited0.
Heartbeat b6e4cf9e was deleted successfully only after full recovery.

| Evidence | Result |
| --- | --- |
| I0d bootstrap | All 83 individually named cases PASS; no missing, duplicate or unexpected case |
| Affected transport tests | All three named tests PASS, including the previously failed incomplete-frame case |
| Existing R2 Rust inventory | All 99 qualified names have exactly one direct PASS row in the workspace phase |
| Separate chart lane | All six named cases PASS; workspace repeats remain separate evidence |
| Strict Clippy | --workspace --all-targets --all-features --locked -- -D warnings PASS |
| Workspace formatting | cargo fmt --all -- --check PASS |
| PostgreSQL | APPROVED; both 59-checksum passes match candidate blobs, NO_OP, 12 DB labels, catalog/roles/0039/cleanup PASS |
| Web | OpenAPI, lint, typecheck, tests, build and E2E PASS |
| Policy/Python | All wrapper steps and ten printed CI-contract names PASS; Python 485 passed, ten skipped, seven warnings |
| Required aggregate | PASS |

The 99 R2 names comprise 30 wire, 18 S1 Raw, one collector, two helper, 19 private
S2 and 29 integration S2 cases. All 48 Linux S2 definitions therefore passed;
the S2 non-Linux and S1 non-Unix cases remain excluded. The 2,948 direct Rust rows
across phases are not presented as a unique-test count.

## Bootstrap and fixture disposition

The [I0d source disposition](2026-10-15-r2-bootstrap-negative-source.md) and
[C01 source correction](2026-10-15-r2-bootstrap-ci-fixture-correction.md) define
the exact scope. The production bootstrap and transport remain unchanged by
these packages. I0d exercises three positive controls, seven invalid attributes,
two memberships, eight protected-role paths, six CREATE cases, 56 failures across
eight credential files, and one late SQL transaction rollback. Real non-root
unreadability, exact denial reasons, catalog equality, pre-error statement
completion, new-role absence and rollback are checked by the passing fixture.

C01 replaces elapsed-time socket lifetime with bounded subscribe/start,
write-ready and assertion-complete signals in the shared loopback test helper.
Existing receipt/heartbeat/poison assertions and all function names are retained.
Its three direct PASS rows are workspace log lines4025,4027,4029. The previous
Closed failure is preserved in [the original I0d result](2026-10-15-r2-bootstrap-negative-ci.md);
the exact historical thread schedule remains unobserved. This correction and
fresh run close the concrete fixture condition without changing production logic.

PG's seven direct role logins pass in both lanes; initial catalog, 23505 conflict,
four rolled-back 23514 probes, staged rerun and 0039 guard pass. Both daily-admitter
helper labels and no-SKIP enforcement pass. Their unprinted nested test rows and
numeric Cargo exits remain **UNKNOWN_NOT_PRINTED**. QA cleanup succeeds and PG
reports PASS_NO_OWNED_CONTAINERS. Quiet Python individual nodes remain unverified.

No new independent worker review is claimed: these bounded changes were directly
reviewed by the coordinator. No further delegation, local Cargo/compiler/Clippy/
tests, project-helper execution, SQL/DB/Docker, provider or production action occurred.
Local checks were source/static, scoped shell syntax and standalone rustfmt only.

## Retained receipts and next boundary

Private directory target/r2-daily-beta/ci/57ca4d1 is0700; files are0600.

- ci-evidence.json: 4,147 lines, SHA256
  `44608dc770aab1f4d1891120eb7906cd06136b2296df4ce62065b4e500112ed4`.
- pg-evidence.json: 1,248 lines, SHA256
  `00d37fbb8889ec65646211a4533a47907c50627c963b2af33bffcde1524e8231`.
- Frozen wave35 inputs: 372 lines/85 pins, SHA256
  `15e2a775179088173ed68fe2684b6a6298f1e2dbf0a09de95f1de66cda39d85f`.
- Frozen inventory: 1,676 lines, SHA256
  `8cc214bd11082c3438ed0fb6548778fe0e3d3c07bc5cfcd7c7e001fa1352d32c`.

The two frozen wave35 files and launch receipt reside under
target/r2-daily-beta/wp-4-integration. Their preparation states remain historical.
The dd1b92f failure, C05 bounded S2 acceptance and all older failed/partial outcomes
remain immutable; this success does not upgrade their original outcomes.

Next is a bounded I2 schema/transaction-authority source scope for migration0060,
with its actual-role denial, concurrency and guarded DOWN tests and sequential
official-harness wiring fixed before first execution. No0060 file or execution
was added here. Runtime UID/ACL access, provisioner filesystem rollback, provider/
copy reconstruction/admission, I6 diagrams before structural main integration,
deployment/Owner, three trading days and final C_R2 remain open. R2-04/09 stay DOING.
