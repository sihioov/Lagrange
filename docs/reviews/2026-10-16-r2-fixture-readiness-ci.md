# R2 I6B3-C02 exact-candidate hosted verification

Candidate `4336fbe0e73b53c9dff5e66a477b3ef28f04cb4d`,
[run 38866283743 attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38866283743),
created 2026-10-16T07:17:33Z, recovery bound 08:47:33Z.
Workflow `ci.yml`, `workflow_dispatch`, branch `excited-dolphin`.
**FAILURE; all eight jobs and complete log hashes recovered. No candidate or
runtime acceptance.** The run completed at 2026-10-16T07:41:51Z.

[Five-file correction](2026-10-16-r2-fixture-readiness-correction.md) follows
full recovery of [failed Wave79](2026-10-16-r2-daily-backup-correction-ci.md).
Both pre/post-push exact-head all-event queries returned zero, the remote SHA
matched, and exactly one dispatch was issued. The recovered run identity matches.
All 1,046 input pins and 483 committed blobs matched before launch.

Private preparation under `target/r2-daily-beta/wp-4-integration/`:

- `wave80-I6B3-C02-source-inputs.json`, 4,268 lines,
  SHA256 `4ca1712c883a1b7eb35feb03ce12dd80f81504594ac7a93079f877ea5e4aa244`.
- `wave80-I6B3-C02-definition-inventory.json`, 7,810 lines,
  SHA256 `9f4f49b21a17204afdfe59566c51cfed1af464a791d98a2333d3e46c3c59ba84`.
- Separate mutable `wave80-I6B3-C02-ci-launch.json` records this one dispatch.

Recover eight terminal jobs/full log hashes and seven actual checkout SHAs;
88 backup-package definitions, full transport11/backupDocker7, all carried
Rust/CI-contract/Web-supervisor34 and browser cases, strict lint/format, all19
dedicated PG target exits/no-SKIP, exact backup12 rows, both62-checksum lanes,
0060/0039 and cleanup. Overlapping groups are not summed as unique tests.
Missing individual rows remain UNKNOWN unless failed-stage evidence proves
NOT_EXECUTED. Unprinted nested helpers remain UNKNOWN. Raw logs stay in memory.

No rerun/cancel/duplicate, local project execution, source change while frozen,
new worker, provider or production action. Documentation HEAD may advance;
the reviewed source candidate remains the SHA above. Independent integrated
review, approved-base whole flow, PUML/PNG, concrete G3/G4, real Owner use and
three actual consecutive trading days remain outstanding. Observation has not
started. The elapsed-time problem is recorded in the
[audit](2026-10-16-r2-ralph-progress-audit.md); passing this candidate is not R2 completion.

## Preserved partial recovery

Format, strict Clippy, Python, Web and dedicated PostgreSQL passed. At
07:33:34 UTC workspace tests were running; required aggregation was pending.
The PG log proves 19 target exits0/noSKIP, 12 unique backup rows PASS, both
62-migration checksum sets, 0060/0039 and cleanup. Policy failed after the corrected
database readiness path: all 11 actual transport definitions passed, while
backup setup failed at migration application with the closed diagnostic
`DAILY_BACKUP_FIXTURE_SQL:migration:3:25001`. Its seven test bodies are
NOT_EXECUTED. Full policy log SHA256:
`83e5bcdb14adb91b8451d8061c4894e14e84ddc6eeaaffc071eef3025785cbb2`.

Concrete source inspection found that the backup fixture wraps every migration
in BEGIN/COMMIT. Seven migrations explicitly declare `-- no-transaction` and
create indexes CONCURRENTLY: 0024,0025,0027,0028,0029,0031,0032. This is a
fixture defect consistent with the observed transaction-state error; the
diagnostic does not directly identify the failed migration number. Production
migration bytes are unchanged. Correct the fixture's transaction policy only
after the remaining jobs and full result are recovered. Frozen inputs remain.

Inventory bookkeeping supplement: both new readiness definitions already exist
in the frozen class groups, but the aggregate `definitions` list retained 86.
The immutable 737-line successor
`wave80-I6B3-C02-definition-inventory-supplement.json` appends those same two
class-qualified rows, for the intended 88. SHA256
`cd5418475afbe6f00f1add2b244b0ce08399e8252be896464511687e78b2ef79`.
No source, test selection, original inventory or CI dispatch changed.

## Full recovery and disposition

All 1,046 frozen input pins, 483 committed source blobs and seven actual job
checkout SHAs matched. Eight terminal jobs and full log hashes were recovered.
Format, strict Clippy, workspace, PostgreSQL, Python and Web passed; policy and
the required aggregate failed. No job was rerun or canceled.

- Workspace: 3,346 individually qualified Rust rows and six separate chart rows
  passed. All 36 current Rust groups and 33 carried groups were reconciled;
  overlapping groups are not added as unique cases.
- The 88 backup-package definitions resolve to 81 PASS and seven NOT_EXECUTED.
  All 11 transport, 64 file/controller and ten CI-contract definitions passed.
  `DailyBackupDockerTests.setUpClass` failed before its seven test bodies with
  the migration-stage SQLSTATE25001 recorded above.
- Dedicated PG: 19 targets exited0 with no SKIP; all 12 backup names appeared
  once and passed. Both full-release lanes matched all62 source SHA384 checksums;
  83 bootstrap negatives, 32 named0060 rows/73 subcases, 0039 and cleanup passed.
- Web: 18 daily UI and six contract markers, 34 supervisor definitions and
  11 named Owner browser cases passed. OFF129/129 and stream14/14 passed with
  zero retries, skips or flaky cases; both cleanup receipts passed.
- Python: 487 passed, ten existing skipped, seven warnings. Legacy production
  operations fixture passed. Unprinted nested helper rows/exits remain UNKNOWN.

Private full receipt: `target/r2-daily-beta/ci/4336fbe/ci-evidence.json`, 26,879
lines, SHA256 `94c3aacdeca1f678e130a35fdd875e80e5a486b337d7fb811f229504be49429e`.
Original log projections, inventories and the aggregate supplement are preserved.
Only sanitized metadata/named results were persisted; raw logs stayed in memory.

After full recovery, the [C03 fixture correction](2026-10-16-r2-backup-migration-mode-correction.md)
honors the seven existing nontransactional declarations and strengthens the
actual schema/restore control. Its execution is still unverified. This failure
remains a failure; prior source and runtime limitations remain open.
