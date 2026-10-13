# Development verification

This procedure supports the development-efficiency plan tracked in
[`../superpowers/plans/2026-10-13-development-efficiency.md`](../superpowers/plans/2026-10-13-development-efficiency.md).
That plan's WP checkboxes and history track tooling work separately from the product roadmap.
It does not advance or revise product roadmap stages.

## Current availability

The Phase 1 CLI and Phase 2 cache/resource source are implemented. Focused checks cover the CLI,
cache contracts, fake child lifecycle, and export promotion. Hosted cold/warm proof on a committed
candidate remains pending. Existing hosted jobs remain the final acceptance gates.

## Staged workflow

Proceed in this order, widening only as needed:

1. **Plan** the changed paths and proposed profiles. The `plan` command is read-only: it
   prints its selection and does not install tools, run checks, or start services.
2. **Preflight** the selected profile and stage. It checks the required tools, versions, declared
   environment, paths/permissions, Docker access, and only the ports that the selected harness will
   use. It must not install software, alter permissions, stop listeners, or claim that a successful
   port probe reserves a port. A preflight pass is not a test pass.
3. **Fast** checks catch inexpensive formatting, syntax, policy, and static-contract failures before
   focused or integration work. They do not start application servers or substitute for tests.
4. **Focused** checks run the bounded tests/lints mapped to changed paths. Local Rust requires an
   explicit real workspace package. Web focused runs OpenAPI and Vitest after lint/typecheck. If
   Rust impact is unclear, use the hosted workspace integration plan.
5. **Integration** uses existing harnesses and CI job commands with their documented prerequisites.
   Do not run full Rust, database, browser, or smoke integration on the production host by default.
   A hosted job performs its own preflight and runs its existing command once; do not duplicate that
   same check through the dispatcher.
6. **Final CI** runs on the confirmed commit and retains every existing required job and the
   `required` aggregate. Neither path filtering, a reused result, nor `verify-all` alone proves that
   final CI passed.

Stop before an expensive stage when its prerequisite fails. Exit status is 0 for success, 1 for a
check failure, and 2 for invalid input or an unmet precondition. Preserve the underlying command's
exit code in its receipt.

## CLI usage

The examples below run from the repository root. From a subdirectory, invoke the script by its
absolute path; the dispatcher resolves the same repository root, argv, and cwd in either case.
It does not fetch a missing Git ref.

```bash
# Print changed paths and proposed checks without running them.
python3 scripts/qa/dev-check.py plan --base origin/main --format json

# Check prerequisites only; no installation or test execution.
python3 scripts/qa/dev-check.py preflight --profile web --stage focused --environment local

# Run promised fast checks and allow eligible deterministic receipt reuse.
python3 scripts/qa/dev-check.py run --profile web --stage fast --reuse

# Run focused QA checks selected by the profile.
python3 scripts/qa/dev-check.py run --profile qa --stage focused
```

Profiles are `docs`, `rust`, `web`, `python`, `qa`, `db`, and `all`; stages are `fast`, `focused`,
and `integration`. `plan` can return multiple profiles. An unknown or shared configuration path
widens the plan rather than passing without checks. Rust focused execution requires
`--package <workspace-package>`. `all/focused` refuses execution and prints the complete hosted
plan. `run ... --stage integration` prints a plan without running a child.

The QA quick profile runs only its listed quick/static checks. Integration plans point to existing
hosted jobs or harnesses; they do not claim `verify-all` is all of CI. Web lint/typecheck commands
must run through `npm run ... --workspace @lagrange/web`, which gives Biome its package cwd at
`apps/web`.

## Changed paths to checks

