# GitHub Actions CI Design

## Goal

Move the long, repeatable verification work from a developer workstation to
GitHub-hosted runners without adding a nightly schedule. Pull requests must
prove the Rust workspace against a disposable PostgreSQL instance and a
freshly generated Phase 0 dataset. A push to `main` must additionally prove
the deployment path with the existing research-worker Compose smoke test.

The repository is public, so standard GitHub-hosted runners are free and do
not consume the private-repository monthly minute allowance. The design still
minimizes redundant work and treats the standard runner's 14 GB SSD as a hard
constraint.

## Trigger Policy

Two workflows divide the work by cost and purpose:

1. `.github/workflows/ci.yml`
   - Runs for every `pull_request` targeting `main`.
   - Runs once more for every `push` to `main`, including a merge commit.
   - Supports `workflow_dispatch` for an explicit manual rerun.
   - Does not define `schedule`.
2. `.github/workflows/research-smoke.yml`
   - Runs for every `push` to `main`.
   - Supports `workflow_dispatch`.
   - Does not run for pull requests and does not define `schedule`.

The PR workflow exposes a stable required-check job name. Enabling that check
in GitHub branch protection remains a repository setting; the workflow makes
the check available but does not mutate remote repository settings.

## Runner and Job Boundaries

All jobs use standard `ubuntu-latest` GitHub-hosted runners. Long work is split
across fresh virtual machines so build products do not compete for one 14 GB
filesystem:

- `policy`: pinned-action audit, foundation validation, and repository CI
  contract tests.
- `format`: `cargo fmt --all -- --check` after the existing formatting drift is
  mechanically normalized once on this branch.
- `clippy`: workspace, all targets, all features, warnings denied.
- `workspace-tests`: Phase 0 generation, disposable PostgreSQL, and
  `cargo test --workspace --locked --no-fail-fast`.
- `research-smoke`: the existing full Bash Compose smoke on its own runner.

Jobs do not cache or upload `target/`. Local measurement reached 12.59 GiB, so
target caching is both too large for the repository cache quota and unsafe for
the runner disk. `CARGO_INCREMENTAL=0` is set for Rust jobs. The workflows do
not run a separate `cargo build` before Clippy or tests, because those commands
already compile what they need.

Every job has an explicit timeout. Pull-request concurrency cancels a stale run
when a newer commit is pushed to the same PR. A completed or currently running
`main` verification is not cancelled by an unrelated branch update.

## Phase 0 Data Preparation

`data/phase0` remains ignored and is never committed. A tracked CI preparation
command materializes it from `tests/golden/phase0/synth_data.py` into an
explicit destination. The command uses the same deterministic seed, 260
sessions, three instruments, schema, and path layout as the golden runner, but
does not execute the Nautilus backtest.

The preparation command must:

- refuse an unsafe destination outside the repository workspace;
- start from an absent or empty destination so stale data cannot mask a bug;
- write the curated Parquet partitions required by `job-queue` tests;
- validate 780 total bars, 260 per instrument, and the expected instrument set;
- print a small, non-secret summary including the generated root and counts;
- return non-zero for schema, count, or path drift.

The `workspace-tests` job installs only the pinned PyArrow version needed for
materialization, generates the dataset locally, runs the tests, and lets the
ephemeral runner delete it at job end. It is not uploaded as an artifact or
stored in a cache. A developer can run the same tracked command locally, so CI
does not hide a special data-production path.

## Disposable PostgreSQL

The workspace test job starts the same digest-pinned PostgreSQL image used by
`deploy/qa/qa-db.compose.yml`, bound only inside the GitHub job. It waits for
`pg_isready` and exports the supervisor `DATABASE_URL` expected by the existing
scratch-database integration harnesses.

The database contains only the fixed QA password already committed in the
disposable Compose contract. No production database, KRX credential, or GitHub
secret is read. The service and all scratch databases disappear with the
runner.

## Workflow Security and Reproducibility

- Workflow permissions default to `contents: read`.
- Checkout does not persist credentials.
- Every third-party or GitHub action reference is a full commit SHA with a
  comment naming the reviewed release.
- Cargo uses `--locked`; Python package versions come from the committed lock
  contract or an exact matching version.
- Workflows contain no production endpoints and no secret-dependent branch.
- Shell scripts use strict mode and quote repository paths.
- A tracked workflow contract test parses the workflow files and rejects a
  `schedule`, mutable action tag, missing timeout, missing read-only permission,
  artifact upload of Phase 0, target caching, incorrect triggers, or an
  unpinned PostgreSQL image.

## Failure Semantics

Each job fails closed and reports the failing command directly in the Actions
log. PostgreSQL startup failure prevents tests from running. Phase 0 validation
failure prevents Rust tests from running. The full workspace test uses
`--no-fail-fast` so independent failing test binaries are all visible in one
run.

The Compose workflow delegates cleanup and evidence validation to
`scripts/qa/research-worker-smoke.sh`, which already uses unique project names,
scoped resources, and cleanup traps. No generated dataset, database volume,
container image, or test report is retained after a successful or failed job.

## Validation

Implementation follows test-first order:

1. Add a workflow contract test and observe it fail while workflows are absent.
2. Add Phase 0 preparation tests and observe them fail while the command is
   absent.
3. Implement only enough workflow and preparation behavior to turn those tests
   green.
4. Run the preparation command in a clean temporary root and run the previously
   failing `job-queue --test backtest_runner` target against it.
5. Run workflow syntax/contract checks, formatting, Clippy, the PostgreSQL-backed
   workspace suite, and the static research-worker smoke locally where the host
   supports them.
6. Push once and use the first GitHub-hosted execution as the authoritative
   Linux runner/disk/time validation. If runner disk pressure appears, split
   the affected job further; do not weaken or skip a gate.

## Out of Scope

- No nightly, cron, or other time-based trigger.
- No real KRX collection and no production credentials.
- No Phase 0 history expansion beyond the existing 260-session golden contract.
- No artifact retention or full Rust target cache.
- No paid larger runner and no self-hosted runner setup.
