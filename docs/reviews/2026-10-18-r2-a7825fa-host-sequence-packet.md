# R2 candidate a7825fa image and conditional integration packet

R2-04/R2-09, Wave104. **IMAGE_ACCEPTED_WHOLEFLOW_FAILED_CLOSED; BOTH_DISPATCHES_CONSUMED.**

2026-10-19 KST: Owner explicitly answered “승인하니 이어서 진행해”.
Authorization `34e176a9` binds this prepared packet to one seventh image attempt
and, only after full image/recovery closure, one fourth actual isolated wholeflow.
Independent recovery started23:09:36UTC; image dispatch1/exit0 and normal Owner
exit0 observed. Shared image window ends2026-10-19T11:09:36.633139Z;
conditional full recovery must complete within60min of its dispatch and by12:09:36Z.
No additional question occurred between phases. Both dispatches have now been used;
image acceptance and failed wholeflow closure are recorded below. No retry is authorized.


### Wave104 terminal recovery — 2026-10-19 KST

Owner 승인으로 a7825fa의 일곱 번째 이미지 빌드를1회 완료했다.12이미지·704파일·49게이트·7번들·18바이너리·2147소스pin을 인수했고23:26:51UTC 같은 Owner 컨테이너 복귀,23:26:52UTC 독립 복구 성공종료·무프로세스·READONLY 정산을 확인했다. 이후 네 번째 실제 격리통합을23:29:50UTC1회 실행했으나 producer exit1/INVALID_ARTIFACT로23:29:57UTC 실패했다. 최초승인·게시·재시작·복원 단계는 미도달이다.23:30:55UTC 임시7컨테이너·네트워크·작업디렉터리 부재와 운영11상태·현재릴리스·보존볼륨 불변을 회수했다. 두 실행 승인은 모두 소진됐으며 자동재시도 없이 소스 검증 경로를 추적한다. 이번 세부원인·기존512 내부원인·base최종비교·미출력62행은 UNKNOWN, 과거0120정리는 INCOMPLETE다. G4·배포·Owner·3거래일·C_R2 미완료.

- Image closure `1048f7858e2d941d182551b304091be96d407fde61c1badda3cb23b03ad5bd64`;
  manifest `6e27840a52d0e54e5001a697ca51a0e42a7f7be82d035ab1fa0defa430d02bf5`.
  Owner restored23:26:51.673989UTC; build terminal23:26:51.681727;
  independent recovery terminal23:26:52.683327. Both actual units inactive/dead,
  MainPID/ControlPID/cgroup0; normal Owner exit0, kernelOOM0.
- Fourth actual wholeflow / fifth prepared packet dispatched23:29:50.260147UTC,
  driver exit1 terminal23:29:57.188534. Only PREPARED_SYNTHETIC_EXTENSION.
  Finite producer diagnostic owner_beta_daily_cycle/INVALID_ARTIFACT, Docker exit1,
  stderr61bytes SHA `daeeae4490b9f694f8cfe4894302a798cd1e6663d416ce8bc095547b780e340d`;
  stdout0bytes. No raw output retained. Precise rejected predicate remains UNKNOWN.
- Wholeflow closure `889ad46afa1c8d72e25e650ac3042700f578299ff8fcf1d7ab51dd7db46c874a`,
 16pins/full recovery23:30:55.755666UTC. Seven containers and exact network destroyed;
 namespace/stage absent, driver/PIDs/cgroup0. PG/API137 occurred during private cleanup,
 kernelOOM0. Currentfce9b90/exact11states/retainedvolume unchanged. Failed unit and result preserved.
- Both approved dispatches consumed; no automatic retry/new host authorization.
  Follow up the exact InvalidArtifact call path in source, preserve guards, establish
  a meaningful regression and a new immutable candidate before another host packet.
  Static analysis or CI cannot substitute for actual later phases or Owner acceptance.
- Successor heartbeat4fbc67c0 confirmed23:31:28UTC, handoff persisted before9fb748db deletion.
  Renew by2026-10-19T10:30UTC/before tick47; expires11:31:28UTC. Same-agent bounded
  continuation, native Ralph blocked/no resume API, no delegation or outage protection.

The following preparation record describes the scope presented before approval.
Candidate `a7825fa8f22835ced72f342837ea0e81aff812ff` has complete hosted acceptance.
The proposed next scope is one seventh image-maintenance attempt, then only after
complete image acceptance and restoration, one fourth actual isolated wholeflow
execution (fifth prepared packet;899a2 was never dispatched). One new Owner decision
covers both ordered phases; no repeat question between approved phases and no retry.

The previous Wave102 approval was consumed by the512625a image build and isolated
wholeflow execution. The current task explicitly requires a new decision after a
concrete successor packet. Root access is not execution approval. No new root or
production reads, project/control execution, local tests, parser/compiler or
independent review occurred in this preparation.

## Expected impact and remaining uncertainty

