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
