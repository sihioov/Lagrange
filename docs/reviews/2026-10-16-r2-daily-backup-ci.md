# R2 I6B3 coherent daily backup hosted verification

Candidate `977e356792881fa50cad2ff95ababc04434e4c91`, [CI run 38858383219 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38858383219),
`workflow_dispatch`, branch `excited-dolphin`, `.github/workflows/ci.yml`.
Created 2026-10-16T05:56:01Z; completed with **failure** at 06:16:09Z.
Full coordinator recovery completed at 06:20:22.899326Z, within the 07:26:01Z bound.

Exact-head all-event queries before and after push returned zero. Remote SHA
matched before the single dispatch. No previous or current run was rerun,
cancelled or duplicated. All eight terminal jobs and full log digests, seven
actual execution checkout SHAs, 1,036 frozen pins and 478 committed blobs match.

| Evidence | Recovered result |
| --- | --- |
| Format / strict Clippy | PASS; workspace format and all-targets/all-features `-D warnings` observed |
| Policy | FAIL; 64 new file/controller definitions: 61 PASS, three failed definitions (five exception events) |
| Actual isolated Docker backup | Six definitions NOT_EXECUTED because the preceding policy step failed |
| Actual-role backup SQL | Eleven definitions: five PASS, six FAIL in both workspace and dedicated PG lanes |
| New transport / enable | Two plus two PASS; containing transport 10 and operations 31 named rows PASS |
| All new definitions | 85 total: 70 PASS, nine FAIL, six NOT_EXECUTED; not an acceptance result |
| Workspace | FAIL; 3,351 named rows comprise six separate chart rows and 3,345 workspace rows; all six failures are the new backup SQL cases |
| Prior Rust groups | All 33 required groups PASS by qualified names; corrected Owner-equity recovery case PASS |
| Dedicated PG | DENIED; first 15 targets exit 0/no-SKIP, backup target exit 101/no-SKIP, final three targets NOT_EXECUTED in this lane |
| PG migration evidence | 62 full-release UP checksums match on both runs; staged 0060 32 named/73 subcases, bootstrap 83, catalog/0039 guard and cleanup PASS |
| Web | PASS; 18 UI/six contract markers, 34 supervisor, 129 OFF and 14 stream cases; all eleven Owner browser definitions matched, no retry/flaky/skip, cleanup verified |
| Python | PASS; 487 passed, ten skipped, seven warnings retained |
| Legacy operations fixtures | NOT_EXECUTED because of the preceding policy failure |
| Required aggregate | FAIL |

The three final PG targets are daily-runner, daily-api and collector-status.
They passed in the workspace lane; that does not replace missing dedicated-lane
execution. Unprinted nested helper rows/exits remain UNKNOWN.

The Python failures are concrete fixture defects: a missing `target.images`
mapping and three manifest assertions displaced into another method. The six
SQL failures share fixed SQLSTATE `23514`: both maintenance transactions rejected
`users.relrowsecurity=true`, although migration 0010 intentionally enables but
does not force RLS on shared users and its actual table owner sees every identity.
The [bounded correction](2026-10-16-r2-daily-backup-correction.md) preserves role,
policy, history, reservation and failure gates. Its new bytes are unexecuted.

Private receipt `target/r2-daily-beta/ci/977e356/ci-evidence.json`: 29,053 lines,
SHA256 `eb36cc66c09fb51cb5b5ed04a97ccab61df4e81b09af6800d8c8050ce50b8cf7`.
Only sanitized metadata, exact named rows, fixed diagnostics and hashes persist;
raw logs remained in memory. Immutable inputs remain unchanged:
`wave78-I6B3-source-inputs.json` (4,287 lines,
`7e8306917ce5dd84b4204ac2cb13b5e7609b7310f309f24a29598c355e77227e`) and
`wave78-I6B3-definition-inventory.json` (7,684 lines,
`1c91c7740b3b87ae6bdf30e09cc85ebf314a8eb52a35d13d04b26155bc32db87`).

The [source scope](2026-10-16-r2-daily-backup-source.md) and previous failures
remain preserved. Direct review is not independent review. No whole-candidate,
production or final C_R2 acceptance. Approved-base whole flow, PUML/PNG updates,
independent integrated review, concrete G3/G4, Owner use and three consecutive
trading days remain open. Actual observation has not started.
