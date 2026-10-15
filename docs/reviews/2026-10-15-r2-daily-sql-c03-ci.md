# R2 I2-C03 — complete harness candidate verification

2026-10-15; candidate dee78c4531e1cb14eefd8abb3bf90189d382ca6e.
[CI38696988981](https://github.com/sihioov/Lagrange/actions/runs/38696988981),
attempt1, CI/ci.yml, workflow_dispatch, excited-dolphin, sihioov/Lagrange.

**IN_PROGRESS; no acceptance.** [C03 source disposition](2026-10-15-r2-daily-sql-c03.md)
changes one evidence-capture pattern after full C02 recovery.249 frozen pins and
204 tracked candidate blobs match. All-event exact-head query0 and remote SHA
match at07:10:28 UTC preceded one dispatch; created07:10:31 UTC. No rerun/cancel.

Recover all8 jobs/steps/log hashes and7 actual checkout SHAs. Required coverage:
32 daily DB definitions,73 canonical unique subcase rows,28 affected legacy names,
99 prior R2 Rust names,6 separate chart,3 transport and83 bootstrap;12 actual DB
label exits0/no-SKIP,60 checksums twice/no-op, direct roles, rollback and cleanup.
Workspace and dedicated PG evidence remain separate. Missing rows stay UNKNOWN
unless failure-stage evidence proves NOT_EXECUTED. No aggregate count is a
substitute for a required named row.

Frozen source-input receipt1055lines/249pins, SHA256
822ae62ff66a4b249d4bca9ac430b8e2f4e9976e9896f51e1932417be2228329;
definition inventory736lines, SHA256
26168085e1b55fe61c1c5459ee448752907ef627bab149f4aa618f31064211f7.
Both Wave40 files are under target/r2-daily-beta/wp-4-integration. Sanitized CI
evidence goes under target/r2-daily-beta/ci/dee78c4; raw logs remain memory-only.
Direct coordinator recovery has a75-minute reporting bound; no delegation or
same-daemon heartbeat claim. Later documentation HEAD does not change this source.

All previous failures/partial successes remain preserved. I2 final acceptance,
I3–I7, provider/copy reconstruction, runtime/Owner, three trading days and final
C_R2 remain open.

## Complete recovery — 2026-10-15

The launch status above is historical. The run completed **SUCCESS** at
07:33:13 UTC. All eight jobs, 84 steps, eight log hashes and seven actual
checkout SHAs were recovered. All 249 frozen pins and 204 committed candidate
blobs match `dee78c4531e1cb14eefd8abb3bf90189d382ca6e`.

| Gate | Recovered result |
| --- | --- |
| Jobs | PostgreSQL, format, policy, Web including E2E, Python, strict Clippy, workspace and required all SUCCESS |
| Steps | 83 SUCCESS; only the conditional PostgreSQL failure-log step was SKIPPED |
| Workspace named rows | 32 daily, 28 affected legacy, 99 prior R2 Rust and 3 transport cases each match exactly one `ok` row |
| Separate chart step | All 6 required names match exactly one `ok` row |
| Dedicated PostgreSQL lane | All 32 daily names and 73 fixed subcase names occur exactly once with passing outcomes in canonical harness evidence |
| Actual DB labels | All 12 recorded numeric exits are 0, with no SKIP markers |
| Migration ledger | Both complete passes match all 60 SHA384 hashes from candidate UP blobs; rerun is NO_OP |
| Authority and rollback | Seven direct principals in both lanes, 0039 guard, four catalog negative probes and the 0060 role/rollback harness pass |
| Bootstrap | All 83 expected names pass in canonical harness JSON |
| Cleanup | PASS_NO_OWNED_CONTAINERS |

The successful PostgreSQL job does not run its failure-log dump. Therefore its
83 bootstrap results are canonical JSON entries, not standalone log lines. The
32/73 DB outcomes likewise retain their canonical evidence source. Workspace
has 2,974 printed rows; that aggregate is not a unique-test acceptance count.
The two Linux platform exclusions remain excluded. Python reports 485 passed,
10 skipped and 7 warnings; those skips are distinct from the enforced DB no-SKIP
gate. Unprinted nested helper names and inner exit codes remain UNKNOWN even
though all 12 outer DB label exits are now known.

Final sanitized receipt: `target/r2-daily-beta/ci/dee78c4/ci-evidence.json`,
2,265 lines, SHA256
`241dcf762329aa3eb3aba04f6df124ad07e2b5cef6b888577d0929d3abb8712d`.
The receipt includes all jobs/steps, log hashes, checkout SHAs, individual required
workspace rows, canonical DB rows, exclusions and acceptance limits. Raw logs
were kept in memory only. Earlier partial receipts, all three failed I2 candidates
and all prior R2 successes/failures are preserved.

The coordinator accepts the bounded I2 implementation and hosted evidence in
[the I2 acceptance record](2026-10-15-r2-daily-sql-acceptance.md). No new independent
source review is claimed. No local project execution, new worker, rerun, cancel,
production operation or runtime activation occurred during recovery. Later
documentation commits do not change the tested source revision.
