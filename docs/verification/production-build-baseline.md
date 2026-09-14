# Production build baseline and measurement contract

## Status

This document defines the equal-condition measurement needed to choose a
production image-build structure. It is not performance evidence and does not
select a common builder in advance.

At the WP-1 launch point, the relevant product baseline is
`f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3` (B). The original structure is
`d1baf9da9b13fcb61649b1c26de56aed87a83418` (A). The planning launch identity
is `6f2571da8820df092bb4e54da795a487137fbd50`. C is valid only after its chosen
implementation is committed from a clean tree; record that exact C SHA in each
run, rather than treating a worktree as an identity.

The earlier report, [production-build-cache.md](production-build-cache.md),
proves an actual cold fixture run and an identical-input layer-cached repeat at
`008488c...` only. Its forced-warm fix at `3b0b88f...`, the deletion error
wording, all later semantic fixture cases, and any A/B/C performance claim
remain unverified. None of those are inherited as a passing result here.

WP-1 tooling and its fake-Docker self-tests can be code-complete while actual
Docker verification remains blocked. A report must keep those two states
separate.

The G1 journal-gate correction is code-complete only after its retained offline
self-test evidence is recorded below. It does not establish a current host
gate, an actual smoke pass, a warm fixture pass, or an A/B/C performance result.

## Fixed release boundary

Every A, B, and C measurement represents the existing twelve final
images/services, with their existing binary lists, entry paths, functions,
features/profiles, embedded commit values, OCI revisions, strict V2 manifest,
and installation, execution, and rollback contracts. A temporary benchmark
image is never a release image and its successful existence is not a release
success condition.

The official release procedure remains responsible for final image inspection
and strict V2 manifest publication. This measurement does not add an
image-only, cross-commit, or verification-skipping release path. It also does
not change runtime Compose services, profiles, networks, volumes, commands, or
the repository `.dockerignore`.

No run may perform a rollout, migration, provider call, credential read,
runtime service recovery, or cache prune. Builds remain one Compose/Docker
service invocation at a time, `COMPOSE_PARALLEL_LIMIT=1`, and
`CARGO_BUILD_JOBS=2`; a logical batch has at most three services and is only a
checkpoint, not a parallel build graph.

## Scenario contract

Run one scenario per benchmark invocation. For a warm comparison, both
detached source clones first receive the same unchanged-source warm-up. Each
then receives the corresponding temporary transformation below and a synthetic
child commit. The child commit makes the source tree, embedded code commit, and
OCI revision of each measured temporary image agree. The exact source patch,
source/tree/commit identities, scenario spec, and temporary Dockerfile
instrumentation are retained and hashed.

Consequently every measured scenario, not only `commit-only`, includes the
fixed release-contract commit transition. It is intentionally not hidden from
Cargo or image inputs. `commit-only` is the control that isolates this cost;
interpret a source scenario against that control when distinguishing a source
invalidation from the required embedded-revision transition.

| Scenario | Temporary input transformation | Meaning and limit |
| --- | --- | --- |
| `commit-only` | Synthetic child commit with an identical tree | Commit/revision input only; no source delta. |
| `rust-leaf` | Append a Rust comment to `crates/api-server/src/bin/api-server.rs` | Leaf API Rust input. |
| `rust-common-library` | Append a Rust comment to `crates/domain/src/lib.rs` | Shared internal Rust-library input. |
| `lockfile-comment` | Append a TOML comment to `Cargo.lock` | Lockfile invalidation only; it is not a dependency-version experiment. |
| `build-script` | Append a Rust comment to `crates/api-server/build.rs` | Build-script input. |
| `configuration-input` | Prefix whitespace to `configs/strategies/baseline-v1.json` | Semantically equivalent DB runtime-configuration input. |
| `web-only` | Append a TSX comment to `apps/web/app/layout.tsx` | Web source input. This follows the installed matching Next documentation for the temporary source edit. |
| `python-only` | Append a Python comment to `nt/backtest-worker/backtest_worker/__init__.py` | NT Python runtime input. |

The fixture smoke test separately exercises a real controlled external
dependency-plus-lockfile update. Do not infer that a lockfile comment measures
the cost of a new production dependency; Cargo dependency/profile/feature
changes are outside this work package.

Likewise, the production benchmark intentionally does not mutate the approved
embedded universe data: its content is protected by a compile-time hash
contract, so an arbitrary comment would test a broken contract rather than a
representative change. The fixture's embedded-data case is the semantic
coverage for that class until a separately approved production-safe input is
identified.

## Evidence and timing contract

The benchmark records the following for every run:

- tool SHA, source SHA/tree/temporary child SHA, exact scenario and
  transformation, source-patch SHA, temporary-Dockerfile/instrumentation SHA,
  Bash, Git, Docker, Buildx, builder, host OS/platform, Docker
  OS/architecture/kernel/root, and cache namespace/state;
- per-service end-to-end temporary image-build wall time, observed Cargo `RUN`
  duration, BuildKit/Cargo `Compiling` and `Fresh` package evidence, and a
  bounded temporary image ID plus OCI-revision inspection;
