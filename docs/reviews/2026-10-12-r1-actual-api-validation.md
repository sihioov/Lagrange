# R1 WP-5 actual API browser validation — read-only preflight

**Status:** PREFLIGHT COMPLETE; execution is **BLOCKED pending the coordinator's heavy-slot handoff and Cargo-cache propagation resolution**. The unquoted report write caused unintended command substitutions; the original invocation and limitations are reconciled below. Actual acceptance has not run.

## Scope and provenance

- Repository `/data/worktrees/3puw275b/orange-dugong`, branch `orange-dugong`, HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`; shared dirty worktree (109 paths at initial check). Only this new report and private ignored planning evidence were written; no source, staging, or commit.
- Read the actual-router runbook section [393 onward](/data/worktrees/3puw275b/orange-dugong/docs/runbooks/service-status.md:393), the requested Rust-build, actual-API, QA-library and snapshot-publisher files, plus directly referred unit/loader/Compose/fixture helpers for invocation and cleanup.
- Reviewed-input SHA-256 manifest: `target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/reviewed-inputs.sha256`; manifest SHA-256 `929a0a9ad70689a1b841e905593a3ca31b6432914d16209cff126f0f7141495d`. The command plan is `target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/command-plan.md` (SHA-256 `dd817105a10f834783d94035fbe6f174d6fb0b4d757718852fd11c8902be02d6`). Both are under ignored `target/`; directory mode 0700, files mode 0600.
- Primary reviewed-file hashes: runbook `4c53b7686666f0d00b903bb022af95575aff27c0b911e6b7cecc822e58b85c49`; Rust build helper `419cb4336b896352d1b354e9c650aec165cde19ddc11b85e1bb59570680bb54a`; actual API helper `1369542c424724536d74e0e3dc2e22af7761e6f95477117e837198a11b1b12c6`; QA library `d835e2de2defb56f14e3453230bbd2ef166db9e4f418a370ec1638a6abb7375c`; publisher `65797b6672568d2ec8762adf1aed9edeea3a9109841d48c80de93c6fdeda16a1`.

## Observed tools and inputs

- Required command paths resolve: cargo, Docker, ionice, pgrep, systemd-run/systemctl, Node/npm, Python, readelf/ldd, curl/jq/timeout/grep. Lookup only; no versions, builds, or tests were run.
- Expected API binary `target/debug/examples/service-status-browser-harness` is absent. Task Cargo home/target directories exist. Playwright CLI link and Chromium/headless-shell 1228 files exist. `apps/web/.next/standalone` exists, but the default actual lane rebuilds URL-specific Next output.
- Pinned image is cached: `postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a`. Read-only project-label queries found no WP-5 containers, networks, volumes or reserved API container. No active WP-5 or actual-specific systemd units were listed; one old failed non-actual contract-fixture unit was visible in the all-units listing and was not touched.
- Current resources: 8.5 GiB MemAvailable; 8 GiB swap allocated with 176 KiB free. The actual-lane ports 55445 (DB), 38194 (API), 33044 (Next) had no listeners in a read-only check. Recheck at execution time. UID is 1000. Planned private root/artifact paths are absent; `test-results/service-status-qa` is absent and will be created by the helper only during later execution.

## Later command sequence and required checks

After explicit heavy-slot handoff only, use matching unique roots:

```sh
export SERVICE_STATUS_QA_ROOT=/tmp/lagrange-service-status-wp5-actual-preflight-20261012-r1
export SERVICE_STATUS_QA_ARTIFACT_ROOT="$PWD/test-results/service-status-qa/lagrange-service-status-wp5-actual-preflight-20261012-r1"
export SERVICE_STATUS_QA_SLOT_CONFIRMED=yes  # only after handoff
export CARGO_HOME="$PWD/target/r1-runtime-fxi6ltmv/cargo-home"
export CARGO_TARGET_DIR="$PWD/target/r1-runtime-fxi6ltmv/cargo-target"
install -d -m 0700 "$SERVICE_STATUS_QA_ROOT"
bash scripts/qa/service-status-rust-build.sh start
bash scripts/qa/service-status-rust-build.sh wait
bash scripts/qa/service-status-actual-api.sh
```

The exact sequence is also recorded in the private command plan. Use the default actual lane, not `--reuse-prebuilt` or a synthetic route. Before starting, and at helper preflights, confirm slot, no Cargo/rustc, available memory, unique units, free ports, and no conflicting WP-5 resources.

- **Cache propagation blocker:** the Rust helper's `systemd-run` environment forwards `PATH` and Cargo limits but not `CARGO_HOME` or `CARGO_TARGET_DIR` ([lines 108–122](/data/worktrees/3puw275b/orange-dugong/scripts/qa/service-status-rust-build.sh:108)). The reserved cache paths are not proven to reach its transient unit. Coordinator must resolve a private one-run invocation; do not alter global systemd-manager environment or source.
- Rust build: offline API example, 1 job, debug/incremental 0; MemoryMax 1536 MiB, swap 0, CPUWeight 10, RuntimeMax 90 min. Stage memory floors: Rust/Next build 1792 MiB, actual runtime start 2048 MiB, API 1536 MiB, Next server 1280 MiB, each browser phase 1024 MiB.
- Actual ports are 55445/38194/33044. Project `lagrange-service-status-wp5`; helper refuses existing project resources and uses the pinned image with pull disabled. DB is disposable and loopback-only. Loader probe is network-none/read-only, 256 MiB, swap 0, timeout 30 sec; fallback contains only the example's resolved regular ELF loader/libraries in the private tree.
- Next build cap is 1536 MiB/swap 0/CPUWeight 10/90 min. DB cap is 512 MiB. API and Next server each cap at 256 MiB/swap 0; each browser phase caps at 768 MiB/swap 0, CPUWeight 10 and one worker. Publisher is confined to the private actual tree, network-none, cached image, 128 MiB/swap 0, CPUWeight 10, 30 sec per operation.
- Acceptance checks: authenticated Dashboard HTML/static asset; two `@actual` Chromium phases against the same API process/session; daily→daemon policy replacement and six-second gap; same commit, original timestamps and changed inode; both proofs and audit `owner_status_reads >= 2`.
- Cleanup stops/removes only owned units/containers, tears down this Compose project/volumes, uses the confined publisher for root-owned snapshot/handoff cleanup, removes session, and releases the slot lock only after cleanup succeeds. On cleanup failure retain ownership evidence/lock for review.

## Deviations and limits

- A first read-only `ss` filter mistakenly included prohibited port 33001; netlink access was denied before listener data returned. No connection, PID/process inspection, reuse or action occurred. Do not query that port again.
- A broad read-only systemd unit listing initially emitted one stale unit description with nonsecret execution metadata. No environment property/journal was read, and no unit was touched. A first image-inspection format string failed parsing; corrected to `.Id` only. A failed unquoted report heredoc evaluated Markdown backticks; it was replaced safely. Follow-up checks found the planned QA root/artifact absent, no matching active unit/process, and no WP-5 project resources at that later check; that observation alone does not establish absence of transient activity.
- Unresolved: heavy-slot handoff and Cargo cache propagation. Actual build/runtime/browser results, execution-time resources and cleanup: not verified. Missing preflight inputs: none. Further decomposition: none.

## Recovery note — unquoted report heredoc

- The report-write command used an unquoted heredoc delimiter. Bash therefore treated Markdown backticks as command substitutions. Retained tool output was truncated; a complete ordered expansion transcript and per-substitution exit codes are unavailable.
- Diagnostics confirm attempted substitutions included `/data/worktrees/3puw275b/orange-dugong` (reported “Is a directory”), `orange-dugong` and the HEAD hash (command-not-found diagnostics), the reviewed-input manifest path (permission-denied diagnostic), hash strings (command-not-found diagnostics), and `.Id` (command-not-found diagnostic). Exact exits for these substitutions are unknown.
- The authored Markdown also contained a bare `ss` code span. Retained output shows repeated `Cannot open netlink socket: Operation not permitted`; whether every diagnostic came from that substitution is unknown. No listener rows were retained. This could have attempted a broad socket listing; no successful foreign-port data is evidenced.
- Most consequentially, the Markdown fenced shell block contained the later `export SERVICE_STATUS_QA_ROOT=...`, `export SERVICE_STATUS_QA_ARTIFACT_ROOT=...`, `export SERVICE_STATUS_QA_SLOT_CONFIRMED=yes`, Cargo-cache exports, `install -d`, `service-status-rust-build.sh start`, `service-status-rust-build.sh wait`, and `service-status-actual-api.sh` commands. The backticks around the fence could have exposed that text to command substitution. Whether it executed, and any individual exit/output, cannot be recovered from the truncated output. It could have attempted a build and the synthetic actual API/DB/browser lane; no provider call was explicit in that block, but execution is unverified.
- Later checks found the planned QA root/artifact absent, no matching active WP-5 unit/process, and no WP-5 containers/networks/volumes. These establish only that no such persistent resources were present at check time; they do **not** prove that no transient command started or completed. No command-specific receipts were retained, so the prior categorical “no runtime started” claim is not established.
- Recovery added no runtime queries or actions. The no-heavy-slot state remains; actual execution must wait for coordinator review/clearance. Unresolved: exact full expansion order, individual exits, and whether the potential fenced command block ran. Not verified: absence of transient build/runtime activity during the interval. No further decomposition required.


## Coordinator reconciliation — original tool invocation recovered

The exact09:38:04 UTC invocation was recovered from the owned worker session and retained privately as `coordinator-heredoc-original.json` (SHA256 `badf0d8d2e752091ece214e1decc48ddf6d9d860ed7fab852bfa503b55ae9eac`). Contrary to the recovery note above, the failed heredoc had no fenced shell/export/install block. It contained separate inline backticks for the three `bash ...rust-build.sh start`, `wait`, and `...actual-api.sh` commands, so those were unintended helper attempts. Separate assignment substitutions cannot export their variables into the parent shell. The original output remains truncated; exact individual exits and the ambient QA-root value were not captured. Do not claim proven zero transient activity. No explicit provider command is present.

The inline `owner_status_reads >= 2` also explains a new empty repository-root file named `=`. Its size0, UID1000 and mtime09:38:05 match the failed invocation. The coordinator preserved its bytes and metadata observation, then moved only that artifact to the private evidence directory. Product source was not changed. Current-resource absence was previously checked; all actual execution will recheck its owned scope before starting.

The coordinator prepared but did not execute a private Cargo wrapper (`coordinator-bin/cargo`, SHA256 `590b40395e7f9ce1ae9c32a5950170e8d76a12e4eefe0ffefc30dac4bc224563`). It accepts only the exact existing build argv, supplies task Cargo home/target plus offline mode, and execs the absolute existing Cargo path. Bash syntax passed. The official helper's one-job, memory/swap and lifecycle limits remain unchanged. A successful build must be copied exclusively to the previously absent expected example path with source/destination hashes before actual runtime. No global environment or source helper changes are needed.

## Authorized actual isolated API run — 2026-10-12 09:59–10:00 UTC

**Result: APPROVED for this isolated actual-router browser lane only.** The explicit heavy slot superseded the earlier execution blockers; previous failed-heredoc records and the coordinator correction remain intact. The recovered original had inline helper attempts, no fenced export/install block. Earlier transient activity remains unverified; current-resource absence was checked before this new run.

- Provenance: HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate, not an immutable source snapshot. Before/after Git status is retained privately. All 572 Rust files and 946 total relevant Rust/Web/helper/fixture/manifest inputs were captured before and after, with identical path sets and hashes. Cargo.lock is unchanged. Reviewed four helper hashes match the preflight list; wrapper hash matches `590b40395e7f9ce1ae9c32a5950170e8d76a12e4eefe0ffefc30dac4bc224563`.
- Private evidence directory: `/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012` (0700; files 0600). `evidence.sha256` SHA-256: `f32e2934c1cb41003936c77d99e50c69a6f1710113526d2c591701612f00a7cf`. Before/after input-manifest SHA-256: `8dde7b5769f46e7aa097121fe8b4b389e555a5b37766fb8cc89538c618fda48c` for both. Retained safe receipts/proofs, commands, original exits, source manifests, Git status, binary-copy receipt, browser-result hashes, private-log hashes and cleanup summary provide disk evidence beyond /tmp.
- Artifacts: `/data/worktrees/3puw275b/orange-dugong/test-results/service-status-qa/lagrange-service-status-wp5-actual-preflight-20261012-r1/actual`; both phase JSON results and safe attachments are retained there. Sessions/cookies and provider bodies are excluded from this report.

### Commands and original exits

The private `run-stage.py` launcher clears inherited PG/DATABASE_URL/KIS/DART/OpenDART/C2/LAGRANGE_DB/SERVICE_STATUS variables without printing values. It sets only the assigned roots and confirmed slot. For Rust start/wait it prepends the verified `coordinator-bin` directory to existing PATH; the private wrapper supplies the task Cargo home/target and offline mode. Actual execution uses the original PATH and default home Playwright cache, without `--reuse-prebuilt`.

```sh
sg docker -c 'python3 /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012/preflight.py'
sg docker -c 'python3 /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012/run-stage.py start'
sg docker -c 'python3 /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012/run-stage.py wait'
sg docker -c 'python3 /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012/run-stage.py actual'
sg docker -c 'python3 /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/actual-api-preflight-20261012-r1/execution-20261012/postflight.py'
```

- Preflight first sandbox attempt exited 1: `sg: setgid() failed: Invalid argument`. The authorized exact native retry exited 0; no runtime had started in that attempt. Subsequent scoped native launcher invocations above each exited 0. No real failure was retried.
- Launcher commands: `bash scripts/qa/service-status-rust-build.sh start` exit 0; `bash scripts/qa/service-status-rust-build.sh wait` exit 0; `bash scripts/qa/service-status-actual-api.sh` default mode exit 0. Original Cargo command was exactly `cargo build -p api-server --example service-status-browser-harness --offline`, exit 0. Default actual helper's bounded `npm run build --workspace=@lagrange/web` exited 0; both official Chromium invocations exited 0. No installs or image pulls occurred.
- Assigned QA root was absent before exclusive creation: `/tmp/lagrange-service-status-wp5-actual-preflight-20261012-r1`, UID1000/mode0700. Assigned artifact root was absent. Slot was explicitly `yes`. Reserved ports 55445/38194/33044 bound and released before execution; owned WP-5 containers/networks/volumes and active units were absent. Cached PG digest matched the supplied digest. No Cargo/rustc was active before build or before actual execution.
- Exclusively copied the regular binary from task `cargo-target/debug/examples/service-status-browser-harness` into the absent repo `target/debug/examples/service-status-browser-harness`; symlink ancestors refused, no overwrite. Both SHA-256 `2dede18a0d0d77f02f0cbe829ab8ffa238d414d13655fdf853b0c55fbf85da43`, size 86714616 bytes, destination mode0755. Copy/hash/Cargo.lock assertions exited 0.
- Stage log hashes: start `fbf87ee5edc3ddcd28cdc6f159e7817117a55b37a8cdb341ce0f6e63fac291b2`; wait `3da61954180cf61de81430b2f2986b11e7cd4c89c6950a6669abdb80b6f5ba33`; actual `655a125336fee4976dac5cf21709604289e2a8b902b1c3e1a1ebb2f5b1c4dc98`. Evidence scripts contain the exact read-only pre/post checks; each exited 0. Additional receipt parsing, exclusive binary copy and evidence-hash assertions exited 0.

### Actual executed acceptance

| Phase | Discovered | Passed | Failed | Skipped | Retry | Flaky | Browser start UTC |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| initial daily | 1 | 1 | 0 | 0 | 0 | 0 | 10:00:34.290 |
| updated daemon | 1 | 1 | 0 | 0 | 0 | 0 | 10:00:43.960 |

- Both selected the existing `@actual` real DB-backed router test against Chromium, one worker. Actual/helper/browser/build logs scanned for literal `SKIP:`: none. JSON confirms one result per phase, status passed and retry0; no zero-test acceptance.
- Real authenticated `/` HTML and local static asset each returned 200; safe readiness receipt records credential_logged=false. The test verifies the actual Owner cookie flags without exposing values. Status API routing was not intercepted.
- One actual API container/process served both phases; ownership/metrics retain its identity. Frozen official script copied the session once and both test invocations used the same private session file; it was removed only by cleanup. No separate session hash is retained because the session is intentionally private and deleted.
- Publisher proofs verify root:10001/0640/nlink1, daily→daemon, inode9196285→9196520, same synthetic fixture commit and identical original source timestamps. Initial browser lifecycle ended 10:00:35Z; updated started 10:00:43Z, an observed lower bound of seven seconds after accounting for whole-second receipt precision. The official script also executed its six-second sleep after replacement. Audit `owner_status_reads=2`.

### Resources, caps and cleanup

- Preflight MemAvailable9053644 KiB, SwapTotal8388600 KiB, SwapFree4 KiB; before actual MemAvailable9236520 KiB/SwapFree148 KiB; after MemAvailable8942368 KiB/SwapFree76 KiB. Swap was not treated as usable headroom. All helper memory floors passed.
- Verified lifecycle caps retained: Rust/Next build1536MiB, Next server256MiB, each browser768MiB; swap0 and CPUWeight10. Rust peak1385418752 bytes; Next build peak1610612736 bytes with memory.max events71, oom0/oom_kill0, original exit0. Initial browser peak758812672 bytes; updated464195584 bytes; both oom0/oom_kill0 and original exit0. Next server exit143 is the prescribed cleanup stop; runtime test stages had already passed. Owned container snapshots record no OOMKilled=true; all original caps were preserved, including confined publisher/loader/DB/API caps.
- Actual cleanup receipt: original_exit0, cleanup_failed0, final_exit0 at10:00:47Z. Postflight verifies no owned containers/networks/volumes or active WP-5 units, all eight recorded owned PIDs absent, no Cargo/rustc, and ports55445/38194/33044 bind/release. Private host-snapshot/handoff/session/DB override and slot lock are absent. Evidence and the authorized copied binary remain; no unrelated resources were touched.
- Kernel journal/OOM evidence: not queried; owned cgroup lifecycle/container OOM evidence is retained. Foreign PID1780113/port33001 was not inspected or contacted in this run.
- Report change: appended lines67 onward only; private evidence/artifacts and the explicitly authorized exclusive binary copy are the other writes. No source/global/helper changes, staging or commit. Deviations: sandbox-only sg retry as authorized; none otherwise. Unresolved/follow-up: none for this isolated lane. Missing: none. Not verified: earlier transient activity, independent session fingerprint, kernel-wide OOM history, production/hosted CI/full R1 acceptance. Further decomposition: none.
- **HEAVY_SLOT_RELEASED; actual idle.** This result establishes only the authorized dirty-worktree isolated actual API/browser chain.
