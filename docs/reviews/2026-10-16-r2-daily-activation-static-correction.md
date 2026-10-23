# R2 I6B2c-C01 daily-mode static contract correction

Roadmap R2-04/R2-09. Baseline6614dda after full CI38847051561 failure recovery.
Direct coordinator source review; independent integrated review remains open.

The old checker assumed exactly four OWNER_BETA policy keys, so it rejected
the newly reviewed literal-disabled API daily mode before all95 root/runtime/
transport definitions and the legacy operations fixture could run.
The correction changes only scripts/ops/production-ops-static-check.sh.

The four existing access, price-input, Paper and equity-signal assertions remain
byte-identical. The total policy-key bound is now exactly five. API daily mode
must appear once with literal disabled. Worker daily mode must appear once in
owner-beta-runner with literal disabled and nowhere else in base Compose.
The controlled prepare/enable/disable dispatch, internal operation module,
shared release lock and both process/DB quiescence calls remain required.
No wildcard mode allowance, default enable, alternate entrypoint or skipped
check is introduced.

Runtime/controller/SQL/Compose/workflow and all test bytes match the failed
candidate. All95 root/runtime/transport definition names and line locations
match the preserved inventory. No new test is required for this one-file
static-contract correction; the existing hosted policy gate and all95 actual
definitions remain mandatory. New operation29/transport8 remain unexecuted.

Static source/diff/hash/name/whitespace checks only locally. No shell checker,
Python project import, compiler, test, Docker, SQL, provider or runtime action
was executed. One new exact-candidate hosted CI follows all-event existing-run
query and remote SHA validation. Old run is never rerun/cancelled/duplicated.

Changed file: 415 lines, SHA256 e0de41a724b2be7c7cfdd38640ca96c6fa19994c6fdc41725c663fb6924599b8.
Prior source-review conclusions and limits carry forward. No source/runtime
acceptance is claimed before hosted results. Consistent backup/reconciliation,
diagrams,approved-base first/multiday flow,independent review,G4,Owner,
three actual trading days/final C_R2 remain open.
