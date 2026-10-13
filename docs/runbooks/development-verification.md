# Development verification

This procedure supports the development-efficiency plan tracked in
[`../superpowers/plans/2026-10-13-development-efficiency.md`](../superpowers/plans/2026-10-13-development-efficiency.md).
That plan's WP checkboxes and history track tooling work separately from the product roadmap.
It does not advance or revise product roadmap stages.

## Current availability

The Phase 1 CLI and profile map are implemented. Local QA regressions and CI contract checks
pass; the quick QA profile and eligible syntax-result reuse have been exercised. Phase 2 CI cache
and resource integration are still pending. Existing hosted jobs remain the final acceptance gates.

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
4. **Focused** checks run the tests/lints/builds mapped to changed paths. For Rust, specify the real
   workspace package when impact is known; if dependency impact is unclear, widen to the workspace.
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
widens the plan rather than passing without checks. Rust focused selection accepts
`--package <workspace-package>`.

The QA quick profile runs only its listed quick/static checks. Integration plans point to existing
hosted jobs or harnesses; they do not claim `verify-all` is all of CI. Web lint/typecheck commands
must run through `npm run ... --workspace @lagrange/web`, which gives Biome its package cwd at
`apps/web`.

## Changed paths to checks

| Changed paths | Fast | Focused / integration | Final CI |
|---|---|---|---|
| Markdown, docs, repository instructions | Diff whitespace, references, and scope checks | Add the affected code profile if the change also alters product structure | Existing required aggregation |
| `crates/**`, `data-pipelines/collectors/**`, `apps/api-server/auth/**` | `cargo +1.97.1 fmt --all -- --check` | `cargo clippy -p <package> --all-targets --all-features --locked -- -D warnings`; related `cargo test -p <package> --locked`; DB-dependent tests use disposable DB validation | Workspace Clippy, chart acceptance, and workspace tests |
| `apps/web/**` and Web npm configuration | `npm run lint --workspace @lagrange/web`; `npm run typecheck --workspace @lagrange/web` | OpenAPI check, `npm test --workspace @lagrange/web`, build, and `bash scripts/qa/candidate-web-e2e.sh` as applicable | Existing Web job |
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
and permitted non-secret environment conditions. Local Rust formatter execution selects the pinned toolchain, disables
rustup auto-install, and removes `RUSTFMT` overrides so the checked formatter is the one executed.
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

## Final-check and scope limits

The hosted workflow remains the final gate and keeps its required jobs and aggregation. This runbook
does not authorize provider access, trading, deployments, or a change to production build limits.
It does not establish resource usage, cache effectiveness, or runtime performance. Those Phase 2
measurements and implementation belong to WP-3.
