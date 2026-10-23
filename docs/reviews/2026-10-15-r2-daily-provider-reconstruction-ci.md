# R2 I1b-S4 first candidate CI and C01 disposition

Recovered 2026-10-15; R2-04/09. Candidate
`48beb256b750c5c561263045cfa043d3b3eef71b`,
[run38722595047 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38722595047)
finished FAILURE. Repository sihioov/Lagrange, branch excited-dolphin,
CI/ci.yml, workflow_dispatch. Exactly one dispatch followed an all-event
exact-head query0 and remote SHA match; no rerun/cancel/duplicate occurred.

All eight jobs and complete log hashes, seven actual checkout SHAs,271 pins
and221 candidate blobs were recovered and matched before any correction.
Format, policy, Python, web including E2E, workspace tests and dedicated PG
passed. Strict Clippy failed; required reflected that failure. There were81
successful steps,2 failed steps and the one conditional PG failure-log skip.

The sole compiler/lint diagnostic is clippy::type_complexity on the seven-entry
fixture tuple at daily_price_only_reconstruction.rs:292:20. Workspace compilation
and tests passed. This is a real strict-lint failure, so S4 is not accepted.

| Named inventory group | Exact phase-matched outcomes |
| --- | --- |
| New S4 Linux definitions | 32 ok |
| Carried S3 definitions | 22 ok |
| Affected normalizer/calendar | 37 ok |
| Prior R2 Rust inventory | 99 ok |
| Daily SQL in workspace | 32 ok |
| Affected legacy DB | 28 ok |
| Transport | 3 ok |
| Separate chart step | 6 ok |

Groups may overlap and are not summed as unique cases. Dedicated PG separately
matched32 daily outcomes,73 unique subcases,83 bootstrap names and12 outer
DB exits0/no-SKIP. Both full-release lanes matched all60 candidate UP SHA384
checksums; NO_OP rerun, direct principals,0039/catalog/0060 rollback and cleanup
passed. Unprinted nested helper rows/inner exits remain UNKNOWN. Two carried
non-Linux exclusions and Python485 pass/10 skip/7 warnings remain explicit.

Sanitized receipt: `target/r2-daily-beta/ci/48beb25/ci-evidence.json`,2864 lines,
SHA256 `3bae05a1a4d22a77330a67a182bd776cfabe0cda0a3f4cbbd9366d0d13819358`.
Raw log bodies stayed in memory; full hashes and fixed diagnostics/names were
retained. Earlier S3/I2 successes and all failures remain immutable.

C01 may now replace only the fixture tuple with named fields and its two
consumers. A new exact-candidate run is required. No local project execution,
delegation, provider/production change or source acceptance occurred. Whole
frontier/artifact/admission/supply, I3–I7, final independent integrated review,
runtime/Owner/three trading days and final C_R2 remain open.
