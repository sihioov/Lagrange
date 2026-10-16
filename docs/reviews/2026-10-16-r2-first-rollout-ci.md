# R2 first-rollout package: exact-candidate hosted verification

Roadmap R2-04/R2-09; Wave82. Status: RUNNING, recorded 2026-10-16.

Partial recovery at 10:43UTC: policy failed at the pre-upgrade schema59
isolated recovery step. All four new native cases stopped in `Snapshot` /
`open_root` with the fixed code `PREUPGRADE_VOLUME_PARENT`, before their
restored-cluster assertions. The exact parent metadata was not printed and
is UNKNOWN. Of 91 new Python definitions, 87 have one PASS row and four have
ERROR rows. The 19 whole-flow fixture tests passed after that failed step;
they still do not establish actual approved-base execution. The four new
Rust definitions remain pending collection.

Subsequent recovery: Web and dedicated PostgreSQL also completed SUCCESS.
Six terminal full-job log digests and actual checkout SHAs now match the
candidate. Strict workspace/all-targets/all-features Clippy with `-D warnings`
and workspace formatting commands were observed. Python reports 487 passed,
10 existing skipped and 7 warnings. Web browser lanes passed 129/129 and14/14,
with no skips, flakes or retries and successful cleanup. PostgreSQL has19
successful no-SKIP targets,146 printed named rows,62 current migration
checksums matched on both installs,32 migration0060 cases/73 subcases,
83 bootstrap-negative cases,0039 rollback protection and owned-container cleanup.
Unprinted nested helper rows remain UNKNOWN. Workspace Rust tests are still
running; the required aggregate has not completed.
All frozen executable inputs remain unchanged. No correction or redispatch
will occur before full run recovery.

Candidate `c6265ddffc51a034c760fc48cf0f9d6fe387decd` is running in
[CI 38888547029, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38888547029).
The run was created at 2026-10-16T10:38:21Z, using `workflow_dispatch`, branch
`excited-dolphin` and `.github/workflows/ci.yml`. The reporting bound is
2026-10-16T12:08:21Z. No result or acceptance is inferred from launch.

The exact-head query across all events/workflows returned zero before push and
again after push. The remote SHA matched the candidate before the single
dispatch. Exactly one matching attempt-1 run was recovered. No previous run
was rerun, cancelled or duplicated. Later documentation commits do not change
the pinned executable candidate.

## Frozen scope

- Six-file backup installation transition and its 22 regression definitions.
- Pre-upgrade cold-cluster/file recovery and bounded maintenance command,
  with 50 regression definitions.
- C04: adapt the installed `/data/raw` leaf to the existing reader's data-root
  contract, with four regression definitions.
- [Approved-base whole-flow driver](../runbooks/r2-approved-base-wholeflow.md)
  and 19 synthetic fixture/mocked-driver definitions. CI runs those 19 tests;
  it does not run the private approved-base host sequence.

These are 95 new written definitions. Their individual outcomes and all
carried Rust/PG/Web/policy gates require collection from this exact run.
Group lengths will not be summed as unique executed cases. Nested helper
results that are not printed remain UNKNOWN.

The source freeze contains 1,074 pins and 502 committed blobs. All 1,049
unchanged prior pins matched. The four changed prior source pins and twelve
new executable paths are registered. All 76 earlier Wave82 definition names
remain; 19 were added. Coordinator text/source review, scoped earlier Rust2024
formatting and whitespace checks are not independent review or compilation.

Private receipts under `target/r2-daily-beta/wp-4-integration/`:

- `wave82-first-rollout-source-inputs.json` — SHA256
  `811ffe8bc33ea97ee59f5560af6c6d552fb9f3e52236fd767f8ac927722f464c`.
- `wave82-first-rollout-definition-inventory.json` — SHA256
  `91135631affbee0352b01206ef212555adc1f2fe28d6e86c6874a0714d0687e8`.
- `wave82-first-rollout-ci-launch.json` records the dispatch and recovery bound.

## Remaining evidence

Recover every job/step, full log digest, actual checkout SHA and named result;
retain only sanitized evidence. No compiler/test acceptance is claimed yet.
The [previous Wave81 success](2026-10-16-r2-backup-migration-mode-ci.md) and all
earlier failures remain preserved within their original scopes.

Actual candidate images and approved-base host execution, diagram rendering
and integration, independent integrated review, concrete G3/G4 operations,
real Owner use and three consecutive trading days remain outstanding. R2 and
final `C_R2` remain incomplete. No local project execution, new worker,
provider request or production action occurred in this launch.

## Bounded follow-up disposition and diagram preparation

The native fixture creates its own labelled Docker volume but does not establish
the already-required private immediate parent before capture. After full run
recovery, correct only that fixture setup: verify the exact owned fresh volume,
stopped generation and descriptor/name identity, then tighten its own parent
to root:root0700. Preserve the production guard and its positive/negative
regression. Actual failed parent mode remains UNKNOWN. No production permission
change is authorized or inferred from this fixture correction.

The isolated I6C diagram draft now covers the maintenance/cold recovery/backup
transition components and refreshes C04 citations.303 citation ranges resolve
against the current source; this is only a text/source checkpoint. PUML files
remain outside this candidate. PNG rendering, integration and independent review
remain open. The private checkpoint is
`target/r2-daily-beta/wp-4-integration/I6C-static-checkpoint-c6265dd.json`.
