# WP-3 build-layout probe

Status: **Offline verification PASS: the final complete `--self-test` returned 0,
including F1–F9 regressions, all 84 fake controller cases and post-matrix publication
checks. Actual results = NOT_RUN; G2 = NOT_PASSED.**

The earlier full invocation passed 84 controller cases and then exited 1 in
concurrent publication; that failure remains retained and is not an overall PASS.
After the publication fix and the F9 Compose-prefix correction, the authorized
final full invocation passed on the exact final controller/helper/fixture hashes
below. Earlier Sol reports and the historical host snapshot are not current
passing evidence.

The frozen experiment and product design are unchanged. This is an offline fixture
prototype, not a choice of product C, a performance measurement, or release acceptance.
Workers performed no actual Docker/Cargo/rustc, host-health/service, or market-provider
operation and did not stage or commit files. The coordinator separately made the
bounded read-only host observation recorded below and accepted the offline files
for a checkpoint commit.

## Corrective findings and observations

| Finding | Implemented behavior and verified offline result |
|---|---|
| F1 | A fresh 128-bit random hexadecimal run ID is stored in initial `run.json`, restored on resume, and checked against saved namespace/tag prefixes. Equal basenames under two parents produced distinct IDs, tags, and caches; resume retained each ID. The full matrix had 84 distinct run IDs. |
| F2 | Actual new/resume invocations acquire one descriptor lock at `/tmp/lagrange-build-layout-execution` before preflight/validation/builds. The owner-only directory inode is never unlinked; process exit releases the lock. Tests used private `/tmp` paths only and proved contention before the fake host boundary, normal/TERM release, and rejection of symlink/unsafe-mode paths. Plan/self-test did not acquire the actual host slot. |
| F3 | Baseline preserves the outer Compose status and requires the exact injected marker, the matching compile RUN's exit 73, and successful Cargo evidence. The executable fake now returns client status 1 with inner status 73. Common/grouped require buildx status 0, an exact INCOMPLETE tree, validated Cargo records, and host-helper status 73. Both boundaries record status/cause evidence; unrelated daemon/build failures are rejected. All three export/resume paths passed. |
| F4 | A VERIFIED EXPECTED_CAUSE event retains the original command status before the immediate post-failure gate. Only the gate receives previous-exit 0, as specified by §5.7. Gate failure returns 1 without a resumable EXPECTED_STOP. Real gate/parser code with synthetic executables passed both gate-pass and memory-fail tests. The matrix audit verified all 27 expected stops had the immediate passing gate and retained status; pre/post-resume gates remained present. |
| F5 | Unspecified build-script compile freshness and execution counts are observations. Raw JSON, profile/source/features, verbose evidence, run count and line hashes remain mandatory. Six combinations of fresh true/false and 0/1/2 executions passed; missing/malformed evidence and changes to each required app/lib/itoa H were rejected. A separate 4,032-combination comparison preserved every prior app/lib/itoa H value. |
| F6 | Image inspection uses one literal backslash in the Go-template TAB escape. The executable fake checks the exact argv/template and rejects the former double-backslash template. The existing helper gate template remains unchanged. |
| F7 | Image absence requires status 1 plus the exact missing-image diagnostic and an empty/empty-array stdout. Image presence, unknown statuses, and daemon/permission errors fail closed. Status, typed cause and stdout/stderr hashes are retained. |
| F8 | Complete stable/prerelease Cargo identities, including parenthesized hash/date, are preserved and validated. Realistic fake P0 plus five valid and twelve invalid parser inputs passed; multiline/malformed identities are rejected. No real Cargo command was invoked. |
| F9 | Baseline accepts unprefixed BuildKit or the exact optional Compose service prefix derived from `service_for` for the phase. The `builder` stage, matching vertex, injected marker, inner exit 73, original outer status and successful Cargo proof remain required. Fake Compose emits `[probe-b-base builder 5/5]` for the injection. Both valid forms returned 0; ten invalid variants returned 1, including wrong service/stage/header vertex/error vertex, marker/bin, inner/client status, failed Cargo and unrelated daemon failure. Full export-failure/resume controllers passed for all three layouts. |

The previously reviewed wrapper status propagation, native identity checks, exact
stdout/runtime comparisons, raw Cargo profile/version/source/default-feature
checks, immutable initial metadata, resume restoration, first-trial-only injection,
retained stop-after-P2 image, and exact-repeat re-submission remain in place.
Baseline still cleans the workspace for every executed one-bin compile RUN.

Two further bounded defects were exposed during verification:

- The existing fake Compose parser rejected the second `-f` used by a real override,
  and its source route incorrectly modeled a guard ledger. Baseline/source recipes
  create no ledger; the real helper reports `rebuild=all reason=missing`. The fake
  now accepts the override and models that missing ledger. The initially failing
  route case then passed without changing H.
- Concurrent publication allowed both workers into the publish section in the
  retained offline reproduction; one failed removing an already removed directory
  lock. Publication now uses a checked regular-file descriptor lock with a bounded
  wait and process-exit release, retaining full-tree digest names, no-clobber
  comparison and distinct evidence trees. Three concurrent reproductions returned
  `[0,0]`; the post-matrix publication suite and fresh common exact-repeat/grouped
  export-resume controller checks passed on the final helper.

## F9 follow-up and final full verification

Evidence root: `/tmp/lagrange-wp3-f9-u33u4krv`. The coordinator's retained prefix reproduction is
`/tmp/lagrange-layout-compose-prefix-review.u1zpb8d9`; it was inspected offline.
The F9 change is confined to the baseline validator, fake injection header and
focused regression coverage. The publication helper is unchanged in this follow-up.

Each command below ran from the repository through
`python3 /tmp/lagrange-wp3-f9-u33u4krv/run-check.py <label> <command>`, which retains the exact argv,
actual exit, output hash and source/helper/fixture hashes before and after execution.
Its PATH prepends `/tmp/lagrange-wp3-f9-u33u4krv/sentinels` and DOCKER_BIN points to that directory's
Docker sentinel. All 15 forbidden-executable sentinels remained uncalled; the
self-test's explicit synthetic executables supplied every build/host boundary.

| Label / exact command | Verified result |
|---|---|
| `focused` / `bash /tmp/lagrange-wp3-f9-u33u4krv/focused.sh` | exit 0; actual validator: 2 valid forms accepted, 10 invalid forms rejected; baseline/common/grouped export-failure and resume controllers PASS. |
| `post-matrix` / `bash /tmp/lagrange-wp3-f9-u33u4krv/post-matrix.sh` | exit 0; publication contention, immutable evidence, Cargo negatives and remaining post-matrix checks PASS. |
| `syntax-probe` / `bash -n scripts/qa/build-layout-probe.sh` | exit 0. |
| `syntax-helper` / `bash -n tests/fixtures/build-layout/layout-helper.sh` | exit 0. |
| `self-test-final` / `bash scripts/qa/build-layout-probe.sh --self-test` | **exit 0; complete self-test PASS**, including all 84 ordered controller cases and the final concurrent-publication checks. |
| `evidence-audit` / `python3 /tmp/lagrange-wp3-f9-u33u4krv/audit.py` | exit 0; exact-source identities, 84 distinct IDs, 27 immediate expected-stop gates with preserved statuses, resume gates, focused/full F9 statuses and zero sentinel calls verified. |

