# R2 I6B3-C03 exact-candidate hosted verification

Candidate `70401cf421c90105f7c5d54703a0ec68c3fa25aa`,
[run 38869717878 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38869717878),
created 2026-10-16T07:50:07Z, recovery bound 09:20:07Z.
Workflow `ci.yml`, `workflow_dispatch`, branch `excited-dolphin`.
**IN PROGRESS. No candidate or runtime acceptance.**

[One-file correction](2026-10-16-r2-backup-migration-mode-correction.md) follows
full recovery of [failed Wave80](2026-10-16-r2-fixture-readiness-ci.md).
Pre-push and post-push all-event/all-workflow exact-head queries returned zero;
the remote SHA matched and exactly one dispatch was issued. All 1,053 input
pins and 485 committed blobs matched before launch. No old run was retried.

Private preparation under `target/r2-daily-beta/wp-4-integration/`:

- `wave81-I6B3-C03-source-inputs.json`, 4,252 lines,
  SHA256 `618e03b8af6007dcc7bc0dd3fade7404090a8209a15542a60546bc2f5551611c`.
- `wave81-I6B3-C03-definition-inventory.json`, 7,825 lines,
  SHA256 `2b44e5d592964334df222be6d8b147a42ef11e656046ed5cceb999c84bb02baf`.
- Mutable `wave81-I6B3-C03-ci-launch.json` records the actual dispatch and run.

At 07:56 UTC, format, strict Clippy, Python and policy completed successfully.
All seven individually named `DailyBackupDockerTests` rows are PASS, including
actual full-schema encrypted capture and networkless restoration. Full policy
log SHA256: `fe63211a92d2f949ee3fc458b4eec86b797578290621d12c22faccaa66ce6bbf`.
Workspace Rust, PostgreSQL and Web remain in progress; required aggregation and
full candidate disposition await all eight terminal jobs/logs and seven actual
checkout SHAs. The initial native backup setup failure is resolved in this run.

Final recovery must reconcile the complete 88-definition package, transport11,
backupDocker7, all carried Rust/Web/CI-contract cases, strict lint/format,
19 dedicated PG targets, 12 individual backup rows, both62-checksum lanes,
0060/0039 and cleanup. Overlapping groups are not summed. Missing rows remain
UNKNOWN unless stage evidence proves NOT_EXECUTED; unprinted nested helpers
remain UNKNOWN. Raw logs are not persisted.

Frozen source and inputs remain unchanged. No rerun/cancel/duplicate, new worker,
local project execution or production action. Documentation HEAD may advance
without changing this candidate. Approved-base whole flow, PUML/PNG, independent
integrated review, concrete G3/G4, actual Owner use and three consecutive trading
days remain outstanding. Observation has not started.
