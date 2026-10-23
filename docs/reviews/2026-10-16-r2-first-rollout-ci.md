# R2 first-rollout package: exact-candidate hosted verification

Roadmap R2-04/R2-09; Wave82. Status: FAILED, full result recovered 2026-10-16.

[CI38888547029, attempt1](https://github.com/sihioov/Lagrange/actions/runs/38888547029)
completed at11:02:19UTC for candidate
`c6265ddffc51a034c760fc48cf0f9d6fe387decd`. All8 jobs, step outcomes and full
log hashes were recovered; actual checkout SHAs match in all7 execution jobs.
The required aggregate failed because policy failed. Workspace Rust, strict
Clippy, format, Web, Python and dedicated PostgreSQL jobs passed.

Of95 new definitions,91 passed and4 native pre-upgrade cases returned ERROR.
Those four stopped in `Snapshot/open_root` at `PREUPGRADE_VOLUME_PARENT`, before
clone assertions. Actual parent metadata was not printed and remains UNKNOWN.
The four new Rust C04 regressions and19 synthetic whole-flow fixture definitions
passed. The latter do not establish actual approved-base execution.

All current/carried required named groups matched PASS or explicit platform
exclusion. Canonical Rust result extraction recovered3351 workspace rows and6
separate chart rows; six explicitly ignored vendor/soak/helper definitions are
recorded separately. Counts alone are not unique test evidence; each required
case has a qualified result and executable target. Unprinted nested helper
rows/exits remain UNKNOWN.

Strict workspace/all-targets/all-features Clippy with `-D warnings` and workspace
format commands were observed. Python reports487 passed,10 existing skipped and
7 warnings. Browser lanes passed129/129 and14/14 with no skips/flakes/retries and
successful cleanup. Dedicated PostgreSQL passed19 no-SKIP targets and146 printed
named rows,62 current migration checksums on both installs,32 migration0060
cases/73 subcases,83 bootstrap-negative cases,0039 protection and cleanup.
All1074 frozen pins and502 candidate blobs matched. No package acceptance or
operational success is claimed from this failed candidate.

The run was created at 2026-10-16T10:38:21Z, using `workflow_dispatch`, branch
`excited-dolphin` and `.github/workflows/ci.yml`. The reporting bound is
2026-10-16T12:08:21Z. The completed result above supersedes the original launch wait.

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
carried Rust/PG/Web/policy gates were collected from this exact run.
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

Every job/step, full log digest, actual checkout SHA and required named result
is now recovered as sanitized evidence. The package remains failed until the
bounded fixture correction receives fresh verification.
The [previous Wave81 success](2026-10-16-r2-backup-migration-mode-ci.md) and all
earlier failures remain preserved within their original scopes.

Actual candidate images and approved-base host execution, diagram rendering
and integration, independent integrated review, concrete G3/G4 operations,
real Owner use and three consecutive trading days remain outstanding. R2 and
final `C_R2` remain incomplete. No local project execution, new worker,
provider request or production action occurred in this launch.

## Bounded follow-up disposition and diagram preparation

The native fixture creates its own labelled Docker volume but does not establish
the already-required private immediate parent before capture. Full run
recovery is now complete. Correct only that fixture setup: verify the exact owned fresh volume,
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