The final full invocation used controller
`c2601276bfe76df7115432209dbccc125ebfe69e6cd3a001e9506a0c8717843a`, helper
`b2dfd05e8c7dcf407132afceafcf5607a1ce8df786a5815ff9279d8909794027` and fixture tree
`3695b672cbbbd79bb1f10f2e8f1ceafe251afc06d15c3222b17e49893d77aacd`. Every individual fixture file hash and the
unchanged before/after identities are retained in `self-test-final.json`.
Raw full evidence is `/tmp/build-layout-self-test.HZiKwO`:
43,732 regular files are hashed by
`full-evidence.SHA256SUMS`; 2 deliberate negative-test symlinks are
recorded separately in `full-evidence-links.json` without following them.

| Evidence file under the F9 root | SHA-256 |
|---|---|
| `focused.log` | `70e493fbc3b85e2f981e201aa40f72f3bd8b90ca58497aa9cdc0258788125912` |
| `post-matrix.log` | `b75ba347026e1b9a0828a78765b1e20e09743bfade487224a0ac59cdf2b9fbbe` |
| `self-test-final.log` | `9b0572991b162cf03a392fec1814250b0512a5db65e97ad12a36ab72733d3e7d` |
| `self-test-final.json` | `5bf9c5c84fb0f6d387045d4ee7d38c05850033142c4c7df23a4cdc0f7411a393` |
| `audit.json` | `d823fa006143b8d2c60598f82ce9d54382be6a095f0dba46e734df47ca81156c` |
| `full-evidence.SHA256SUMS` | `1e0cd7884846322e53c840be2778f0ee76095f57c3923e6409c7dc764db6ee6b` |

## Earlier corrective-run evidence

Evidence root: `/tmp/lagrange-wp3-astra-zsoqlmgw`. Temporary drivers load the actual controller definitions;
no permanent selector, mode, assertion bypass or new dependency was added.

| Command | Observed result |
|---|---|
| `bash /tmp/lagrange-wp3-astra-zsoqlmgw/targeted.sh` | exit 0, F1–F8 targeted PASS; initial evidence `/tmp/lagrange-wp3-targeted.BXb9xU`. The full invocation also reran these regressions before its matrix. |
| `bash /tmp/lagrange-wp3-astra-zsoqlmgw/h-preservation.sh` | exit 0, 4,032 unchanged app/lib/itoa H comparisons. |
| `bash /tmp/lagrange-wp3-astra-zsoqlmgw/route.sh` | first exit 1 (retained fake mismatch); after the bounded fake fix, exit 0. |
| `PATH=/tmp/lagrange-wp3-astra-zsoqlmgw/sentinels:$PATH bash scripts/qa/build-layout-probe.sh --self-test` | **exit 1**, after **84/84 controller PASS**, at post-matrix publication contention. This is not a full self-test PASS. |
| `python3 /tmp/lagrange-wp3-astra-zsoqlmgw/publication_fixed.py` | exit 0; three paired publication trials returned `[0,0]`. Prior failures remain under `publication-review/`. |
| `PATH=/tmp/lagrange-wp3-astra-zsoqlmgw/sentinels:$PATH bash /tmp/lagrange-wp3-astra-zsoqlmgw/post-matrix.sh` | exit 0; all post-matrix checks PASS on final helper, retained under `/tmp/lagrange-wp3-post-matrix.SGRIIB`. A temporary-driver local/function error in `post-matrix-01.log` was corrected before this run. |
| `PATH=/tmp/lagrange-wp3-astra-zsoqlmgw/sentinels:$PATH bash /tmp/lagrange-wp3-astra-zsoqlmgw/publication-controller.sh` | exit 0; fresh common/exact-repeat and grouped/export-fail-p2 including resume PASS, `/tmp/lagrange-wp3-targeted.X22csy`. |
| `python3 /tmp/lagrange-wp3-astra-zsoqlmgw/static_checks_final.py` | exit 0; captures individual exit 0 for both `bash -n` commands and new/resume plans. No plan writes or forbidden calls. Exact argv/status/log hashes are in `static-checks-final.json`. |
| `python3 /tmp/lagrange-wp3-astra-zsoqlmgw/audit.py` | exit 0; 84 ordered complete controller runs, 84 distinct IDs, 27 verified expected-stop gates, zero forbidden-command sentinel calls. |

`cargo-malformed/results.json` retains eight exact `cargo-assert` argv/status
records for correctly rejected missing/malformed script evidence. The complete
matrix evidence is `/tmp/build-layout-self-test.veYBKM`; its 43,173 regular-file
hashes are in `full-evidence.SHA256SUMS` (manifest SHA-256
`138a8b73ca165be449754cf78444e64102980d1810cacc3abb9a579cf0fdcad1`).

| Evidence file in the root above | SHA-256 |
|---|---|
| `targeted-01.log` | `1ab22c1da5e62158536f989492244861d374df45b111795f41f6a57c0c3ba02e` |
| `h-preservation.log` | `97bb407722e906708fda6ba738be46fac17d1e992de7a112dd9df2b8659428ce` |
| `route-01.log` | `cafa397b01fd728102ca7768a7bb25e4f7d87d91dde4cc0291090468811d7901` |
| `route-02.log` | `603a9a002a5ff1e203cb95609cd29a282eb021897d9c02d7a4156ab305802174` |
| `self-test-full-01.log` | `d6d269d49465cbe0c44eebc7d01e4550f46b14c08ba41d82a5bcabd3cea98869` |
| `post-matrix-02.log` | `f918b2291b345dcded41df9bcfdf9b491e7b46b0f3d94e949d2dc08827f03433` |
| `publication-controller.log` | `b9313e31461499687210a1fae25d750990574bffe3b61a796b1c524e70004efe` |
| `audit.json` | `8ca93ddc19846dca451be8599825df6fab259fc479e83d1111cd0fa4ddda0ba5` |

The earlier, overall-exit-1 matrix used controller `719ccc28802983d7adba1eea2fc2173166f6dc7f5eec5d66b86d494abfe7e20b` and helper
`41f41cd091e42edb5f30e19246c3ff43624fa888628f65fe9d8a1426ab2e8ea4`.
The final helper is `b2dfd05e8c7dcf407132afceafcf5607a1ce8df786a5815ff9279d8909794027`;
its publication correction was verified by the bounded checks above. These are
separate tool identities; the earlier full run is not relabeled as a run of the
final helper. The final baseline Dockerfile SHA-256 is
`146f753514b928448cc38018c6fca2f0f44f9c8bb4639416603577594c08a1e3`.

## Scope, fixed inputs, and remaining work

This F9 follow-up changes `scripts/qa/build-layout-probe.sh` at lines 386–395
(service-aware validator), 1449–1450 (fake injection header), 1814–1854
(focused protocol regressions), and this report. Exact changed line spans and
diffs are retained under the F9 evidence root. No helper or other fixture file
changed in this follow-up.

The preceding takeover changed the controller, helper, baseline Dockerfile line 96
and report; its exact spans/source hashes remain in
`/tmp/lagrange-wp3-astra-zsoqlmgw/final-code-changes.json`. Product files remain
unchanged. Neither worker staged or committed files.

Frozen inputs remain:

- HEAD: `95bce51723100f22b2f8bd4ce38b4489295b3a3a`.
- Design SHA-256: `414616b3a40e25bb56fb9732e2c096cc242448ae8df8520b09ab1a7c097a2c2b`.
- Baseline document SHA-256: `5ef1025b023f2f64449b27b01567ea9f4b8eadd89cd1da0327f202e944beae45`.
- Original build-cache fixture Git tree: `fbe9999abfc3873e1e07ca0d733b00c666cb2a9d`.
- Benchmark/smoke tool hashes remain the supplied G1 values; neither tool changed.

