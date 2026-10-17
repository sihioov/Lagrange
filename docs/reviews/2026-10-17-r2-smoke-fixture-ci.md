# R2 Wave89 exact candidate verification

Candidate `ab48a7953dd07fc479407cf80a58f37e2c0c6a83` contains the bounded
[smoke prerequisite and cleanup correction](2026-10-17-r2-smoke-fixture-correction.md).
The original integrated review, required CI success and separate smoke failure
are preserved. No new independent review is claimed for the two QA files.

All-event/all-workflow exact-head queries found no matching run before each
dispatch, and the remote branch SHA matched. Each workflow was dispatched once:

| Workflow | Exact run | Created UTC | Full recovery reporting bound |
| --- | --- | --- | --- |
| CI | [39037896827 attempt1](https://github.com/sihioov/Lagrange/actions/runs/39037896827) | 2026-10-17 13:45:46 | 15:30:46UTC |
| Research worker smoke | [39037899130 attempt1](https://github.com/sihioov/Lagrange/actions/runs/39037899130) | 2026-10-17 13:45:48 | 15:30:48UTC |

Both are workflow_dispatch on excited-dolphin. The initial propagation query
showed only CI; no second dispatch was issued. A subsequent read recovered
both exact identities. No old or new run was rerun or cancelled.

Frozen1155pins/513committedblobs match. All1147 unchanged prior pins remain;
only the two QA files changed. Inventory carries the110 previous required
definitions and adds7 cleanup cases,117 total, plus all previous named groups,
platform exclusions and the existing104 affected-module definitions. All are
unexecuted for this new candidate until fresh named evidence is recovered.

Inputs: `target/r2-daily-beta/wp-7/wave89/ci-inputs.json`, SHA256
`a64b245a887c95bc700220e99e3ccab1a6754822b8a5f9cc747fe59ace355128`.
Inventory SHA256
`0ebf3eb4d68738d6e035ef306fdeb1982316047b051eeb83fe861e3228900f2c`.
Private receipts are in `target/r2-daily-beta/ci/ab48a79/`.
Raw logs remain in memory only; persisted evidence is finite metadata/named
outcomes, actual checkout SHAs and full-log hashes. Unknown nested outcomes
remain UNKNOWN.

At13:47UTC format completed successfully; its full log and actual checkout SHA
matched the candidate. Other jobs and the separate functional smoke were still
active. Full candidate acceptance remains pending. A static PASS alone cannot
close functional smoke, and its functional PASS also requires successful
cleanup and terminal job success.

Self-heartbeat0257875e continues the remaining R2 dependencies after recovery;
it is not deleted merely because a package ends. Renew or replace it before
expiry17:40:44UTC with an explicit bounded handoff. Native Ralph still reports
blocked; no active-state claim is made. The [current host packet](2026-10-17-r2-current-host-handoff.md)
records the fresh access failure and prepared next steps. No local project
execution, actual image build, production change or new worker occurred.