| Changed paths | Fast | Focused / integration | Final CI |
|---|---|---|---|
| Markdown, docs, repository instructions | Diff whitespace, references, and scope checks | Add the affected code profile if the change also alters product structure | Existing required aggregation |
| `crates/**`, `data-pipelines/collectors/**`, `apps/api-server/auth/**` | `cargo +1.97.1 fmt --all -- --check` | Explicit package: `cargo +1.97.1 clippy -p <package> --all-targets --all-features --locked -- -D warnings` and `cargo +1.97.1 test -p <package> --locked`; full workspace is hosted integration | Workspace Clippy, chart acceptance, and workspace tests |
| `apps/web/**` and Web npm configuration | `npm run lint --workspace @lagrange/web`; `npm run typecheck --workspace @lagrange/web` | Focused: OpenAPI and Vitest. Hosted integration: production build and `bash scripts/qa/candidate-web-e2e.sh` | Existing Web job |
| `nt/**`, Python checks, or fixtures | AST syntax check for changed Python files | Confirm locked environment and Phase0 prerequisites; `uv run --project nt pytest -q`, with focused selection recorded in argv where appropriate | Existing Python and mixed-workspace jobs |
| `scripts/qa/**`, `scripts/ci/**`, workflows | `bash -n` for changed shell, Python AST, workflow YAML/contract checks | Relevant unittest/self-test and impacted existing harness | Policy and impacted jobs, then all required jobs |
| `deploy/db/**` or migration contracts | `bash deploy/db/integration-validation/static-check.sh`; `bash deploy/db/integration-validation/validate.sh --self-test` | `bash deploy/db/integration-validation/validate.sh --evidence-dir <new-private-path>` | Existing PostgreSQL job; do not skip runtime validation |
| `Cargo.lock`, workspace manifests, `.cargo/**`, toolchain, shared config, or unclassified paths | Applicable fast checks across the repository | Every affected area; use full integration when impact is uncertain | Every required job |

Keep the legacy `scripts/verify-all.sh` and `scripts/verify-all.ps1` checks intact when aligning their
Rust arguments with CI. They are useful local entry points, not a replacement for hosted CI. The
required Rust CI commands include:

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p api-server --test http_owner_equity_v2_chart --locked
cargo test --workspace --locked --no-fail-fast --features kis-client/test-support
```

## Preflight and receipts

Preflight checks only prerequisites needed by the selected profile: pinned tools, declared
non-secret environment-variable presence, accessible input/output paths, Docker availability when
needed, and harness-specific ports. It reports missing npm/uv/Chromium preparation commands without
installing them. Existing harnesses remain responsible for runtime port reservation, process
identity, and cleanup. A probe is not a reservation. Report port and observed errno for bind/access
failures, not credentials or secret values.

Receipts are written to ignored `target/dev-check/<run-id>/` private directories. JSON records
include schema version, check ID, target HEAD, input fingerprint, argv, cwd, start/end and duration,
original exit code, verdict, and evidence path. Fingerprints cover relevant source (including new and
deleted paths), lock/manifests/configuration, dispatcher/profile version, tool versions, OS/architecture,
and permitted non-secret environment conditions. Local Rust commands select `+1.97.1` explicitly and disable
rustup auto-install; formatting also removes `RUSTFMT` overrides so the checked formatter executes.
Receipts record these effective environment overrides. Compare inputs before and after a check; a changed
input, timeout, partial run, parse failure, or unknown external condition makes it ineligible for
reuse. Symlinks (including ancestors), nonregular files, and unreadable inputs block execution
instead of following unknown data. Failed commands retain bounded, redacted diagnostics in the
private receipt directory; the CLI prints the path and the receipt records its hash.

Automatic reuse is initially limited to deterministic checks whose complete inputs can be bound,
such as formatting, shell/Python syntax, and manifest-only pin checks. Other checks may be recorded
but are not automatically reused. DB, browser, service-health, network, clock, permission, process,
and final required CI checks run every time. A reused result is labeled `REUSED` with its source
receipt and original target commit, never as a new `PASS` for the current commit. Package/build
caches and receipt reuse are separate; a cache hit does not satisfy a check.

## Explicit local heavy execution

Use the same private runtime directory across worktrees. The CLI verifies Linux, directory owner,
mode 0700, symlink-free ancestry, and a regular owner-held mode-0600 lock file. It uses a fixed
`lagrange-dev-check-heavy.lock` name. If `XDG_RUNTIME_DIR` is unavailable, supply one shared private
location outside worktrees; the CLI does not create or repair that directory.

```bash
# Requires the pinned toolchain, package prerequisites, and an existing private runtime directory.
python3 scripts/qa/dev-check.py preflight --profile rust --stage focused --package domain
python3 scripts/qa/dev-check.py run --profile rust --stage focused --package domain
# Alternative: append --heavy-lock-dir /absolute/shared/private/runtime