Scope/design/CLI/H/gate deviations: **none**. The final full self-test repeat was
explicitly authorized because the earlier invocation failed and the publication
helper had changed; it was run once after the focused checks passed.
Frozen-contract contradictions: **none**. Remaining known implementation defects
in F1–F9 and the bounded publication/route paths: **none**. Further decomposition
required: **none**.

Coordinator diff/evidence/CLI review is complete as recorded below. Actual
Cargo/cache/feature-reuse, Docker/BuildKit/export/route/image, timing/performance
and product A/B/C experiment results remain **NOT_RUN**. The bounded host
observation is not a complete resource/health gate or an OOM-absence proof.
G2 remains **NOT_PASSED**; this does not complete 12-image/product C/performance
acceptance. Other unresolved/follow-up/unverified items: **none**.

## Coordinator acceptance and actual-execution blocker

The final Paseo worker returned `status: idle`. The coordinator inspected the
implementation and independently verified the final evidence and public CLI:

| Check | Result and retained evidence |
|---|---|
| Final evidence versus checked-out source | PASS: exact invocation exit 0, unchanged source hashes, all 84 ordered cases, 84 unique run IDs, 27 expected-stop gates retaining original statuses, final log hash and zero forbidden sentinel calls. `/tmp/lagrange-layout-root-final-evidence.mkd8tujd/result.json`. |
| Public CLI | PASS: explicit/default plan modes are read-only; missing or duplicate health inputs are rejected before output or external calls. `/tmp/lagrange-layout-root-cli.jTaOoUAf/result.txt`. The coordinator ran the narrow CLI audit, not another full self-test. |
| Integration scope | PASS: the four frozen G1 file hashes and HEAD above matched; only the 20 new WP-3 controller, fixture and report files were present. `python3 /tmp/lagrange-layout-root-scope-check.py` returned 0. |

The controller/helper/fixture hashes accepted by the coordinator are the exact
final hashes above. The subsequent coordinator edit changes only this report.
Offline WP-3 implementation and verification are accepted for a clean checkpoint;
the actual experiment and G2 selection are still incomplete.

The last bounded host observation was at **2026-09-15 02:58:39 KST**
(`2026-09-14 17:58:39 UTC`), retained in
`/tmp/lagrange-layout-host-observation-20260914T175839Z.json`:

- MemAvailable: **1,112,312 KiB**, below the required **2,097,152 KiB**.
- SwapFree: **236 KiB**, below the required **524,288 KiB**.
- `lagrange-station-research-worker-1`: running and restarting, health
  **unhealthy**, restart count **1613**, selected `OOMKilled` field false.
- No Cargo/rustc/rustdoc process was observed. Kernel journal and systemd units
  were not checked; this observation cannot establish that no OOM occurred.

These observed failures prevent actual fixture execution. No actual build,
service repair/restart, swap change, cache prune, provider call or deployment was
performed. Resume requires the existing full resource/health gate to pass;
long production image measurements additionally require a valid image-only
execution scope for the exact candidate. The expired historical exception is
not reused. Product C has not been selected or implemented, and the 12-image,
strict V2 manifest and A/B/C performance acceptance remain unverified.

## 2026-09-15 resume-gate implementation and verification

This section records the bounded continuation from
`ff4a67dd0c113955ac1bca98db7d9825fa4fef20` under the accepted
[resume specification](../superpowers/plans/2026-09-15-production-build-resume.md).
The preceding F9 results are retained historical evidence, not proof for these
changed sources. **Final offline verification: PASS; actual execution: NOT_RUN.**

### Implementation

- `BUILD_LAYOUT_RESEARCH_EXCEPTION` is the sole opt-in. Its JSON must have exactly
  the specified keys and actual types, the fixed image-only scope/name/incident,
  and a currently valid observation/expiry interval of at most 24 hours.
  Absolute canonical paths are traversed using directory descriptors and
  `O_NOFOLLOW`; the regular file must belong to the executing UID with mode 0600.
- Immutable `run.json` binds the original path, fields, SHA-256 and exact bytes
  encoded as base64. The private `research-exception.json` snapshot contains the
  original bytes with mode 0600. Every real gate revalidates the input and snapshot
  before host commands and before recording PASS. Resume also checks prior gate
  evidence against the first/latest saved observations and journal origin.
  Missing, altered, substituted, renewed or expired inputs fail closed; a strict
  run cannot acquire an exception on resume. Plans remain read-only without
  requiring health inputs, and validate any supplied/bound exception.
- Only the exact pinned research container uses the additional fixed `.Image`
  and `.State.ExitCode` fields. Other containers retain the original seven-field
  template and strict parser. Research must be running without OOM, with the
  pinned container/image IDs and project; only the specified health/restart/exit
  combinations pass. Each parsed count and its observation time/limit are retained,
  including counts in rejected observations. Counts cannot decrease or exceed
  `initial_restart_count + ceil(elapsed_seconds / 30) + 2`.
- First container/unit identities and journal-since remain unchanged. Only the
  exceptional worker's latest observation advances. Its passing gate reason is
  `image-build-only-known-incident`, with the exception fields/hash in evidence;
  strict healthy gates still use `healthy`. This is not service recovery or a
  release-ready health result. Other services, RAM/swap thresholds, compiler scan,
  kernel proof/range scan, execution-slot lock and failure statuses remain enforced.

Changed line spans in the final sources:

| Owned file | Lines |
|---|---|
| `scripts/qa/build-layout-probe.sh` | 47–48, 290–293, 1011–1024, 1144, 1199–1218, 1260, 1558, 1935–2160, 2229: binding, immutable metadata/resume, and focused self-test integration. |
| `tests/fixtures/build-layout/layout-helper.sh` | 648–772, 790–801, 907–980, 1017, 1105–1149: private input validation, monitored observations/state, and gate integration. |
| This report | 210 onward: this separate resume-gate evidence section only. |

### Commands and retained evidence

Evidence root: `/tmp/lagrange-wp3-research-dl1yFnLV` (private directory).
Commands ran through `python3 <evidence-root>/run-check.py <label> <command>`.
Each label's JSON retains actual argv/exit, controller/helper/fixture file hashes
before and after, the raw log hash and counts for 15 forbidden-command sentinels.
All sentinel counts are zero, including Docker, Cargo, rustc/rustdoc,
systemctl/journalctl and provider/network executable boundaries.

| Label / command | Actual result |
|---|---|
| `focused-01` / `python3 <evidence-root>/focused-loader.py` | exit 1; a quoted test label caused output-path rejection before the intended type check. Failed evidence retained; the test label was corrected. |
| `focused-02` / `python3 <evidence-root>/focused-loader.py 02` | exit 0; 124 focused checks, including a complete synthetic exception-bound P2 stop/resume controller. |
| `syntax-probe-final` / `bash -n scripts/qa/build-layout-probe.sh` | exit 0. |
| `syntax-helper-final` / `bash -n tests/fixtures/build-layout/layout-helper.sh` | exit 0. |
| `self-test-final` / `bash scripts/qa/build-layout-probe.sh --self-test` | **exit 1**, retained. The evidence wrapper's inherited umask 077 made F2's requested 0755 test directory become 0700; the existing F2 assertion failed before the matrix. This is not a full PASS. |
| `f2-recheck` / `bash <evidence-root>/f2-recheck.sh` | exit 0 after setting only the private wrapper's child umask to 022. Controller/helper code and F2 assertions were unchanged. |
| `self-test-final-02` / `bash scripts/qa/build-layout-probe.sh --self-test` | **exit 0**, complete PASS: 124 focused checks, F1–F9, exactly 84 controller cases in their original order, and the post-matrix publication checks. |
| `evidence-audit` / `python3 <evidence-root>/audit.py` | exit 0: unchanged final source hashes, 84 distinct IDs, all 27 expected stops with immediate passing gates and preserved original exits, 28 exception-assisted controller gates, and zero sentinels. Original corrected controller/assertion and strict-parser function bodies were also compared with the base commit. |