The existing Owner V2 container stops normally once, pausing its updates and quote
processing during image preparation. Independent recovery must be actually active
before the stop and restore the same container/image healthy immediately after
build terminal or failure. Nine other serving containers must remain healthy;
research remains stopped with exit2. The image window is bounded by12 hours, a
maximum deadline rather than a duration estimate. The subsequent isolated driver
has20 minutes, with full terminal/cleanup reporting within60 minutes.

**This candidate fixes diagnostic loss, not a proven producer runtime cause.**
The prior512 producer exited1; its internal diagnostic was lost. It remains UNKNOWN.
The new source retains only finite attached-command service/exit, full stream
hashes and exact allowlisted daily diagnostic labels. Another failure is possible.
No claimed success of later phases, automatic retry or relaxed exit/OOM/state/
volume/resource guard is permitted. On failure recover the actual finite evidence,
owned-resource disposition and terminal state; unknown output stays UNKNOWN.

Retain anonymous volume
`159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`.
Ownership is unproven because the prior0120 deleted bootstrap Mounts association
was not retained. Its metadata/no-reference state matched before/after512. This
packet allows only exact metadata/reference comparisons before/after new
integration. No mount, content read, deletion or prune. Drift blocks admission.
The prior0120 cleanup remains INCOMPLETE even if new cleanup succeeds. Unexpected
new persistent volumes retain their associated container and intent evidence.

G4 backup/version transition, production deployment/migration, research activation,
provider/Auth0 access, trading, Owner use and final acceptance are outside this
packet. No host ownership/permission changes or historical-unit reset/relaunch.

## Verified inputs and static preparation

[Hosted recovery](2026-10-18-r2-wholeflow-producer-diagnostics-ci.md):
CI39131944940 attempt1 SUCCESS13:51:09UTC and functional smoke39131946761
attempt1 SUCCESS14:14:35UTC. Full recovery14:15:29UTC matched9 full logs,
8 actual checkout SHAs,1302 input pins/526 committed blobs,150 required names,
15 image outcomes and41 wholeflow cases. All carried Rust/strict Clippy/format,
PG19 processes/146 names/0060 32+73/83 bootstrap/62 checksums twice/0039 cleanup,
Web and actual publication/repeat/10 cleanup0/owned-resource absence passed.
The nine new diagnostic cases are mocked regressions; five carried Docker cases
create/inspect hosted private fixtures. Neither establishes approved-base host
wholeflow. Explicit exclusions and unprinted independent nested UNKNOWN remain.
No rerun/cancel/duplicate of either completed workflow.

Clean detached `/data/worktrees/3puw275b/r2-release-a7825fa` has2147 tracked
file/symlink pins matching the candidate. Prior512 checkout's2144 pins were checked
against its own commit and preserved. Inert image env remains byte-identical0600.
Eleven image controls have only finite candidate/path/unit/dependency-hash
rebindings; inverse bytes, expanded argv and unit templates match. Three runtime/
work probeSQL files are unchanged. No prepared control was executed or installed.

The coordinator's initial2145 file-count assumption failed after creating only
the clean checkout. Actual Git tree has2147 files because three review documents
were added since512. The initial draft/error is retained; preparation resumed
only after exact clean detached source verification and exclusive output creation.
No host or project command ran on that error. Final2147 pins are authoritative.

Wholeflow admission differs from512 only by two attempt paths, with inverse bytes
matched. Fixed base, metadata/ancestor/data-root/service-parent/PG/retained-volume
boundaries remain unchanged. Prepared recovery additionally rejects duplicate
result keys and validates the exact finite command_failures shape/labels before
persisting it; raw streams remain in RAM. Its labels match accepted source bytes.
These coordinator drafts have static disposition only, no local execution or new
independent review. Current host health, source-input hash, output/unit absence,
new image IDs and manifest remain UNKNOWN until approved fresh admission.

Receipt paths are relative to `target/r2-daily-beta/`.

| Receipt | SHA-256 |
| --- | --- |
| `ci/a7825fa/hosted-terminal-evidence.json` | `a700c5eb3d139f0d14c47720e47695da2729afa5331b111526ccd3ae36c70784` |
| `ci/a7825fa/functional-smoke-evidence.json` | `8960e4a1d2819aeaf28a52fca658b0a132ec3652df281a27228ae8e62353a675` |
| `image-preparation-a7825fa/preparation.json` | `954370776561e046e4edc99d102369193205f9d23717b8f074c52653f36f8252` |
| `wp-8/maintenance-a7825fa-a7/preparation.json` | `f721b1035bfb5301039832f29d038ec744abb85114da36867e30f13f72d93221` |
| `wp-8/maintenance-a7825fa-a7/command-ledger.json` | `f261a4f6c8c654f4880ad42427167972dca98a1186680350c3add2e6d46b4b1d` |
| `wp-8/wholeflow-a7825fa-a5/preparation.json` | `01f0bb707e25c0065f748acf72f6d201360767b1ff7f469bedc9d982929e5220` |
| `wp-8/wholeflow-a7825fa-a5/command-ledger.json` | `7bd6b758a4d8243410fd6ee1020aa11921ce7004266ae31deda950b561374bcf` |
| `wp-8/wave104/retained-volume-disposition.json` | `56b15d204748c5c676fe271366e1269bce66825640c3ccf21cddb73e69ad68c9` |
| `wp-8/wave104/prepared-sequence.json` | `511aab1ae2f9c4a4200acc97946b3ff1666c559f89bf9aa67254e1f7851865c6` |
| `wp-8/wave104/final-static-reconciliation.json` | `f666ac7b60e2b255bcb0c3c1b4ccd4450bf21c6702117d0eb96f7bfa9bdbbedf` |