- preparation, warm-up or cold execution, scenario preparation, measured image
  builds, comparison, resource summary, and overall elapsed phases;
- sampled `MemAvailable`, `SwapFree`, repository free disk, and Docker-root
  free disk before/during/after every image build. The reported minimum
  available capacity is host pressure evidence, not process RSS, exact consumed
  memory, or a linker-time measurement.
- for every preflight and batch gate, probe/lookback exit status, validated
  entry and OOM-match counts, fixed first-gate range and current `until`, raw
  stdout/stderr SHA-256 values, and a fixed reason. Raw kernel JSON, including
  `MESSAGE`, remains private temporary data and is removed after hashing.

`cargo_ms` is the compiler `RUN` duration observed in the BuildKit output. It
must never be named linker time. Link time is `not-separated` unless a future
instrumentation method produces a separate, equally applied value for every
comparison target.

For the release-sized A/B/C comparison, use the official builder/checker and
record one additional enclosing timeline:

| Phase | Includes | Accounting rule |
| --- | --- | --- |
| Preparation | clean-revision check, cache-state setup, shared artifact preparation, and preflight gates | Count shared preparation once, before any dependent image. |
| Execution | each sequential Rust/Web/DB image build and any C common/role builder step | Count a shared builder once; do not add its elapsed time once per consumer. |
| Verification | every final-image/binary/revision inspection and strict V2 manifest issuance | Include this time in the total; a tag or image ID alone is insufficient. |

The full elapsed time is the interval from preparation start to strict manifest
completion. Retain the three phase durations and total as separate fields. If
C introduces a common or role-specific builder, define its vertices and their
dependency mapping before measurement and add its elapsed time to Preparation
or Execution exactly once. Do not reuse the current per-service parser to make
an unmodelled common vertex disappear from the total.

## Cache and comparison rules

Cold runs use a fresh namespace and `--no-cache`; warm runs use a distinct
namespace per revision, warm unchanged input first, then invalidate only the
consumed `RUN` input while allowing normal Docker layer reuse. Cache namespaces
and their prior state must be recorded. A clean empty namespace is not the same
thing as a warm namespace.

For each selected scenario, compare A to B and B to C under the same host,
toolchain, Docker builder/platform, resource gates, service order, cache mode,
scenario transformation, and measurement contract. Do not merge results from
different scenarios or add shared work multiple times. A valid C claim needs,
at minimum:

1. fixture semantic checks and actual deletion/restore evidence;
2. all twelve final images inspected with their expected revision/binary
   contract and a strict V2 manifest issued by the official flow;
3. measured lower duplicated internal compilation and lower whole-release
   elapsed time for the same representative scenario relative to B; and
4. an A comparison on the same terms, showing the effect relative to the
   original structure.

The historical approximately 98-minute seven-Rust-image observation and the
30--70 minute discussion range are context only. They are not an absolute goal,
baseline measurement, or performance guarantee.

## Required host gates

Re-query these immediately before every actual Docker/Rust invocation and at
every logical batch boundary. A prior passing observation cannot be reused.

- `MemAvailable` is at least 2,097,152 KiB and `SwapFree` is at least
  524,288 KiB; the thresholds may be raised but never lowered.
- `/proc` reads succeed; Docker's bounded platform/status fields and the
  selected background systemd service state are readable; the current cgroup
  proves the benchmark is running inside that low-CPU/low-I/O-priority service.
- Establish journal readability first with exactly
  `LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1`. It must exit
  zero with empty stderr and exactly one valid JSON kernel entry containing
  `__REALTIME_TIMESTAMP`, `__CURSOR`, `_BOOT_ID`, `_TRANSPORT=kernel`, and a
  string `MESSAGE`; its normalized boot ID must equal the current
  `/proc/sys/kernel/random/boot_id`. An unreadable `/proc` value, empty output,
  warning, error, malformed entry, or boot mismatch is failed journal evidence.
- Only after that probe succeeds, use exactly
  `LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json --since <fixed
  first-gate-UTC-minus-1800s> --until <gate-UTC> -n 1000`. The first actual gate
  fixes `since` for the run; each later gate records its own `until`. The query
  must exit zero with empty stderr and valid current-boot JSON whose timestamps
  lie inside that range; a 1,000-entry result is rejected as potentially
  truncated. A successful empty query is zero OOM only after the readable
  probe. The case-insensitive `out of memory|oom[-_ ]?kill|killed process`
  match blocks the run. A boot change before, during, or between gates blocks
  the run as well.
- Journal stdout/stderr and parser diagnostics are written only to a private
  temporary directory. Public `batch-resources.tsv` retains statuses, counts,
  range, SHA-256 values, OOM count, and fixed reason; it contains no kernel
  `MESSAGE`. Do not use `--quiet`, `--grep`, `2>/dev/null`, `|| true`, a masked
  pipeline, or a permission bypass for either journal read.
- No previous compiler/build process remains, the previous build exit is zero,
  and all operator-approved production health units and containers are healthy.
  A restarting/unhealthy `research-worker`, degraded systemd state, or any
  unresolved service fault blocks the run; WP-1 is not authorized to repair it.