Focused checks use actual parser/controller functions with private synthetic
executables. They cover schema/types/duplicates/unknown keys, unsafe paths/modes,
missing/changed/substituted resume grants and snapshots, immutable initial/latest
state, identity/health/exit/OOM/restart bounds, other-service failures, RAM/swap,
kernel failures, active compiler rejection and read-only plans. The owner check
uses a synthetic executing UID against the unchanged parser body; no real chown
or UID change is used for that test. Expiry during observation is
tested with the actual clock and a delayed synthetic inspect, without a time seam.

Final source identities (identical before/after the successful full command):

- Controller SHA-256: `f3d7072f7ada68fbf8586e78505e273ca4ed1af769a954298a5fd491e91922da`.
- Helper SHA-256: `d89b8e8d8647706c5639426688a364c8bfc7de4c61881f724fb47788950ed666`.
- Fixture input tree SHA-256: `66a1949d34a5265634d7a66c5eeb647c7e11627f0dd2a813dcefa81e653b84ca`.
- Full log SHA-256: `b42c020be52b667d648df6b85790d7b1702be4e3c639e94be580fc97ee7530a5`.
- Full invocation JSON SHA-256: `ef9c995d2ac27f0c1fdb086911812b97eb7b0a90f402b8c37dd5b3d992a21add`.
- Audit JSON SHA-256: `7b5dca70c3d3676460f553675824086455160ad1c474ce27ac1615d0aede2f5e`.

Raw final evidence: `<evidence-root>/tmp/build-layout-self-test.nUjbcd`.
`full-evidence.SHA256SUMS` hashes 44,846 regular files; eight deliberate symlinks
are recorded separately without following them. The manifest SHA-256 is
`dfed4e12d15b7a52c82552c3af5c0a9dad3098f6892ff06e27e7dacd04388bf2`.

Scope/specification deviations: **none**. The two failed invocations and the
focused reproduction justify the corrected reruns above. Implementation defects,
unresolved implementation items and further decomposition required: **none**.
The coordinator-owned design/resume files, other QA tools, all Dockerfiles,
dependencies, release validator and diagrams were not edited by this worker.
No stage/commit, service/session operation, global prune or provider operation was
performed. Actual Docker/Cargo builds, host health, cache/route/runtime behavior,
release/performance acceptance and G2 remain **NOT_RUN / NOT_PASSED** for this package.

## 2026-09-15 fixture snapshot COPY correction

This bounded correction starts from clean HEAD
`257d199fc7e77bf7c633033218486f4bc0bed98e`. **Corrected offline checks: PASS;
corrected real layout: NOT_RETESTED.** The coordinator retains the actual build
slot and must rerun the real layout from a new clean commit.

### Retained failure and diagnosis

The following actual-run facts and evidence paths were supplied by the
coordinator; this correction did not rerun or modify those records:

- Corrected cache smoke passed all 15 scenarios at the starting HEAD, with
  pre/post gates PASS. Receipt:
  `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/smoke-02.json`;
  run tree: `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/runs/smoke-02`.
- Actual layout `all/all` then failed its first baseline cold P1 before Cargo at
  `guard_inputs`, with a compile input hash mismatch. The original receipt records
  unchanged source and actual exit 1:
  `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/layout-02.json`.
  Exact log:
  `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/runs/layout-02/baseline/cases/cold/attempt-001/logs/p1.log`.
  No test or gate waiver was used.
- Two coordinator scratch-only local exports isolated the COPY behavior:
  `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/copy-mode-diagnostic/result.json`.
  Individual package COPY instructions created destination package roots with
  mode `0755` from source roots with mode `0775`, changing the mode-sensitive
  compile hash from
  `df295e7d5eb51e7ea03d649fe13db940203c2825d7b1b364a5b0b358b8753dc5`
  to `b44f4e6ad17731290a0364511f6ffbdbddb4c60fc4d1a32f90479beb044d4159`.
  Copying the whole validated synthetic fixture snapshot preserved its child
  directory modes and reproduced the expected hash in that diagnostic.

### Exact correction and source identities

Each source stage now uses one `COPY . ./` at the original location under the
inherited `WORKDIR /build`, with a two-line comment explaining the package-root
metadata requirement. The byte comparison against HEAD permits exactly this
replacement of the four consecutive COPY lines. Every other stage, ARG/ENV,
native-identity operation, command order, cache mount, guard, check, artifact,
runtime COPY and failure injection is unchanged. In particular, the entire
`verified-artifacts` stage and its separate
`COPY layout-helper.sh /tmp/layout-helper.sh` are unchanged. File modes are
unchanged; mode hashing and guard assertions remain enforced.

| Changed fixture file | Corrected lines | SHA-256 before | SHA-256 after |
|---|---|---|---|
| `tests/fixtures/build-layout/Dockerfile.baseline` | 49–51 | `146f753514b928448cc38018c6fca2f0f44f9c8bb4639416603577594c08a1e3` | `36ac4a5458e20ff022848008e63f2380410cac417434abbb8ec9d40c12d0d6ae` |
| `tests/fixtures/build-layout/Dockerfile.artifacts` | 50–52 | `5109cd156a3ee6672909065d27050225ad6649c93dbdeefdb58b2acc393a4e57` | `edc45bfd7063e71a9310cc3bf7fcdef99c2fe2b89f393f945df0b18b843c9a7a` |
| `tests/fixtures/build-layout/Dockerfile.consumer` | 48–50 | `d0e0442f632042010ffacf9eab1b4c4bef62015613d880333aeebc1592607eab` | `7196764401fccf834f17b0b97554f4c0fc583f313d60017d1f9ef8d238983ed4` |

The controller remains
`f3d7072f7ada68fbf8586e78505e273ca4ed1af769a954298a5fd491e91922da`;
the helper remains
`d89b8e8d8647706c5639426688a364c8bfc7de4c61881f724fb47788950ed666`.
All report content through the preceding section is preserved byte-for-byte.
The original report SHA-256 is
`c8a44af8406e56160ab2fe49bce3d6c2db899d2f90336f4613156816f02cc6e9`;
its final appended-file hash is recorded in `after.json` below.

### Focused offline verification

Evidence root: `/tmp/lagrange-wp3-copy-fix-9e55jnc5`.
`before.json` records the clean starting HEAD and six source/report hashes and
modes. `checks.json` records exact command argv, exits, stdout/stderr hashes,
the focused assertions, and matching source hashes before/after the checks.
`after.json` records final file hashes, the append-only report check and the
four-file unstaged diff scope. The exact final diff is retained as `final.diff`.

| Command/check | Result |
|---|---|
| `python3 /tmp/lagrange-wp3-copy-fix-9e55jnc5/check-copy-correction.py` | Exit 0, PASS: exactly one `COPY . ./` in each intended source stage, inherited `/build` workdir, only the specified replacement relative to HEAD, unchanged file modes, and byte-identical verified-artifacts stage/helper COPY. |
| `bash scripts/qa/build-layout-probe.sh --plan --layout all --case all --output-dir /tmp/lagrange-wp3-copy-fix-9e55jnc5/plan-output` | Exit 0, PASS: existing static contract validation; no health inputs; output directory not created; actual Docker/Cargo results remain `NOT_RUN` in plan output. |
| `bash -n scripts/qa/build-layout-probe.sh` | Exit 0, PASS. |
| `bash -n tests/fixtures/build-layout/layout-helper.sh` | Exit 0, PASS. |
| `git diff --check` over the four owned paths | Exit 0, PASS; exact argv retained with the evidence. |

