# R2 D4 fixture correction: hosted verification

R2-04/09, Wave94. Candidate
`2a165fe577e833529134627e980a129b9717ae6f` contains the single self-test
[correction](2026-10-18-r2-image-fixture-feature-correction.md). It is not
accepted. The old candidate's CI failure and separate functional success
were fully recovered before this source change.

## Exact executions

The remote branch SHA matched and the all-event/all-workflow exact-head query
returned zero before dispatch. Each workflow was dispatched once with exit0.
Both runs were observed in progress at01:53:41 UTC on2026-10-18:

| Workflow | Run / attempt | Created UTC | Full recovery reporting bound UTC |
| --- | --- | --- | --- |
| CI | [39088097930 /1](https://github.com/sihioov/Lagrange/actions/runs/39088097930) | 01:53:23 | 03:38:23 |
| Functional smoke | [39088099684 /1](https://github.com/sihioov/Lagrange/actions/runs/39088099684) | 01:53:25 | 03:38:25 |

Both use `workflow_dispatch`, branch `excited-dolphin`, and the exact candidate.
No old run was rerun, cancelled or duplicated. Later documentation commits do
not change this revision. Source and frozen inputs remain fixed until both
executions have terminal full recovery.

## Evidence requirements and partial recovery

`target/r2-daily-beta/wp-8/wave94/ci-inputs.json` contains1,174 pins and519
committed blobs, SHA256
`e8c4a45635e189c5fcedf8e0bf4ee9827c2076c02b023d5b344867be331d11d4`.
The inventory SHA256 is
`31fa3e45602e8d9fc96a84af3728fc2d231d6194358c857a460683a1e03ace8c`.
Of the prior1,166 pins,1,165 remain unchanged and only the self-test changed.
The118 prior definitions, nine image-suite markers and six new qualified D4
outcomes are required. All carried Rust/PG/Web/policy groups and platform
exclusions remain required; old results are not successor evidence.

Format and Python jobs have succeeded; their two full logs and actual checkout
SHAs were recovered. The other execution jobs and functional smoke were still
running at the latest partial observation. The Python total is aggregate-only
evidence. No118-definition or15-image-outcome acceptance is claimed. Sanitized
partial evidence is in `target/r2-daily-beta/ci/2a165fe/ci-evidence.json`.

Full recovery must include eight CI jobs and one smoke job, all steps/pages,
nine complete log hashes, eight actual checkout SHAs, direct qualified rows,
strict Clippy/workspace format, PG19 process exits/146 names/0060 32+73/
83 bootstrap cases/62 checksums twice/0039/cleanup, and actual functional
publication/idempotence plus ten cleanup outcomes. Unprinted nested helper
rows/exits stay UNKNOWN. Raw logs remain in memory only.

## Continuation

Heartbeat `c4f4db00` was confirmed active for the same coordinator and persisted
before predecessor `e9dd6173` was deleted. It expires13:54:17.607 UTC; renew
by13:00 UTC and before the47th tick. This tracks the remaining R2 work after
this package. Native Ralph remains blocked; the heartbeat does not protect
against daemon outage.

The third production interruption remains unauthorized. No new production
probe, SQL, build or lifecycle action occurred. Actual twelve verified images,
approved-base host whole flow, G4 deployment, Owner use, three consecutive
trading days and final C_R2 remain open.
