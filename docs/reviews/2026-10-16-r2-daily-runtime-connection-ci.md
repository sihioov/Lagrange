# R2 I4g3 runtime connection — bounded hosted acceptance

R2-04/09, Wave62. Candidate `eed17d2bf79b287d65ca33e1ca0d1c3eee82995f`,
[CI38804040503 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38804040503),
workflow_dispatch on excited-dolphin. Full recovery at
`2026-10-15T21:02:04.817602+00:00`; all8 jobs SUCCESS.

Accept the [registered runtime connection](2026-10-16-r2-daily-runtime-connection-source.md)
and listed isolated regressions at this exact candidate. Direct coordinator source
review is not independent review. This is not installed/runtime/Owner or final R2 acceptance.

## Evidence

All8 terminal logs recovered; all7 actual checkout SHAs match the candidate.
All642 frozen pins and346 committed source/reference blobs match. Private sanitized
receipt `target/r2-daily-beta/ci/eed17d2/ci-evidence.json`:8212lines,
SHA256 `6691c9de618be8faf44f935acd0d5b2311d9205f9b6b6b07588629a9d7607d6f`.
The source-input and definition inventories remain frozen with the launch receipt.
Raw CI logs were held in memory only; evidence retains hashes, fixed metadata and named rows.

| Verification | Result |
| --- | --- |
| Workspace and strict lint | workspace format and all-target/all-feature Clippy PASS |
| New runtime connection |23/23 required named rows PASS:10Linux native,9other nonDB,4actualDB |
| Earlier readback/navigation |18+19 named rows PASS; prior failures preserved |
| Dedicated DB schedule/readback |32/32 named PASS, exit0/noSKIP |
| Dedicated daily runner |24/24 named PASS, including corrected complete migration-version assertion |
| Dedicated publication/recovery |8historical publication,6daily publication,6daily recovery PASS |
| Other Rust regressions |every frozen required name matched once within its execution phase; groups overlap and are not summed |
| Chart |6 separate named PASS |
| PostgreSQL harness |17 target exits0, noSKIP, APPROVED |
| SQL contracts |32daily cases,73subcases,83bootstrap cases PASS |
| Full release migrations |62UP SHA384 checksums match committed blobs on both installs; rerunNO_OP |
| PostgreSQL boundaries |7direct roles,0039guard,catalog probes,owned-container cleanup PASS |
| Web |34supervisor cases;121OFF and14market-stream cases, no skip/retry/flaky; cleanup PASS |
| Python |487pass,10existing skips,7warnings |

There are85successful steps and one conditional failure-log printing step skipped.
No required test step was skipped. Three non-Linux definitions remain excluded on
this Linux candidate; no cross-platform execution is inferred. Unprinted nested
helper rows and inner exits remain UNKNOWN despite successful recorded outer exits.

The previous Wave61 migration test failure is resolved by comparing the complete
ordered applied-version set against embedded UP migrations. Populated-row rollback
and privilege assertions still pass. Preserve [Wave61 failure](2026-10-16-r2-daily-admitter-readback-c01-ci.md)
and all older CI evidence. No old/new run was rerun, canceled or duplicated.

## Remaining work

Proceed to I5 API config/read contracts, then I7 Web and I6 controlled release work.
The real approved-v3/Raw first-candidate flow across three OS identities, complete
multi-day supply, crash/transport-loss behavior, installed UID/ACL boundaries and
installer rollback still require integration evidence. Diagrams must match before
main. Independent final review, concrete G4 operations, actual Owner use, three
consecutive trading days and final C_R2 remain open. Ralph continues.