Final static reconciliation matched2147 source pins,11 image controls/inverse
bytes,11 critical wholeflow pins,7 sequence references,1302 frozen hosted pins,
0600 inert env, admission inverse bytes and expanded image/recovery argv.

## Ordered execution after new approval

1. Reconcile exact scope/time, all source/control/env pins and clean checkout.
   Perform fresh bounded host/no-clobber and READ ONLY work admission. Execute
   official plan/preflight and observe source-input hash afresh. Never reuse a
   previous attestation or C3 wrapper. Historic active-exited units require exact
   PID/cgroup evidence; do not stop/reset them. Install only the new controls.
2. Create a fresh shared12h window before launching independent recovery; verify
   it actually active before ONE stop intent/normal SIGTERM. Owner exit0 is
   mandatory, no forced-kill fallback. Require settled work and valid attestation.
3. Official builder runs in background systemd Nice10/idle I/O/weights20,
   COMPOSE_PARALLEL_LIMIT1/CARGO_BUILD_JOBS2. Four batches3; exactly ONE service per
   Compose invocation. All health/memory/swap/OOM/previous compiler termination
   gates remain. Recover12 exact image IDs/OCI revisions, saved bytes/layout,
   source bindings and immutable manifest. Markers alone do not accept images.
4. Recover successful actual build/ExecStopPost and independent recovery terminal
   events plus no processes. Verify same Owner ID/image healthy, exact11 production
   states and postrestore READ ONLY work settlement. Missing unit alone is not
   terminal proof. Failure closes this attempt without retry or wholeflow launch.
5. Only then fresh five-minute wholeflow admission binds NEW accepted manifest/
   12 images/2147 pins/11 critical pins/local PG digest/fixed base/resource gates/
   production states/retained volume. Resolve base ONLY from fixed owner-beta-runner
   `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`
   single readonly `/data/owner-beta-artifacts` bind; no old image/path fallback.
   Bind exact ledger argv; dispatch once. Driver1200sec/2GiB snapshot, private
   nonce DB/network/resources, no provider/Auth0/production SQL; outer50min,
   SIGINT/stop5min/SendSIGKILLno. Complete recovery within60min.
6. Recover actual admissions/publication/HTTP/restart/capture-restore/62 checksums,
   original base unchanged, new-owned cleanup, retained old volume unchanged,
   exact11 production states/current release unchanged and actual terminal/no
   processes. On failure retain finite diagnostics and hashes, never raw output.
   Deadline, unobserved phases or uncertain cleanup remain INCOMPLETE/UNKNOWN.

New rootaux `/etc/lagrange/release-manifests/.r2-maintenance-a7825fa-a7-20261018`;
units `lagrange-r2-build-a7825fa-a7.service` and
`lagrange-r2-recovery-a7825fa-a7.service`.
Owner ID `f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`,
image `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
Stop only in first hour; build10h/watchdog10.5h/attestation11h/no renewal.
Wholeflow dispatch by image-window hour12; full recovery by hour13 and within60min.
Unit `lagrange-r2-wholeflow-a7825fa-a5.service`; new result
`/var/lib/lagrange-r2-wholeflow-evidence/a7825fa-a5-20261018/result.json`.

Prior512 images/manifest/closure6566857a and failed wholeflow closure4d3770a7,
all0120/899/2a/ab48 evidence/failed units and retained volume remain unchanged.
The root lease observed through2026-10-20T17:00:57Z is access only, not permission.
No new policy probe or permission-policy change is needed for this preparation.

Until the new packet decision, bounded static no-change reconciliation only;
no repetitive production probes, documentation commits or approval questions.
Overall R2 remains open through current images/approved-base wholeflow/G4/
deployment/Owner use/three consecutive trading days/final C_R2. Native Ralph is
blocked; the same-agent heartbeat provides bounded continuation, not a worker or
daemon-outage protection. Confirm and persist any successor before deleting its
predecessor; renew before12h expiry and tick47.

One async decision question was issued after packet completion; answer pending.
Heartbeat64149a8c confirmed14:23:59UTC/handoff7c817362 persisted before6df07fcf
deletion. Renew by2026-10-19T01:00UTC/tick47; expiry02:23:59UTC.
