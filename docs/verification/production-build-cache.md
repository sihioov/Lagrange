# Production build-cache verification

## Result

Independent WP-4 review is **incomplete but resumable**. The local image-cache mechanism is
implemented and its static/fake-Docker checks pass at candidate
`3b0b88f6102d53a3c95ff669a4d62333b87b2927`. Actual Docker evidence from its parent proves only a
cold fixture build and an exact-input layer-cached repeat. It also exposed a harness defect that
the current candidate fixes statically. The corrected warm path, remaining semantic fixture cases,
the twelve production images and strict V2 manifest, and an equal-condition baseline/candidate
benchmark have **not** been run.

Further decomposition is not needed. Resumption needs a safe host, satisfied production health,
and an actual smoke rerun within the already authorized fixture scope before any production-sized measurement.

## Identity and scope

- Production-change baseline: `d1baf9da9b13fcb61649b1c26de56aed87a83418`.
- First integrated candidate, used by the actual fixture run:
  `008488c8ca5f735b06d0ebbdde9c424cfac31d00`.
- Current candidate: `3b0b88f6102d53a3c95ff669a4d62333b87b2927`.
- `3b0b88f...` has direct parent `008488c...`. At the start of this follow-up, `HEAD` was the full
  current candidate SHA and `git status --porcelain=v2 --branch` contained only branch metadata.
- The current-candidate patch changes only `scripts/qa/build-cache-smoke.sh` and
  `tests/fixtures/build-cache/Dockerfile` (37 insertions, 9 deletions). `git diff --check` passed.
- Implementation was reviewed read-only. This report was the reviewer's only repository write.
  The reviewer staged or committed nothing and performed no deployment, restart, prune, or provider call.

## Current-candidate patch review

The patch addresses the observed defect without weakening Cargo freshness assertions:

- The fixture declares `CACHE_FIXTURE_RUN_TOKEN` and consumes it in the compile step
  (`tests/fixtures/build-cache/Dockerfile:20-25`). A changed token therefore invalidates the
  compile `RUN` while leaving the three locked cache-mount IDs unchanged.
- `run_fixture_build` now distinguishes exact repeat, warm forced-RUN, and genuinely cold modes
  (`scripts/qa/build-cache-smoke.sh:485-506`). Only cold mode 2 adds `--no-cache`; warm mode 1
  changes the consumed run token; repeat mode 0 reuses the first cold build's token.
- The case routing uses cold mode 2 for `cache-absent` and `cache-repopulate-absent`, exact-repeat
  mode 0 for `repeat-same-source`, and warm mode 1 for `different-bin` and every semantic/recovery
  case (`scripts/qa/build-cache-smoke.sh:837-868`).
- The fake Docker verifies the token and `--no-cache` combination for every case class
  (`scripts/qa/build-cache-smoke.sh:923-956`). The self-test still requires `itoa` to be `Fresh`,
  not `Compiling`, in a forced warm run (`scripts/qa/build-cache-smoke.sh:345-380,1138-1139`).
- The missing-source matcher remains narrow at `scripts/qa/build-cache-smoke.sh:396-403`. Cargo
  1.97 wording such as `can't find bin` was not observed because the actual run stopped earlier;
  this case remains untested rather than being broadened speculatively.

