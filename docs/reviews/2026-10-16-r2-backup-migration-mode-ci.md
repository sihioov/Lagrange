# R2 I6B3-C03 exact-candidate hosted verification

Candidate `70401cf421c90105f7c5d54703a0ec68c3fa25aa`,
[run 38869717878 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38869717878),
created 2026-10-16T07:50:07Z, recovery bound 09:20:07Z.
Workflow `ci.yml`, `workflow_dispatch`, branch `excited-dolphin`.
**SUCCESS; coordinator accepts the hosted I6B3 package scope.** All eight jobs
completed by 08:06:44 UTC (17:06:44 KST). This is not operational or final R2
acceptance. Full recovery and disposition are recorded below.

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

Historical 07:56 UTC checkpoint: format, strict Clippy, Python and policy completed successfully.
All seven individually named `DailyBackupDockerTests` rows are PASS, including
actual full-schema encrypted capture and networkless restoration. Full policy
log SHA256: `fe63211a92d2f949ee3fc458b4eec86b797578290621d12c22faccaa66ce6bbf`.
Workspace Rust, PostgreSQL and Web were still in progress at that checkpoint;
required aggregation and candidate disposition awaited all eight terminal
jobs/logs and seven actual checkout SHAs. Final recovery below supersedes this
checkpoint. The initial native backup setup failure is resolved in this run.

The final recovery requirements were the complete 88-definition package, transport11,
backupDocker7, all carried Rust/Web/CI-contract cases, strict lint/format,
19 dedicated PG targets, 12 individual backup rows, both62-checksum lanes,
0060/0039 and cleanup. Overlapping groups are not summed. Missing rows remain
UNKNOWN unless stage evidence proves NOT_EXECUTED; unprinted nested helpers
remain UNKNOWN. Raw logs are not persisted.

Reviewed source and inputs remain unchanged. No rerun/cancel/duplicate, new worker,
local project execution or production action. Documentation HEAD may advance
without changing this candidate. Approved-base whole flow, PUML/PNG, independent
integrated review, concrete G3/G4, actual Owner use and three consecutive trading
days remain outstanding. Observation has not started.

## Final recovery and coordinator disposition — 2026-10-16

All eight terminal jobs and full-log digests, seven actual checkout SHAs,
1,053 frozen input pins and 485 candidate blobs match. The complete 88-definition
backup package passed, including all seven actual Docker backup/restore cases.
All required current and carried named groups passed; declared platform
exclusions remain exclusions. The previously failing Owner recovery case passed.

| Gate | Recovered result |
| --- | --- |
| Workspace format / strict Clippy | PASS; exact required commands observed |
| Workspace Rust / chart | Job PASS; 3,346 complete workspace named-result tokens and six separate chart tokens retained. These totals are not a unique-definition proof. |
| Policy fixtures | PASS; backup64, transport11, native backup7 and all carried installer/runtime/CI-contract groups |
| Dedicated PostgreSQL | 19 targets exit0/noSKIP; 12 distinct backup cases PASS; both62-checksum lanes, bootstrap83, 0060 32/73, 0039 guard and cleanup PASS |
| Web | OFF129/129 and market-stream14/14; zero retries/skips/flaky cases; both cleanup checks PASS |
| Python | 487 passed, 10 existing skips, seven warnings |

Cargo stdout/stderr interleaving caused one recovered `robustness_harness` result
to inherit the next binary's header. The exact named PASS at log line5842 is
bound to its unique definition in `crates/result-model/tests/robustness_harness.rs:524`.
A complete rate-limit result at line4128 was also omitted by the original
whole-line extractor. A separate REST result at line4145 has the noncanonical
suffix `okok`; its individual outcome remains UNKNOWN and is excluded from the
complete-token count. It is not a missing required package-inventory row.
Unprinted nested helper rows/exits also remain UNKNOWN. No aggregate result was
used to manufacture any of these individual outcomes.

Private records under `target/r2-daily-beta/ci/70401cf/`:

- Original `ci-evidence.json`: SHA256 `7d5d9918f027ca595c741502d5537a028cc6ba14c80e182b8f93f40aacb38560`; preserved unchanged.
- `named-row-supplement.json`: SHA256 `f55cb5a652880f59bbb1441eb4fd10ded19588f7bc6f57459d139ad70e4280c6`.
- `coordinator-disposition.json`: SHA256 `84344ba07fab48f3c4c550fd525e53a45708917088fe9eab658e16dc36115059`.

This closes I6B3/C01/C02/C03 hosted verification, with coordinator source review
explicitly distinguished from independent integrated review. Actual pre-upgrade
recovery, the installed backup transition and approved-base whole flow remain
open as specified in [first-rollout readiness](2026-10-16-r2-first-rollout-readiness.md).
No production action or new CI was started during recovery.

## Elapsed-time follow-up requested by the Owner

At 17:18 KST, Ralph had been active for about22 hours since 2026-10-15 19:16 KST.
The [earlier audit](2026-10-16-r2-ralph-progress-audit.md) covered36 terminal runs.
Adding only the four subsequent recorded candidates gives40 runs:16 success and
24 failure, with48,259 seconds (13h24m19s) of sequential CI run intervals. This is
not a claim that the coordinator was idle throughout those intervals. Excessive
package fragmentation and repeated late compile/lint/fixture corrections caused
avoidable delay. The current candidate CI is terminal; the coordinator is
reconciling evidence and preparing the remaining integration/operational work.
No three-trading-day observation has begun, and no R3 readiness is claimed.
