# R2 I6B2c controlled activation CI failure

Roadmap R2-04/R2-09. Candidate `6614dda99cd0613c5199c617ef253b3bb7a48b3c`.
[CI38847051561 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38847051561)
completed with failure. All eight terminal jobs/full log digests, seven actual
checkouts, 996 pins and 460 candidate blobs were recovered.

Policy failed inside `scripts/ops/static-check.sh`, at the delegated
`production-ops-static-check.sh:109` assertion requiring exactly four
`OWNER_BETA_*` variables. The reviewed base Compose now includes the fifth
literal-disabled daily mode. The fixed diagnostic and source condition agree.
The original full policy log digest and bounded diagnostic supplement remain
preserved; the failure is not a failure of a new runtime test.

All29 new operation/root and8 actual PostgreSQL transport definitions are
NOT_EXECUTED because their steps were skipped. Previous58 root/runtime/Docker
definitions and the legacy operations fixture were also skipped in this run.
Earlier Wave75 bounded successes remain historical evidence, not this candidate's
runtime acceptance.

Workspace32 required Rust groups and separate chart6 passed by individual name.
Strict Clippy and workspace formatting passed. Dedicated PostgreSQL18 targets/
134 named rows, schedule43,62 migration checksums in both installations,0039,
bootstrap83,schema daily32/subcase73,catalog,rerun and cleanup passed.
Web UI18,contracts6,supervisor34,OFF129/stream14 and exact Owner11/daily8 browser
rows passed. Python job passed. Required aggregate failed due to policy.
Unprinted nested helper rows and inner exits remain UNKNOWN.

Private sanitized receipt: `target/r2-daily-beta/ci/6614dda/ci-evidence.json`, 9945 lines,
SHA256 `65e40e5dea04dfa123107252f6ec7841c5a03cda2800b3fe8c47ba909cbbf0c9`.
Policy diagnostic supplement:
`target/r2-daily-beta/ci/6614dda/policy-static-diagnostics.json`.
Raw logs stayed in memory. No rerun, cancellation or duplicate dispatch occurred.

C01 is limited to the static contract: preserve the existing four mode checks,
require the new API and worker modes to be literal disabled in base Compose,
and retain the sole controlled wrapper and release quiescence guards.
Runtime, SQL, Compose, test and workflow bytes remain unchanged. A fresh candidate
needs one exact-head CI after source review and all-event existing-run query.

No I6B2c source/runtime acceptance. Consistent backup/restore reconciliation,
evidence-bound diagrams, approved-base first/multiday flow, independent
integrated review,G4,Owner,three actual trading days/final C_R2 remain open.