# Print every hosted job and command when impact needs full integration.
python3 scripts/qa/dev-check.py run --profile all --stage integration
```

The slot is acquired before a heavy child starts. A competing invocation exits before spawning;
Rust children receive `CARGO_BUILD_JOBS=2`. The lock remains through communicate/reap, without a
parent-only timeout kill. Ctrl-C waits for the owned child to exit before releasing the slot. The
child inherits the lock descriptor so an abrupt CLI termination does not release the direct
child's slot prematurely. Detached descendants and existing harness cleanup remain outside this
small lock's supervision. The `ci` label only bypasses the local slot on an actual GitHub Actions
runner; it is not a local bypass flag.

Package test prerequisites are a small explicit map backed by the source files listed in
`check-profiles.json`. Clippy alone needs no DB or Phase0. All 16 actual workspace names are accepted.

| Package test scope | Prerequisites beyond the Rust toolchain |
| --- | --- |
| `domain` and the other unlisted pure packages | No DB, Python environment, or Phase0 requirement |
| `market-data` | Prepared Phase0 curated bars tree |
| `job-queue` | Prepared Phase0, locked NT environment/uv, Python 3.12 with pyarrow selected by `PYTHON` (default `python`), fixed intraday disposable QA URL |
| `api-server` | Locked NT environment/uv and the same Phase0 generator interpreter; fixed intraday disposable QA URL |
| `collectors`, `result-model`, `migration-contract` | Configured, reachable disposable loopback DB |

A package DB preflight checks URL routing/contract and TCP reachability, without authenticating or
claiming schema readiness. It does not bind the occupied DB port to test whether it is free. The
hosted workspace and PostgreSQL jobs probe free startup ports before their harness starts the DB.
Their preflights follow pinned toolchain/dependency preparation. Hosted Clippy narrows preflight
with `--check-id cargo-clippy-workspace`; the smoke host Cargo call uses
`--check-id cargo-test-workspace`. Preflight runs no compile, install, or test.

Local independent workers default to two, at most three plus a coordinator, with separate write
scopes. Light checks may run two at a time with distinct output paths. Existing hosted job VM
parallelism remains; heavy commands within each VM run sequentially. Production retains its
one-service, Cargo-2, Compose-1 policy and existing batch health gates.

## Hosted cache policy and evidence

Separate `actions/cache/restore` and `actions/cache/save` use verified v5.0.5 commit
`27d5ce7f107fe9357f9df03efb73ab90386fccae` (Node 24 runtime). Cache errors allow the real command
path to proceed. Installs/tests/builds always run, including `npm ci` and locked uv sync. The pinned
setup-node action has `package-manager-cache: false`, preventing a second implicit npm cache.

| Cache | Allowed path set |
| --- | --- |
| Cargo downloads | Only registry index/cache/src and git db/checkouts under the public Cargo home |
| Cargo output | Stable job-specific `RUNNER_TEMP/lagrange-cargo-target-*` for Clippy, workspace, PostgreSQL, and smoke host Cargo |
| npm downloads | Web job's `RUNNER_TEMP/lagrange-npm-cache-web/_cacache` |
| uv downloads | Workspace/Python job-specific `RUNNER_TEMP/lagrange-uv-cache-*` |
| Docker layers | Stable per-service `RUNNER_TEMP/lagrange-smoke-build-cache/import/<service>` |

Keys bind the validated ASCII namespace, OS/image/architecture, actual tool versions, locks,
manifests/configuration and relevant flags/feature lane. Source revision suffixes permit compatible
prefix restoration while Cargo/installs still validate current inputs. Docker identity includes
BuildKit/Buildx, Compose/Engine and build inputs including base digests. Cold/warm paths are stable
because GitHub's cache version also incorporates the path set. Credentials/config, installed
`node_modules`/`.venv`/`.next`, repository target/evidence, provider data, DBs, receipts and logs are
excluded. The temporary Compose override only adds two service build-cache fields.

PRs restore only. Saves require a successful trusted same-repository push/dispatch, a successful
restore miss or prefix match, and successful job checks. Exact hits skip saves of immutable keys.
The smoke also requires a validated new export before saving each Docker store. Its private
`docker-container` builder is passed explicitly; the default builder and shared caches are untouched.
Research-worker builds to completion before candidate-runner starts. Export goes to separate `new`
paths with `mode=max,ignore-error=true`; missing/failed exports do not invalidate a completed real
build and are never promoted/saved as an export success. Layer caching does not export Cargo cache
mounts. A validated new export replaces only that job's restored service directory, avoiding stale
local blobs without deleting remote cache entries or pruning shared builders. The no-cache context
audit and six fresh Raw probe volumes, locked downloads, and offline
compile run on every smoke.

For hosted layer caching, each service's Compose `build --print` output is reduced to one validated
Bake target before execution with the private builder and `--load`. Compose also prints unselected
`db-migrate` and `db-role-bootstrap` definitions; only these known auxiliary definitions may be
present, and they are removed from the executed graph. The default group must select only the
requested service and its values are preserved. Both Compose 2.38 local-load output
(`type=docker,load=true,push=false`) and newer `type=docker` output are accepted exactly; registry
push, destination paths, and additional outputs remain rejected. Compatibility checks exercise the
actual runner-image Compose version as well as the local version before a new hosted proof.
Bake receives only the exact
service export `fs.write` grant, plus the import `fs.read` grant when that store is configured.
Unknown targets, selected target references, outputs, privileges or cache paths stop before the
build with a fixed rejection code. Local
and unavailable-cache paths retain the original single-service Compose command. This uses Bake's
[explicit filesystem grants](https://docs.docker.com/reference/cli/docker/buildx/bake/#allow)
without changing production builds or disabling filesystem admission.

`cache-evidence.py record` writes private JSON and bounded step summaries with workflow/job/SHA,
namespace, keys, exact/prefix/miss/unknown restore state, save outcome, bytes or unavailable, and
actual command durations. A successful save action outcome is `attempted_unverified`: this action
has no outputs and can return success after a warning. Persistence requires the upload log,
cache inventory and warm matched key. Cache transfer durations and resource metrics not available
from the commands remain unavailable; a zero command stage means no separate command of that type.
Smoke separates Docker image command time from the host Cargo command time (compile plus synthetic
ingest); remaining probe/functional work is test time. This is not isolated compiler performance.

```bash
# Compare recovered records in an owner-private directory; no token/environment dump is an input.
python3 scripts/ci/cache-evidence.py compare --cold /private/cold.json --warm /private/warm.json \
  --output /private/comparison.json --summary /private/comparison.md
