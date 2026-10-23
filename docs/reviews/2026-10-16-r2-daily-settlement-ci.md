# R2 I4b single-candidate CI recovery

Date: 2026-10-16 KST. Roadmap: R2-04 / R2-09. Status: FAILURE recovered.
The Rust and dedicated DB evidence passed; the overall candidate remains failed
because the web E2E step failed. No full I4 or R2 acceptance is claimed.

Candidate `a72aaa4fef618a7bdb7a292b171fd70d6245d52b`,
[run 38768192668 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38768192668),
CI/ci.yml, workflow_dispatch, excited-dolphin. One dispatch followed exact-head
all-event query zero and remote equality. No rerun, cancellation or duplicate.

All eight terminal jobs and complete log digests were recovered. Seven checkout
logs contain the exact candidate; all 467 frozen pins and 314 committed blobs
match. Sanitized receipt: `target/r2-daily-beta/ci/a72aaa4/ci-evidence.json`,
4,857 lines, SHA256
`589b80b9bafa70fce788f71bd45a092ea94a72d6567492e04a3dcfc4a42528ea`.
Raw logs were not persisted. Later documentation commits do not change this candidate.

| Evidence | Result |
| --- | --- |
| Workspace format, strict all-target/all-feature Clippy, policy, Python | SUCCESS |
| Workspace tests | SUCCESS; 12 new I4b, 14 I4a and eight affected settlement unit names PASS |
| Preserved regressions | All listed I3, S5, S4, S3, SQL, historical, service and transport names PASS |
| Separate chart phase | Six exact named PASS rows |
| Protected artifact fixture | Required modes, setup and nonrecursive cleanup PASS |
| Dedicated PostgreSQL | SUCCESS; all 16 outer targets exit zero, no SKIP |
| Web | OpenAPI, lint, typecheck, unit tests and build PASS; E2E FAILURE |
| Required aggregate | FAILURE |

Dedicated PG independently proves 14 daily schedule, six daily failure settlement,
six daily recovery and eight historical publication named rows. It also retains
32 daily migration cases, 73 fixed subcases, 83 bootstrap cases, 60 exact committed
UP SHA384 checksums in both lanes, NO_OP rerun, direct roles, 0039/catalog and cleanup.
Unprinted nested helper names and inner exits remain UNKNOWN. Workspace DB test
rows alone are not substituted for these dedicated PG results.

The web log contains only `state=failed exit=1 coverage=complete` at line 1034;
the detailed lane/case receipts were private files on the hosted runner. The run
artifact query returned no artifacts. Its complete log SHA256 is
`d3aa3c3d4479af0e680340c4954a663e3fac357fe38a3a2f83216d54d99cb221`.
The failing case and root cause are UNKNOWN. A sanitized supplemental receipt
preserves that limitation. A later green run cannot establish this failure's cause.

After full recovery, the next bounded package adds daily success publication and
safe web lane/case diagnostics. It does not weaken tests, retries, timeouts or
cleanup. Actual approved-base success integration, full runtime orchestration,
superseded cancellation, integrated independent review, I5-I7, deployment, Owner
use, three trading days and final C_R2 remain open.