Supporting semantics, not execution evidence: Docker documents
[`RUN --mount=type=cache`](https://docs.docker.com/reference/dockerfile/#run---mounttypecache),
[build-cache invalidation](https://docs.docker.com/build/cache/invalidation/), and
[`BUILDKIT_CACHE_MOUNT_NS`](https://docs.docker.com/reference/dockerfile/#buildkit-built-in-build-args).
The conclusion that `--no-cache` defeated mount reuse on this host comes from the retained actual
logs below, not from a general performance assumption.

## Production contract review

Static inspection and the production-image checker confirm:

- All seven Rust Dockerfiles have locked registry, git, and target mounts. Each target ID contains
  the pinned builder/toolchain identity, `${TARGETPLATFORM}`, and `release`, but no source commit.
  Each compile step performs exactly one `cargo clean --workspace --release --locked`, preserves
  the explicit binary order, and stages every output from `/cargo-target` into
  `/build/target/release` in that same `RUN`:
  `crates/api-server/Dockerfile:29-35`, `crates/job-queue/Dockerfile:24-32`,
  `crates/job-queue/Dockerfile.backtest-runner:38-44`,
  `crates/job-queue/Dockerfile.owner-beta-runner:30-36`,
  `crates/job-queue/Dockerfile.owner-equity-v2-runner:38-44`,
  `data-pipelines/collectors/Dockerfile:34-58`, and
  `deploy/runtime/Dockerfile.paper-runner:26`.
- The collector keeps its ten ordered binary builds and ten staged/runtime copies
  (`data-pipelines/collectors/Dockerfile:38-58,72-81`). Ordinary jobqueue and backtest runtime
  stages still copy `nt` directly (`crates/job-queue/Dockerfile:49` and
  `crates/job-queue/Dockerfile.backtest-runner:64`). The paper Cargo build remains on line 26, so
  the existing `runtime_deployment.puml` evidence citation remains valid.
- OCI revision validation and labels remain present in all runtime images; representative label
  locations are `crates/api-server/Dockerfile:43-47`, `crates/job-queue/Dockerfile:37-40`, and
  `deploy/runtime/Dockerfile.paper-runner:30-33`.
- The canonical V2 format and twelve-service list remain in
  `scripts/ops/lib/release-image-manifest.sh:8-22`. The builder validates the clean exact commit and
  manifest output path early, forces `COMPOSE_PARALLEL_LIMIT=1`, invokes one service per build,
  records per-service success/failure timing, inspects every image ID/revision, and publishes via a
  same-directory no-clobber link (`scripts/ops/build-production-images.sh:108-177,180-203,223-235,254-266`).
  A complete rerun is the documented resume path (`scripts/ops/README.md:167-189`).
- From baseline through the current candidate there is no change to `Cargo.toml`, `Cargo.lock`, the
  repository `.dockerignore`, Web or DB Dockerfiles, Compose topology, or either architecture
  diagram. No diagram update is warranted because the cache change adds no structural edge.

These are static/fake results. They do not prove that twelve real images exist, that their actual
binary sets/revision labels match, or that a strict V2 manifest can be published on this host.

## Executed verification and evidence

All retained artifacts are under `/tmp/lagrange-cache-wp4-static-V465ygKV/`. They contain no
credentials or provider payloads.

### Current candidate (`3b0b88f...`)

| Command | Result | Evidence |
| --- | --- | --- |
| `git diff --check 008488c...3b0b88f...` | PASS | terminal review; only the two expected files changed |
| `bash scripts/qa/build-cache-smoke.sh --self-test` | PASS; fake Docker only | `08-3b0b88f-build-cache-smoke-self-test.log`, SHA-256 `5d925ef34d81d9a360a39a0ed197ffb3b9ebdae1d48a1d74acea810f30c0a7f3` |
| `bash scripts/ops/build-production-images-static-check.sh` | PASS | `09-3b0b88f-build-production-images-static-check.log`, SHA-256 `edeeeb2ea55e359e8fda654cec9690f306beac15019d6be19eeae69ed398bed9` |

No Docker daemon, container, or Rust compiler was used in this follow-up. In particular, these
results are not an actual smoke pass for `3b0b88f...`.

### Parent candidate (`008488c...`)

The earlier independent focused checks all passed:

1. `bash scripts/ops/build-production-images-static-check.sh`
2. `bash scripts/ops/build-production-images-self-test.sh`
3. `bash scripts/ops/production-ops-static-check.sh`
4. `bash scripts/ops/production-ops-self-test.sh`
5. `bash scripts/qa/build-cache-smoke.sh --self-test`
6. `bash scripts/qa/build-cache-benchmark.sh --self-test`

Their exact outputs are `01-...log` through `06-...log`. `production-ops-self-test.sh` passed.
The separate, broader `scripts/ops/self-test.sh` previously encountered a fakeroot numeric-`chown`
error and was not rerun in this review; that limitation remains unresolved. The checkout
reported many local files as mode `0775` while Git records executable files as `100755`; this was
an existing host/worktree mode presentation and no mode was changed or test weakened.

Independent plan/path checks also passed or failed closed as intended. The smoke and benchmark
plans accepted canonical absolute `/tmp` output paths. Relative paths, paths inside the repository,
and a symlink-traversing smoke path were rejected; the benchmark additionally rejected identical
revisions and a missing baseline object. Outputs are retained in `validation/`.

### Actual fixture run, parent only

Executed once with native Docker permission:

```text
bash scripts/qa/build-cache-smoke.sh --apply \
  --output-dir /tmp/lagrange-cache-wp4-static-V465ygKV/actual-smoke
```

The launch log is `07-build-cache-smoke-apply-launch.log`; detailed results are in
`actual-smoke/cases.tsv`, `actual-smoke/metadata.tsv`, and `actual-smoke/logs/`.

| Scenario | Actual result at `008488c...` | Evidence |
| --- | --- | --- |
| Cold/empty cache | PASS | 4514 ms total, 1300 ms Cargo; compiled fixture app, workspace library, and `itoa`; both binary outputs matched exactly |
| Same source repeated | PASS | 585 ms total, compile vertex `CACHED`, 0 ms Cargo; both binary outputs matched the cold build |
| Forced warm `different-bin` | FAIL, harness defect | 2150 ms total, 1200 ms Cargo; `cargo clean` removed 0 files and Cargo downloaded/compiled `itoa` again, contradicting the required warm `Fresh itoa` evidence |

The failed parent command passed `--no-cache` to the forced-warm case. The harness stopped before
executing that case's binaries and before every later scenario. The current candidate removes
`--no-cache` from forced-warm cases and uses the consumed run token, but that correction has not
received an actual Docker rerun.

The actual-run metadata recorded Docker client 29.7.2, Git 2.53.0, and fixture Rust toolchain
1.97.1. The adjacent WP-4 tool check separately recorded Cargo/rustc 1.97.1, Buildx 0.36.1, and
Bash 5.3.9. The Docker server version was not recorded. The fixture used
the isolated image prefix `lagrange-build-cache-smoke-r354a76467a50-`, cache namespace
`build-cache-fixture-smoke-r354a76467a50`, and a separate repopulation namespace. Binary containers
used `--network none`, read-only root filesystems, dropped capabilities, and no-new-privileges.
`actual-smoke/cleanup.tsv` records removal of only the three owned fixture tags. No production tag,
production cache namespace, service lifecycle command, or global prune was used.

## Acceptance matrix

| Acceptance item | Status | Basis / resumption requirement |
| --- | --- | --- |
| Candidate lineage and initial cleanliness | PASS | Full SHA and direct parent verified before report write |
| Seven production Rust Dockerfile cache/staging contract | STATIC PASS | Code inspection plus production-image static checker |
| Twelve-service list, binary order, revision and strict V2 contracts | STATIC/FAKE PASS | Static checker and prior self-tests; no real twelve-image manifest |
| Runtime NT copies and paper line-26 diagram evidence | STATIC PASS | Lines cited above; no topology change |
| Cold/empty fixture | ACTUAL PASS AT PARENT ONLY | Binary and Cargo evidence from `008488c...` |
| Exact-input cached-layer repeat | ACTUAL PASS AT PARENT ONLY | Compile vertex cached and binary outputs exact at `008488c...` |
| Current warm invalidation fix | STATIC/FAKE PASS | Two-file review and current self-test; actual `3b0b88f...` rerun required |
| External versus workspace dependency reuse | NOT VERIFIED | Parent forced-warm failed before proof; rerun must show external `itoa` Fresh and workspace rebuild/reuse phases |
| Rust source, workspace library, build script, dependency/lock, embedded data, and build-setting changes | NOT VERIFIED | Parent smoke stopped before these cases |
| Commit-only change and old-mtime/branch restoration | NOT VERIFIED | Parent smoke stopped before these cases |
| Deletion failure, no stale result, restore and repopulation | NOT VERIFIED | Parent smoke stopped; Cargo 1.97 error matcher remains untested |
| Full twelve production images and actual strict V2 manifest | NOT VERIFIED | Not authorized and host gates fail |
| Equal-condition baseline/candidate benchmark | NOT VERIFIED | No controlled run; plan/self-test only |
| Performance improvement | NOT ESTABLISHED | The historical `f164d28` approximately 98-minute total / 96m33s Cargo figure is reference-only, not a controlled before/after measurement |

## Operational prerequisites and limitations

Before the parent fixture run, approximately 1.7 GiB RAM was available, swap had only about 240 KiB
free, and no active Cargo/Rust compiler was observed. After it stopped, the reviewer observed
`MemAvailable: 1854608 kB` and `SwapFree: 152 kB`; a later coordinator observation still reported
about 1.7 GiB available and zero usable swap. These values fail the benchmark's minimum 2 GiB RAM
and 512 MiB swap gates recorded by that historical run.

WP-20's bounded resource-policy follow-up records a separate B/C warm run that stopped after
299 seconds. Its final sample was `MemAvailable: 8722008 kB`, `SwapFree: 499484 kB`;
the bounded kernel OOM query found no matching event. That supervisor did not record PSI.
An offline regression combines those retained memory/swap values with explicit low-PSI fixture
input and passes the current shared policy in `scripts/ops/lib/build-resource-policy.py`.
This is not retrospective proof of the run's PSI: live acceptance requires a fresh observation.
`MemAvailable` must be at least 2097152 KiB and full PSI `avg10`
must remain below 5.0%; `SwapFree` is telemetry/advisory only. The 5% value is a conservative
10-second moving-average workflow threshold, not a kernel-prescribed or universally proven limit.

The bounded post-run Docker status read showed ten production containers healthy and
`research-worker` restarting. A bounded inspection returned
`restarting|running=true|oom=false|exit=2|restart=980|health=unhealthy`; the coordinator later saw
restart count 982 at about 07:26 UTC. No pre-run container-health baseline exists, the fixture used
an unrelated synthetic workspace, and causality cannot be assigned. The unhealthy production
service is nevertheless a mandatory stop condition. It was not restarted or remediated.

The attempted OOM check was:

```text
journalctl --quiet --kernel --no-pager --since '30 minutes ago' -n 1000 2>/dev/null |
  rg -i -c 'out of memory|oom-kill|killed process' || true
```

It produced no count. Because journal errors were suppressed and the pipeline result was masked,
this evidence cannot distinguish no matching OOM event from unavailable journal access. Absence of
recent OOM is therefore **not established**.

Do not start another build until production health is satisfied, the shared MemAvailable/PSI gate
passes, journal OOM access is established, and no prior compiler remains active. SwapFree must still
be observed and recorded, but does not independently block resumption. A production-sized run additionally
requires explicit scope resolution, background low-priority systemd execution, batches of at most
three with exactly one service per Compose call, `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`,
and resource/OOM/health checks between batches.
