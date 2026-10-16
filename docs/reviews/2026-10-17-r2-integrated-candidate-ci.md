# R2 integrated candidate hosted verification

Status: **required CI is in progress; separate functional smoke failed at
migration0060. No complete executable or release acceptance.** Required source review is closed as described in the
[coordinator disposition](2026-10-17-r2-integrated-review-closure.md).

Candidate: `492cbae5618f695c3e27126e4ccf490a1ffda981`.
Reviewed executable candidate: `692f57f792e5090e267a8dd533bea4c3328c985c`.
The intervening changes are documentation, including the D02 evidence template.

| Required run | Exact run / attempt | Created UTC | Latest recovered state |
| --- | --- | --- | --- |
| Required CI | [38955175446 / 1](https://github.com/sihioov/Lagrange/actions/runs/38955175446) | 2026-10-16 19:28:29 | 19:44UTC: format/policy/Clippy/Python/Web/PG success; workspace running; aggregate not yet created |
| Functional research-worker smoke | [38955182520 / 1](https://github.com/sihioov/Lagrange/actions/runs/38955182520) | 2026-10-16 19:28:33 | FAILED before functional image/publication stages; details below |

Both are `workflow_dispatch` on `excited-dolphin` at the exact candidate SHA.
The all-event/all-workflow exact-head query returned zero before push. Remote
SHA and matching-workflow absence were checked again before each dispatch.
Each dispatch was issued once and returned exit0; run IDs/head/attempt were
then recovered. No previous run was rerun, cancelled or duplicated.

Frozen inputs:

- `target/r2-daily-beta/wp-7/wave88/ci-inputs.json`: SHA256
  `3bb728151a380256117e64c19074a86c6c7e2697b8f928a22cb28ff5306d2d7a`,
  1,149 file pins and511 committed blob comparisons.
- `target/r2-daily-beta/wp-7/wave88/test-inventory.json`: SHA256
  `4a8d0bb44645a6f119e3d3d82c64d4de0c9d974710adf5048151b342a6d182d0`,
  110 required new definitions, including the ten review corrections. Four
  changed modules contain104 definitions with all94 earlier names retained.
- The prior f215665/CI38901117928 success and original failure history remain
  immutable historical evidence. Each carried group requires fresh outcomes;
  overlapping groups are not added as unique tests.

Launch receipts are `ci-launch.json` and `research-smoke-launch.json` beside
the inputs. Sanitized current metadata is in
`target/r2-daily-beta/ci/492cbae/`. Raw logs are not persisted. Seven terminal job logs and actual checkout SHAs are recovered below; remaining
job logs, qualified outcomes and cleanup are not yet fully recovered; aggregate counts alone cannot establish named-test acceptance.
Unprinted nested independent rows and exits remain UNKNOWN.

Monitoring: heartbeat `f50af5d3`, every five minutes, expires at
`2026-10-16T21:29:11.870Z`. Reporting bounds are21:13:29/33UTC for CI/smoke.
Before that bound recover complete and missing evidence and renew or explicitly
hand off the remaining scope. Same-daemon monitoring cannot protect an outage.
No source/frozen-input mutation while the runs remain active; delete monitoring
only after terminal full recovery or an explicit bounded handoff.

No local compiler/test/build/project-helper, SQL/DB, provider, root/Docker or
production action occurred. No worker was added. Actual images, approved-base
host flow, root access, G3/G4, Owner use, three consecutive trading days and
final C_R2 remain open. Later documentation HEAD does not alter this candidate.


## 2026-10-16 19:35–19:39 UTC partial recovery

All five completed job logs were fetched in memory, hashed in full and reduced
to sanitized evidence under `target/r2-daily-beta/ci/492cbae/`. Each actual
`git log -1 --format=%H` returned the exact candidate. Format and strict Clippy
command flags were observed. Policy emitted287 qualified Python PASS rows,
including all ten correction cases and all104 definitions in the four changed
modules. The required110-case new-definition inventory has106 named PASS from
policy; four Rust daily-ingest cases await the workspace log. Python suite
summary is487 passed/10 skipped/7 warnings, not a per-case acceptance list.

The functional smoke's complete449-line log has SHA256
`9737c7e9eafd84b09a5ce8d4965a46af131a21be2aebbcbbd5056447cf48d206`.
Its actual checkout matches492cbae. Static phase passed at log151; log430 reports
`daily role preflight denied`, log432 identifies
`0060_owner_beta_daily_inputs.up.sql`, and log433 records exit1. Source comparison
finds the disposable fixture at `scripts/qa/research-worker-smoke.sh:665-679`
creates six roles and omits `daily_input_admitter`, which migration0060 requires
at lines8–25. This explains the fixture prerequisite gap; the actual pg_roles
catalog was not printed and no production-role deficiency is inferred.

Full migration endpoint, the two functional Rust image builds and publication
checks were not reached. The EXIT trap suppresses cleanup command output and
exit statuses, so actual cleanup remains UNKNOWN. The failed workflow is
preserved and will not be rerun or cancelled. The required CI is still running;
source/frozen inputs remain untouched. Only after terminal complete recovery
will the coordinator make the bounded fixture correction and bind successor
verification. No new worker, local execution or production operation occurred.

## 2026-10-16 19:40–19:45 UTC Web and PostgreSQL recovery

Six of seven execution jobs are now successful. The workspace Cargo test step
is still running (started19:32:37UTC); the required aggregate is not yet created.
No new source changes or workflow dispatch occurred. Current receipts include
seven complete log hashes and seven actual checkout SHAs, counting the failed
separate smoke. Each checkout matches492cbae. All1,149 frozen input pins still match.

Web log SHA256 is
`463e007bc40b75b37d51520e9ee6c6c319ad6f38f12505832345be5963e69e86`.
All34 supervisor names,6 daily-contract names and18 UI names match their
expected sets and PASS. The two hosted browser lanes contain129 and14 unique
case identities, all passed once with no skips/retries/flaky cases. Both record
cleanup and reservation success. These overlapping scopes are not summed as
unique tests.

PostgreSQL log SHA256 is
`31bac55c584a1ae6ab56c0e6224f3eaf460fd09edf387ad38ac3dc0246642467`.
All19 explicit process exits are0 with no SKIP;146 qualified named rows match
the required sets and PASS. Migration0060 has32 named results and73 subcases;
all83 bootstrap cases match. First install and no-op rerun each report62 current
migration checksums, independently matched to local source SHA384 values.
The0039 guard, role-login checks and cleanup PASS_NO_OWNED_CONTAINERS are
recorded in the current run. Unprinted nested helper independent outcomes
remain UNKNOWN.

`target/r2-daily-beta/ci/492cbae/web-pg-reconciliation.json` binds this partial
disposition to both complete log hashes and named results. The failed functional
smoke and its unknown cleanup are separate and preserved. Required CI completion,
full candidate disposition and all host/final-release gates remain open.
