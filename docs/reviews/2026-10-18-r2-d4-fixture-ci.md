# R2 D4 fixture correction: hosted verification

R2-04/09, Wave94. Candidate
`2a165fe577e833529134627e980a129b9717ae6f` contains the single self-test
[correction](2026-10-18-r2-image-fixture-feature-correction.md). Its required
CI scope is accepted after full recovery at02:23:58 UTC. The separate functional
smoke is still running, so hosted acceptance remains open. The old candidate's
CI failure and separate functional success remain preserved.

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

At02:20:03 UTC, all eight CI jobs were terminal SUCCESS: policy, format,
strict Clippy, workspace, PostgreSQL, Web, Python and the required aggregate.
Eight complete log hashes and seven actual checkout SHAs were recovered.
All1,174 frozen file pins and519 exact-candidate committed blob bytes matched.
The immutable CI-only receipt is
`target/r2-daily-beta/ci/2a165fe/required-ci-evidence.json`, SHA256
`8ec43bc2497c7ab36c4415ad0889c2a022ba36a91bdf012c62b1d4fcbf4cd362`.

The118 required definitions and all15 image outcomes passed by direct named
rows. This includes the two D4 producer receipts and four missing/unexpected
feature rejections through the real validator. The four changed modules retain
all104 cases, the ten original correction cases and seven cleanup cases pass,
and all181 carried policy entries match. These groups overlap; their counts
must not be added as unique tests. Native restore fixtures printed eight exact
source/restored metadata observations of UID999/GID0/mode0700.

The36 workspace groups and33 carried groups matched qualified target/phase
rows. Workspace printed3,350 complete PASS rows and six ignored rows; the chart
lane separately printed six PASS rows. One fragmented non-required synthetic
anchor name and nested helper independent attribution remain UNKNOWN. Three
non-Linux source cases remain EXCLUDED_LINUX, never inferred successful from
an empty target set. All231 Cargo headers/root blocks closed.

PostgreSQL verified19 direct process exits0 without SKIP,146 named rows,
0060's32 names and73 subcases, nine harness rows,83 bootstrap cases, and both
sets of62 migration SHA384 checksums against the source. The0039 guard,
catalog boundaries, rerun and owned-resource cleanup passed. Web matched
34 supervisor cases, six contract and18 UI markers, and129+14 browser identities;
all passed with cleanup/reservation checks, no skip, retry or truncation.
Strict Clippy included all-targets/all-features/locked and `-D warnings`;
workspace format passed. Python's495 passed/10 skipped remain aggregate-only.

Functional smoke39088099684 attempt1 was still active at02:20:03 UTC. Recover
its complete log/hash, actual checkout, publication and repeat invariance,
and all ten cleanup outcomes before hosted acceptance. Source and frozen
inputs remain fixed. No production-image or final-R2 acceptance follows from
these hosted synthetic fixtures. Raw logs remain in memory only.

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
