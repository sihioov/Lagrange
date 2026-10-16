# R2 integrated candidate hosted verification

Status: **both exact-candidate runs are in progress; no executable or release
acceptance yet.** Required source review is closed as described in the
[coordinator disposition](2026-10-17-r2-integrated-review-closure.md).

Candidate: `492cbae5618f695c3e27126e4ccf490a1ffda981`.
Reviewed executable candidate: `692f57f792e5090e267a8dd533bea4c3328c985c`.
The intervening changes are documentation, including the D02 evidence template.

| Required run | Exact run / attempt | Created UTC | Initial observed state |
| --- | --- | --- | --- |
| Required CI | [38955175446 / 1](https://github.com/sihioov/Lagrange/actions/runs/38955175446) | 2026-10-16 19:28:29 | In progress; format job metadata success, six execution jobs running; aggregate not yet created |
| Functional research-worker smoke | [38955182520 / 1](https://github.com/sihioov/Lagrange/actions/runs/38955182520) | 2026-10-16 19:28:33 | One execution job running |

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
`target/r2-daily-beta/ci/492cbae/`. Raw logs are not persisted. Full job logs,
hashes, actual checkout SHAs, qualified test rows and cleanup are not yet
recovered; aggregate counts alone cannot establish named-test acceptance.
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