- The launch uses the existing low-priority background systemd pattern and
  checks memory, swap, kernel OOM evidence, build exit, and service health
  between batches.

If a gate fails, retain the failure reason and stop before Docker build. Do not
weaken the assertion, suppress journal errors, or revive an expired image-only
exception.

## Reproduction sequence

These offline checks are always safe:

```bash
bash -n scripts/qa/build-cache-smoke.sh
bash -n scripts/qa/build-cache-benchmark.sh
bash scripts/qa/build-cache-smoke.sh --self-test
bash scripts/qa/build-cache-benchmark.sh --self-test
```

After every host gate is satisfied and only from the approved background unit,
an operator may create a new empty absolute evidence directory and run the
fixture verification:

```bash
bash scripts/qa/build-cache-smoke.sh --apply \
  --output-dir /tmp/lagrange-wp1-smoke-<new-run-id>
```

For a benchmark plan (no Docker build), supply exact A/B or B/C revisions and
one scenario:

```bash
bash scripts/qa/build-cache-benchmark.sh --plan --warm \
  --baseline-commit <40-hex-A-or-B> \
  --candidate-commit <40-hex-B-or-C> \
  --scenario rust-common-library \
  --output-dir /tmp/lagrange-wp1-benchmark-<new-run-id>
```

`--apply` uses the same explicit arguments only after the gate and authorization
checks above. It produces temporary benchmark images, not the official final
images or manifest. Record its tool SHA separately for B when it differs from
the B product SHA; record the C tool SHA as well if C changes the measurement
tool.

## Current verification matrix

| Item | State at WP-1 handoff | Required next evidence |
| --- | --- | --- |
| Scenario/telemetry and G1 journal contract | Code/self-test only | Retained offline self-test output/tool SHA; a separate current host gate before any actual run. |
| Warm fixture path after `3b0b88f...` | Not actually rerun | New gated smoke run with `Fresh`/binary evidence. |
| Source deletion wording and no stale image | Not actually observed | New gated smoke deletion/restore cases; do not broaden the matcher speculatively. |
| A/B/C full release elapsed time | Not measured | Clean C commit plus equal-condition official release runs. |
| C architecture selection | Not selected | Compare cache-only, common-builder, and role-builder candidates against this contract. |
| Strict V2 final manifest | Not measured by WP-1 benchmark | Official builder/checker evidence for every A/B/C release-sized run. |

## G1 offline review evidence and host status

Retained offline evidence was written at `2026-09-14T13:54:06Z` to
`/tmp/lagrange-wp1-g1-self-test.xW1GjVkjcG` (directory mode `0700`, files mode
`0600`). `SHA256SUMS` in that directory records the following immutable log
inputs: `recorded-at-utc.txt`
`9cae24872c04cca58513e0ea7968d69419d5571b5225505ba528e173873561c5`,
`bash-n-smoke.log` and `bash-n-benchmark.log`
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
`build-cache-smoke-self-test.log`
`5d925ef34d81d9a360a39a0ed197ffb3b9ebdae1d48a1d74acea810f30c0a7f3`,
`build-cache-benchmark-self-test.log`
`12ddc539d3c166299eb1bc2b5f9f8c96e758ddc6272aa871ebe475003947a962`, and
`journal-contract-static-check.log`
`c69000c469e2a9581cad6a0d92fa2c7d86d7bc2176e373f5f815cb1fab8f2b9f`, and
`git-diff-check.log`
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
The tested script SHA-256 values are
`scripts/qa/build-cache-benchmark.sh`
`f1314dc4f41200295822e67b35f896e5a22c33124095e3de19db2c0118e79749` and
`scripts/qa/build-cache-smoke.sh`
`d58156c9c328aa6504399794082d086ac1f0b65993b3117f3f6cadb0fa35feea`.
Those logs cover syntax, the existing smoke self-test, and the benchmark
self-test's readable quiet-window, nonzero query, exit-zero warning,
unreadable/empty probe, invalid JSON/boot, OOM, sampler, and scenario cases.
They are offline fakes only; no Docker daemon, Rust compiler, or actual smoke
`--apply` was invoked.

The last bounded host observation available to this work package was on
2026-09-14; its original command timestamp was not retained, so the
`2026-09-14T13:54:06Z` timestamp above is explicitly the offline-log record
time, **not** a new host observation. The earlier values were `MemAvailable:
1596356 KiB`, `SwapFree: 0 KiB`, a degraded systemd state, and
`lagrange-station-research-worker-1` restarting; those gates failed, and no
service recovery was attempted. Compiler processes were observed idle. The
former `kernel_journal=available recent_oom_matches=0` line came from the old
stderr-discarding, no-readability-probe query and is an **unverified historical
observation**, not evidence of OOM absence, a current host gate, or an actual
smoke pass.

No actual Docker/Rust evidence directory is claimed by this document. A future
authorized actual run must establish the strengthened journal gate anew under
the resource and health thresholds above.