```

Comparison rejects malformed/missing fields, mismatched workflow/job/SHA/namespace/compatibility
key and inconsistent duration totals. Unknown durations prevent a fabricated total. Peak RSS/cgroup
and OOM observations are unavailable until measured. The preparation snapshot on 2026-10-13 showed
zero active cache entries/bytes; the configured storage-limit endpoint returned 402, so the configured
limit remains unknown. This response does not establish that restore/save is blocked. Observe real
cache usage, sizes and evictions during proof without changing billing, limits or permissions.

After the coordinator freezes the candidate, recover/deduplicate any qualifying runs, then perform
one cold and one warm run per workflow on the same SHA and cache namespace. The initial bound is
two CI plus two smoke runs; do not delete caches or blindly retry. Preserve private logs and compare
restore/save transfers with install/build/test durations. All eight CI jobs and seven required needs
remain mandatory. Hosted policy parses `verify-all.ps1` with pwsh's Parser without executing it;
local parser verification is pending where pwsh is unavailable.

## Final-check and scope limits

The `44077a4` cold proof on 2026-10-13 completed its
[research smoke](https://github.com/sihioov/Lagrange/actions/runs/38365911022), including the two
sequential image builds, validated Docker exports and functional PASS. Eight unique cache uploads
across CI and smoke matched REST entries (3,862,531,733 archive bytes). Smoke wall time was 3,426
seconds; this is a cold measurement, not a demonstrated speedup. Its
[CI run](https://github.com/sihioov/Lagrange/actions/runs/38365843138) failed one service-status DB
fixture bootstrap, so no warm run was dispatched. The bounded test-fixture correction and a new
same-SHA cold/warm proof remain pending. Individual smoke trap cleanup results were not emitted;
do not infer them from workflow success.

The hosted workflow remains the final gate and keeps its required jobs and aggregation. This runbook
does not authorize provider access, trading, deployments, or a change to production build limits.
It does not establish cache effectiveness or runtime performance. WP-3 hosted measurements and
Owner/coordinator acceptance remain separate from implementation and focused checks.