The check environment supplies only a fixed PATH and `LC_ALL=C`. Sixteen
forbidden-command sentinels, covering Docker, Cargo/rustc/rustdoc, host observation,
network and provider commands, recorded **zero calls**. The successful plan's
stdout SHA-256 is
`7629b0455c92c3b31d9b6dbc02c0c7859b7f7c41008dd994149fd7d84e678d16`.

Scope/specification deviations: **없음**. Static-check contradictions, unresolved
implementation items and further decomposition required: **없음**. Only the
three fixture Dockerfiles and this appended report section changed; no staging,
commit, root `.dockerignore` change, test weakening, lifecycle action or cache
removal occurred. No production application source or env/credentials was read.
The prior 124 focused checks, F1–F9, 84 controller cases and publication PASS
remain historical evidence on the recorded prior sources; the full self-test
was not rerun for this COPY-only correction.

Not verified here: corrected actual Docker/BuildKit/Cargo execution, real layout,
cache/route/runtime behavior, host health or release/performance acceptance.
Corrected real layout remains **NOT_RETESTED**, with the actual rerun reserved
to the coordinator's existing build slot after a new clean commit. Other
unresolved/follow-up/unverified items: **없음**.

## 2026-09-15 complete journal range correction

Starting checkpoint: clean `f34bdf74107ebfe15bc4781c372f1cd813e842e5`.
This implements the committed
[complete kernel journal capture amendment](../superpowers/plans/2026-09-15-production-build-resume.md#complete-kernel-journal-capture).
**Offline correction and full self-test: PASS. Actual full-range host proof:
NOT_RUN. Layout G2: NOT_PASSED.** The coordinator retains the actual build slot.

### Retained observation and exact scope

The coordinator supplied a second actual preflight failure at the old journal
1,000-record cap, retained at
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/real-gate.vliy1a/gates.jsonl`.
Its frozen interval was **2026-09-15 02:18:21–02:48:21 UTC**, with
`since_us=1789438701000000`. A separate read-only diagnostic of that same interval
with cap 10,000 returned exit 0, empty stderr and 1,014 valid-looking Docker
veth/bridge entries with zero OOM matches. Its supplied SHA-256 is
`79ac55041ce3fce5440b9d596be10d9cf967dbb43d8bdb60656e3025dd3727aa`.
That diagnostic was **not a gate PASS**. These historical records, the previous
actual smoke result, COPY diagnosis and COPY correction remain intact; none was
rerun or modified by this worker.

Only the two synthetic QA sources and this appended report section changed.
The three accepted fixture Dockerfiles, mode-sensitive source/compile guards,
artifact code, coordinator design/resume documents and product files are
unchanged. No production application source, credentials or env files were read.

### Collection and validation

- The access probe remains exactly the existing current-boot kernel query with
  `timeout 10s` and `-n 1`, including its existing schema/boot validation.
  The range command uses the existing resolved journal executable and
  `LC_ALL=C journalctl -k -b --no-pager -o json --since <fixed run since>
  --until <frozen gate until> --no-tail`. It has no tail limit or message filter.
- `gate_collect_range` uses `Popen`, nonblocking pipes and a selector. It writes
  stdout/stderr to the existing private gate temporary files in chunks of at most
  32 KiB, with incremental hashes and JSONL framing counters. It does not collect
  an unbounded command-output buffer. Both streams must reach EOF, the child must
  be reaped with actual exit 0, and stderr must be empty before collection passes.
- Limits are internal constants: a single 10-second monotonic deadline, 64 MiB
  stdout, 64 KiB stderr, 1 MiB per JSONL line including its terminating LF, and
  fewer than 100,000 nonblank records. As explicitly required in the brief,
  reaching a bound fails closed. There is no public mode, limit override or new
  command-selection seam. Unterminated final records, timeout, failed exit,
  stderr, I/O/spawn failure and unproven termination all reject the capture.
- On a bound or timeout, only the collector's own subprocess receives SIGKILL.
  Reaping uses any remaining portion of the same deadline; at expiry the
  collector yields once and tries a nonblocking reap, without adding a grace
  budget. An unproven exit remains `null` in the capture receipt and `unknown` in
  the public range exit, with `child_reaped=false` and capture failure. The two
  focused timeout cases exercised that failure outcome. Their synthetic PIDs
  were gone after the caller returned; no successful termination was invented.
  Bound failures with remaining time proved reaping and recorded actual exit -9.
- The range parser keeps its required fields, boot/transport/time checks and
  case-insensitive OOM expression. The obsolete `count >= 1000` rejection is
  replaced by the 100,000-record bound. It also independently rejects byte/line
  bounds and missing final LF, and emits fixed diagnostics for invalid JSON.
  A successful complete empty range is still valid only after the access probe.
  Validated record/OOM counts remain separate from collection/framing metadata.
- Public range evidence adds exact argv/locale, limits, elapsed nanoseconds,
  stream EOF flags, completion, actual child exit/reap state, stop/failure state,
  captured byte counts, framing counts and hash scope. Existing stdout/stderr
  hashes, validated count/OOM result and private-temp cleanup remain in use.
  Raw kernel MESSAGE text is not published or retained outside the private temp.

The fixed since origin and frozen until, boot continuity, service/resource and
research-exception checks, execution lock, public CLI, failure/resume behavior,
and helper/controller hash bindings are preserved. Old source-bound failed runs
cannot be resumed against changed sources through the unchanged metadata checks.

Changed source line spans:

| Owned file | Changed lines |
|---|---|
| `tests/fixtures/build-layout/layout-helper.sh` | 795, 801–802, 809–811: capture evidence; 846–976: collector; 980, 989–1011: parser; 1130, 1162, 1222–1225, 1228: initialization and range integration. |
| `scripts/qa/build-layout-probe.sh` | 1550–1552: completion assertions; 1938–2112: focused regressions; 2172: existing fake's exact range argv; 2338: full self-test integration. |
| This report | This appended correction section only. |

### Tests and retained evidence

Evidence root: `/tmp/lagrange-wp3-journal-9puf3vkm`.
Every command below ran as
`python3 <evidence-root>/run-check.py <label> <command>`. Each label's JSON records
actual argv/exit, exact source hashes before/after, separate stdout/stderr hashes,
elapsed time and forbidden-command counts. Children use umask 022 and an explicit
environment containing only the private sentinel PATH, `LC_ALL=C` and private
TMPDIR. All 16 Docker/Cargo/host/network/provider sentinels recorded **zero calls**.

| Label / exact command | Result |
|---|---|
| `syntax-helper` / `bash -n tests/fixtures/build-layout/layout-helper.sh` | Exit 0, PASS. |
| `syntax-controller` / `bash -n scripts/qa/build-layout-probe.sh` | Exit 0, PASS. |
| `plan` / `bash scripts/qa/build-layout-probe.sh --plan --layout all --case all --output-dir /tmp/lagrange-wp3-journal-9puf3vkm/plan-output` | Exit 0, PASS; no health inputs or output-directory creation. |
| `focused-01` / `python3 /tmp/lagrange-wp3-journal-9puf3vkm/focused-loader.py focused-01` | Exit 0, PASS; 62 focused collector/parser/controller invocations in 29.737 seconds. The loader removes only the final main invocation and binds the script directory before calling the actual test function. |
| `self-test-final` / `bash scripts/qa/build-layout-probe.sh --self-test` | **Exit 0, complete PASS**, 1,162.020 seconds (19m22s): new 62-invocation block, existing 124 research/default checks, F1–F9, original 84 ordered controller cases and post-matrix publication checks. Ran once; no full-suite repeat. |
| `evidence-audit` / `python3 /tmp/lagrange-wp3-journal-9puf3vkm/audit.py` | Exit 0, PASS: exact source/log hashes, protected source and original tests, 84 unique ordered runs, 27 expected stops with immediate passing gates and preserved original exits, and zero sentinels. |

Focused coverage includes complete 0/999/1000/1014/99999-record captures, OOM and
malformed records earlier than the latest 1,000, wrong boot/transport/time or
missing fields, stderr, nonzero exit, EOF before a failed exit, open-pipe and
post-EOF timeouts, partial JSON/final LF, and below/at/above byte/line/record bounds.
It verifies child disappearance/reaping evidence, private-temp cleanup, access
probe failures, complete-empty gate behavior and preservation of previous exit 37.

A proven complete 1,000-record interval is intentionally accepted now. The
existing research fake changed only its exact expected range query from
`-n 1000` to `--no-tail`; no health/failure assertions were removed. Missing EOF,
partial final records, bound exhaustion and failed termination are the actual
incomplete-capture regressions. The audit reconstructs the original controller
byte-for-byte after removing the new test additions and restoring that one query
expectation, and compares protected helper sections directly with HEAD.

Baseline and final source identities. The final identities were unchanged
before and after the successful full invocation:

| Source | SHA-256 before | SHA-256 after |
|---|---|---|
| Controller | `f3d7072f7ada68fbf8586e78505e273ca4ed1af769a954298a5fd491e91922da` | `a5ddb9ea01f4e4b28358ac8007991c89a9a022e04235793343c8c41b3a16cfab` |
| Helper | `d89b8e8d8647706c5639426688a364c8bfc7de4c61881f724fb47788950ed666` | `eaaa043416d7b83f7cd30895e30ef3eeb0cc8d448a0b122c34c2276b144b7e52` |

- Focused stdout SHA-256: `dea4dd338f38fb997edf0dcff2ae095ce1baddb32d27830ec6f93e7b35cc469f`.
- Full stdout SHA-256: `2f1e49dd10aac8875034829ead64a7b587e000d89b6bfa2a886d61d756321dce`.
- Full stderr SHA-256: `21e9dfecd860c3b93bd0fb81c0c1c283f023a026e50f44b06a873ef19a81a5d3` (11 expected negative image/artifact diagnostics; retained).
- Full invocation JSON SHA-256: `e3c3dc8f780b3964de43cd247689d8a96a128ba63a357fafdac3fa92c133e7ab`.
- Audit JSON SHA-256: `d68d8d61036941822b416139200cb48db6aa129ed40945cc0b938cb793ee0bec`.

Raw full-suite evidence is retained at
`<evidence-root>/tmp/build-layout-self-test.sFc0RY`. The manifest hashes all
45,084 regular files; 12 deliberate synthetic symlinks are recorded separately
without following them. `full-evidence.SHA256SUMS` SHA-256:
`a3509d6a5861c6510fc7d49e30154c7fe9b1e835794db0d1c13069f1e1d6c738`.
The final three-file diff, report append-only check and before/after file hashes
are retained in `<evidence-root>/final.diff` and `<evidence-root>/after.json`.

Scope/contract deviations, unresolved implementation defects and further
decomposition required: **없음**. No stage/commit, actual Docker/Cargo/host/network/
provider command, service/lifecycle operation, cache removal or Basic Memory
operation occurred. The actual full-range host proof and real layout rerun from
a new clean commit remain coordinator follow-up: **NOT_RUN / G2 NOT_PASSED**.
Real host health, cache/runtime behavior and release/performance acceptance were
not verified here. Other unresolved/follow-up/unverified items: **없음**.

### Coordinator acceptance and actual read-only journal proof

The coordinator recovered the worker's `idle` result and independently checked
the five command receipts, source/log hashes, all 84 ordered cases, 62 journal
invocations and 124 exception checks. Evidence:
`/tmp/lagrange-journal-root-acceptance.json`. No full-suite repeat was needed.

The final helper SHA above also matches the helper before and after the actual
read-only host gate recorded in
`/tmp/lagrange-journal-complete-root-preflight.json`. Its gate evidence is
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/real-gate.CBox4Y/gates.jsonl`.
The same formerly truncated interval passed with 1,014 validated kernel records,
zero OOM matches, 687,850 captured bytes, both EOFs and a reaped command exit 0.
Capture elapsed was 17.414 ms. The gate reason was
`image-build-only-known-incident`; this is not a healthy-release claim.
Actual complete-range host proof is therefore **PASS** for this helper.
Actual all-layout execution and G2 remain pending; the failed `layout-02` run
will not be resumed against changed sources.

## 2026-09-15 — cold-chain P1-only no-cache correction (offline)

Starting clean HEAD: `8e976c50f1d35d5789e3a0eb459dc2ea74174ddd`.
This implements the coordinator's §5.2 cold-chain clarification after the accepted
journal correction at `1a600159c7ccc3f5b2bfa669103106be5cd77f36`.
Only the controller and this appended section change.

### Change and preserved failure

`artifact_build` and `compose_build` now add `--no-cache` only when
`CUR_COLD=1` and their phase argument is `p1`. P0's existing identity policy is
unchanged. Cold/setup/empty-cache P2–P4 retain the target populated earlier in
the same chain. The single-P1 route calls retain their cold behavior.

The private fake reuses `load_state(target, reset=True)` at source compilation
when argv contains `--no-cache`. It resets only that target; verified artifact
consumers never enter source compilation. Selected build arguments, source path,
target namespace, reset flag, previous pending state and observed Fresh values
are added to the fake call evidence.

The namespace/scope/K/H construction, token consumption, Cargo/guard/artifact
policies, oracle Fresh sets, routes, failure/resume behavior, CLI and all prior
tests remain unchanged. The helper, Dockerfiles, compose fixture and design are
byte-for-byte unchanged. No cache namespace redesign or cache removal occurred.

The original actual `layout-03` failure remains **exit 1**, source unchanged,
at baseline/cold P2. Both retained P1/P2 logs show `Removed 0 files` and
`itoa` with `fresh=false`. Existing evidence was read and hashed, never rewritten:

- Receipt: `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/layout-03.json`, SHA-256 `b8858d12c3d6649c44028f1d8edb790a5b84597d39dc4c6b47d7b940bc175243`.
- P1/P2 log SHA-256: `5f9d6b3d7b7919a30f3be45971bf55af6bed0973c410411b6ec263eb0c5de211` / `37f62cf7f0e1e363cfd9d67e789086509df2557b390ac5387c1f5bdc3172b8a3`.
- Coordinator's marker diagnostic: `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/no-cache-diagnostic-f90688b3acac/result.json`, SHA-256 `d898bb4d12dca6d683cf069ac434a4b07871fd3283bfb8d54ce8aee94f2d9066`; actual recorded sequence `MISS,MISS,HIT`, exits `0,0,0`.

### Focused verification

Evidence root: `/tmp/lagrange-wp3-cold-c_wckq30`.
Each command ran through `python3 <evidence-root>/run-check.py <label> <command>`.
Receipts retain exact argv, actual exit, elapsed time, source hashes before/after,
stdout/stderr hashes and sentinel counts. The environment contains only the
private sentinel PATH, `LC_ALL=C` and private TMPDIR, with umask 022.
All 16 Docker/Cargo/host/network/provider sentinels recorded **zero calls**.

| Label / exact command | Result |
|---|---|
| `syntax-final` / `bash -n scripts/qa/build-layout-probe.sh` | Exit 0, PASS. |
| `plan-final` / `bash scripts/qa/build-layout-probe.sh --plan --layout all --case all --output-dir /tmp/lagrange-wp3-cold-c_wckq30/plan-output` | Exit 0, PASS; no health inputs or output-directory creation. |
| `focused-final` / `python3 /tmp/lagrange-wp3-cold-c_wckq30/focused-loader.py focused-final` | Exit 0, PASS; 100.022 seconds. |
| `evidence-audit-final` / `python3 /tmp/lagrange-wp3-cold-c_wckq30/audit.py focused-final` | Exit 0, PASS. |

The private loader removes only the final main invocation, binds canonical
script paths and invokes the actual `self_cold_chain` function under the existing
private self-test flags. That function uses `run_case` for each ordered
cold → exact-repeat → forced-warm → empty-cache chain and `self_controller` for
failure/resume and route cases. The bounded prefix does not claim an all/all
completion. The public CLI is exercised separately by `--plan`.

The final block verifies **21 controller invocations, 155 synthetic build calls
and 75 source-compilation observations**, across baseline/common/grouped:

- P0 no-cache, P1 compile no-cache, P2–P4 compile without no-cache; Compose selects
  exactly one service with jobs=2/parallel-limit=1, and artifact compile jobs=2
  remains fixed in the unchanged Dockerfiles.
- Same warm-chain target namespace through cold/repeat/forced-warm, only the
  existing grouped base/wide split, and distinct empty-cache namespaces. Exact
  repeat produces Docker hits and identical image results; forced-warm executes
  with retained target contents and the unchanged Fresh oracle.
- `compile-fail-p2` in every layout: cold warmup, command exit 42 retained in
  EXPECTED_CAUSE/EXPECTED_STOP and command status files; expected-stop 75 then
  complete 0, passing post-failure gate, reused P1, resumed P2–P4, preserved
  namespace and pending-ledger rebuild. Already-complete resume invokes no build.
- Both route contracts per layout: P0/source P1/artifact P1 remain cold, artifact
  consumers produce no source-compilation observation, and existing image and
  artifact checks pass.
- A private copy of only `artifact_build`/`compose_build` restores exactly the
  two old predicates. All three original variants fail at P2 with exit 1, no
  expected-stop normalization and no P3/P4. L0 reproduces
  `unexpected-recompile-set-itoa`; L1/L2's unchanged oracle checks the library
  first and reports `unexpected-recompile-set-lib`. Their retained observations
  also prove `itoa` rebuilt. The check order and Fresh requirements are preserved.

Development evidence remains intact: `focused-01` completed its controller
cases but exited 1 in the new namespace assertion, which initially omitted the
existing `-target` suffix. `assertion-recheck-01` exited 1 on the new diagnostic
expectation; `assertion-recheck-02` passed after accounting for the existing
wrapper diagnostic and L1/L2 library-first order. Only these new assertions
changed before the final focused rerun. The successful final block was not
repeated. The accepted full 84/F1–F9/62-journal/124-exception suite was not rerun.

### Source identities and scope audit

| Source | SHA-256 before | SHA-256 after |
|---|---|---|
| Controller | `a5ddb9ea01f4e4b28358ac8007991c89a9a022e04235793343c8c41b3a16cfab` | `dd4effc44520fdc6fa190babfa19a7211ceaed924b55c63b917d36887b05894d` |
| Helper (unchanged) | `eaaa043416d7b83f7cd30895e30ef3eeb0cc8d448a0b122c34c2276b144b7e52` | `eaaa043416d7b83f7cd30895e30ef3eeb0cc8d448a0b122c34c2276b144b7e52` |

- Final focused stdout/stderr SHA-256: `a8e1cc3d15616955aeeb6ec1b95c06a5c72c8c810fffa56218b5b0fd559fb894` / `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
- Final focused receipt SHA-256: `4f1d002858acf2d311263204bbeb4e0109c7135af2fdcc1c86609d8444e413bf`.
- Raw focused evidence: `<evidence-root>/focused-final`; 5067 files covered by
  `focused-evidence.SHA256SUMS`, SHA-256 `6da34feb7b822f674ed01a71d65799dc478bf992336c1473d28409d1d5a8b966`.
- `before.json`, `after.json`, `original-evidence.json`, `audit.json` and
  `final.diff` retain the source/report identities, original failure evidence,
  exact scope proof and final diff.

The audit reconstructs the entire controller from HEAD using only the two
predicate/comment edits, selected fake changes and the new private test block
plus its self-test call. This proves the original oracle, gate/resume code,
84-case loop/final assertions, F1–F9, journal and exception tests are unchanged.

Changed line spans: controller **321–322, 365** (predicates/comment),
**1327–1329, 1360–1363, 1385, 1432, 1464** (private fake), **1931–2100**
(focused regressions), **2520** (existing self-test integration); this report
**565–687**, append only.

Implementation/scope deviations and further decomposition required: **없음**.
The negative-test diagnostic ordering is explained above; no acceptance criterion
was relaxed. Unresolved implementation defects: **없음**.
Coordinator follow-up: review/commit, then actual full84/layout execution in the
exclusive build slot. Corrected actual layout is **NOT_RETESTED; G2 NOT_PASSED**.
Actual Docker/Cargo/cache/host/runtime behavior was not verified by this offline
package. Other unverified items: **없음**. No stage/commit, actual host/network/
provider command, lifecycle action, production source audit or Basic Memory
operation occurred.

## 2026-09-15 — fixture exporter metadata correction

Actual `layout-04`, at `b43415afcaff9142fc7c62a3739368db78edbe6c`, passed
all four baseline cold binaries. P2 reused `itoa` (`fresh=true`), confirming the
cold-chain fix. Exact-repeat P1 then used the cached compile vertex but failed
the unchanged full result/image-ID equality check. The run exited 1 after
48.011 seconds; `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/layout-04.json`
retains the actual receipt and unchanged-source proof.

Cold/repeat had identical binary bytes/stdout, runtime payload, revision, image
config (`87d6c5f00c4948b95d5a6512adfb7370d398ea2882cf640b81025b447e213342`)
and runnable manifest (`0d7f114ab41f2f93327ff809bd6741c116377807f9745e90a6c38468da583cc2`).
Only the newly exported attestation and enclosing index/image ID differed.
The retained export lines are in `/tmp/lagrange-layout04-export-identity.json`.
An attempt to inspect the previous index by ID failed after the same tag was
rebuilt; that failed lookup is not treated as proof of the old image's contents.
The cold result and its original BuildKit log provide the recorded comparison.

A separate marker-only Compose test with YAML `provenance: false` still exported
attestations and failed identity equality. Its logs remain in
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/provenance-diagnostic-e6d9ae1fb5da`.
The follow-up using explicit `--provenance=false` passed: no attestation export,
two exit-0 builds and the same image ID
`sha256:c409ddb79c4a14cd1863150ec28da24d19cd1f26abd1ef5ac4c7fc074c67165d`.
Its exact commands/hashes are in sibling
`provenance-diagnostic-5a18ab6d5ca6/result.json`.

The coordinator therefore added that explicit flag only to the synthetic probe's
Compose call. The fake rejects a missing flag and the existing exact-argv audit
requires it. Image-ID/binary/payload/revision equality, Cargo oracles, artifacts,
gates and resume checks are unchanged. Product Compose, provenance and V2 remain
unchanged; this is fixture reproducibility, not product performance evidence.
The corresponding policy and official Docker/Compose references are in design §5.2.

`bash -n`, public `--plan`, and the actual private `self_cold_chain` block passed.
The focused block took 99.721 seconds: 21 controller invocations, 155 fake build
calls, 75 source observations and three legacy rejection cases. Sixteen denied
command sentinels recorded zero calls. Source/log hashes and original exits are
in `/tmp/lagrange-provenance-root-882qagqk/{syntax,plan,focused}.json`.
Tested controller SHA-256:
`8f0b867425fb342e1b02e0bf7c662d9418bd1e5972a6dc8f434736283c836ce4`.
No successful test was repeated. Actual all-layout rerun is pending; **G2 remains
NOT_PASSED**. Other unresolved implementation items: 없음.

## 2026-09-15 actual layout-05: branch return and host clock

The all-layout run at `ab3965876ee6b705ac8b7c4e667f6be3be90d5ac`
ended with exit 1 after 1028.195 seconds. Fifteen baseline scenarios passed,
including both feature-switch trials. Branch-return's forward trial passed;
its return trial failed before Cargo because the copied compile input hash did
not match the host source. No G2 or product acceptance is claimed.

Evidence root:
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/`.
The immutable supervisor receipt is `reports/layout-05.json`, the failed build
log is `runs/layout-05/baseline/cases/branch-return/attempt-002/logs/p1.log`,
and the suite rejection is `reports/layout-05-suite.json`. The source stayed
unchanged and the unit ended with MainPID 0 / ExecMainStatus 1. All 345 recorded
host gates passed; this is the bounded image-only incident allowance, not a
healthy-production-release claim.

A separate two-call scratch COPY/local-export diagnostic reproduced the stale
transfer without compiling or running a binary. See
`reports/branch-copy-diagnostic-7ea7b7f5e3f5/result.json`: the returned source had
compile hash `df295e7d5eb51e7ea03d649fe13db940203c2825d7b1b364a5b0b358b8753dc5`,
but the export retained the forward source hash
`a2d3ce14841669465be24345df2ec93162fe790585258525e111f6a5c86a90d9`.
Only `fixture-app/src/bin/cache-bin-a.rs` differed. The content guard rejected
this mismatch; a transport correction and actual retest remain required.

Separately, host uutils date 0.8.0 did not honor `%s%3N` as milliseconds. Its
variable-width fractional output corrupted `timing.tsv`, including negative
intervals. Those wall-time records are invalid and must not be divided or
otherwise reconstructed as measured performance. Cargo JSON, binary/hash
assertions, Python event timestamps and the supervisor's monotonic total are
separate evidence; their passing checks do not validate the corrupt timing TSV.

The controller now obtains integer epoch milliseconds with Python
`time.time_ns() // 1000000` and rejects malformed/oversized/backward intervals
before appending a timing record. Its focused self-test checks independence
from the incompatible date output, valid/zero intervals and nine negative
inputs. Syntax and focused tests passed; a real 100 ms clock check measured
116 ms. Exact source/driver hashes and command receipts are in
`/tmp/lagrange-clock-root-lmxvw7m0/result.json`. No Docker/Cargo command ran in
that clock check. Corrected actual timing remains unverified; fresh measurements
will accompany the transport retest. Existing raw run evidence is retained.

The caller paths also propagate clock/read-record failures explicitly, including
when the surrounding trial runs in a Bash conditional (where `set -e` alone
would not stop the trial). Focused checks after that propagation fix passed;
the exact receipt is `/tmp/lagrange-clock-root-4lkcus56/result.json` and its
100 ms interval measured 115 ms. This supersedes the earlier clock source hash.

## 2026-09-15 source transport correction: digest context directories

Implemented against clean `02974c31f00b9a1169829d5d087452dbdf2ce328`, following
design §5.2. The controller now copies each attempt's current synthetic source
into a private sibling `source-transport/context-<full source-input-hash>`.
It requires the existing execution-slot descriptor and private attempt root,
copies with `cp -a` into a fresh partial directory, and verifies source/copy
tree hashes and descendant modes/mtimes before publication or reuse. Symlinks,
special entries and mismatched existing destinations fail before Docker.
P0, artifact Buildx and the first Compose `-f` all use that snapshot. Per-phase
injection happens first; the existing preparation/execution wrapper intervals
include materialization. Outputs, overrides, K/P/H, cache namespaces and Cargo
oracles retain their existing paths and contracts.

The original layout-05 failure and same-basename diagnostic above remain intact.
The coordinator supplied the successful three-export forward/return/forward
diagnostic at `reports/content-context-diagnostic-476dbe78b6ea/result.json`
under the same evidence root (SHA-256
`c34859e97696327e018352afc5949cb1865f580cffe1edf6583619faab043ce0`).
That actual evidence was read, not rerun by this worker. Host transport preserves
the original mtimes, including the old-mtime scenarios. Exact mtimes inside a
reused Docker COPY layer are not guaranteed or added as an acceptance criterion;
the unchanged content/mode guard and Cargo/binary assertions remain mandatory.

Offline evidence: `/tmp/lagrange-wp3-context-rap8bk3_/`. All 12 focused controller
checks passed: branch-return (both trials), route-contract, compile-fail-p2 plus
resume, and export-fail-p2 plus resume, for each layout. Their 167 fake build calls
all validated the actual context path and copied inventory/mtime; 96 source
compilations passed the unchanged oracle or retained their expected failure.
The checks preserved original exits 42/1/73, expected-stop 75, resumed completion
0, P1 reuse, pending rebuilds, cache namespaces, P1-only cold compilation,
single-service/jobs=2 and explicit `--provenance=false`.

The private transport regression passed five host-copy cases, three original
flat-basename rejections and 24 pre-Docker rejection checks. Sixty-six paired
wrapper comparisons against starting HEAD differed only in source path arguments.
Existing `self_clock`, wrapper status tests, syntax, public `--plan`, diff checks
and the scope audit passed. The first wrapper test rejected its old 0775 test
root; `checks/wrappers-01.json` and its log retain exit 1. Its setup now explicitly
creates a private 0700 root; `wrappers-02` passed with every assertion preserved.
Twenty-one denied command sentinels recorded zero invocations.

Controller SHA-256 changed from
`e84c8dbd0fcf4b13dc004f3803fb6c54b129a36fa33b566638599bdf54573576` to
`c1084359a6358f6191bef52f05fc89fc01829dbc816d98fb90aea36e4b4f47cf`.
Exact argv/exits/times/source/log hashes are in `checks/*.json`,
`focused-result.json`, `argv-result.json`, `scope-audit.json` and the final
evidence manifest. Only the controller and this appended section changed;
clock functions/self_clock, helper, Dockerfiles, Compose and source fixtures
are unchanged. No staging or commit occurred.

Implementation deviations / further decomposition: 없음. Unresolved implementation
items: 없음. Actual corrected layout is **NOT_RETESTED**, **G2 NOT_PASSED**.
The full offline 84-case/journal/exception suite was not repeated. Coordinator
follow-up is the actual focused acceptance and full matrix from a clean commit;
no actual build or product performance/adoption claim is made here.
