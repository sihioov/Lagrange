# R2 first-rollout package: exact-candidate hosted verification

Roadmap R2-04/R2-09; Wave82. Status: RUNNING, recorded 2026-10-16.

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
