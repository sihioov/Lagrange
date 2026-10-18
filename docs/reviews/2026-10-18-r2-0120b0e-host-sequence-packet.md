# R2 candidate 0120b0e image and conditional integration packet

R2-04/R2-09, Wave100. Candidate
`0120b0ea6d54198e030545c08276d257fd0da17d` is HOSTED_ACCEPTED_ONLY.
Owner explicitly approved this packet with “승인하니 이어서 작업 진행해”.
The fifth image-maintenance attempt was dispatched exactly once at09:25:19UTC
and is running. After complete image acceptance and restoration, the same
approval covers one isolated approved-base wholeflow run. The frozen preparation
receipts retain their historical PREPARED_NOT_AUTHORIZED_NOT_EXECUTED status.
There is one production interruption, no automatic retry or window renewal.

## Owner impact and decision boundary

The image phase normally stops the existing Owner V2 container once, so its
updates and quote processing pause during that phase. An independent recovery
service must be running before the stop. The same container and image must
return healthy immediately after build termination or failure. Nine other
serving containers stay healthy; the existing research worker stays stopped.
The maximum shared image window is12 hours; this is a deadline, not an estimate.

Only after all12 images, manifest, restoration, independent recovery termination
and post-restoration work settlement pass does the isolated integration run
follow. Its driver is bounded to20 minutes, with complete recovery reporting
within60 minutes. It creates private temporary resources and does not require
another production-service interruption. A decision accepting this entire
packet covers both ordered phases without another permission question between
them. Either failure closes this attempt without an automatic retry.

The preceding Wave98 approval covered candidate899f259. Its image attempt was
completed, and its conditional integration was closed before dispatch. At preparation time, the
Owner handoff said “NO successor production authorization.” The new explicit
answer now supplies the required decision for this candidate. Root access alone
did not grant it. Deployment, production migration, new provider calls,
research-worker activation, trading, G4 backup transition and Owner acceptance
remain outside this packet.

## Evidence and prepared inputs

[Hosted evidence](2026-10-18-r2-host-ancestor-contract-ci.md): CI39109370627
attempt1 completed successfully at08:18:52UTC; functional smoke39109372364
attempt1 at08:30:55UTC. All9 full logs/8 actual checkout SHAs,1204 input pins/
523 committed blobs,131 required definitions,15 image outcomes,23 wholeflow
fixtures and all carried required groups were reconciled. Actual publication,
repeat execution,10 cleanup outcomes and owned-resource absence passed.
The nine new root-ancestor regressions ran in hosted CI. No rerun or cancellation.

The new detached checkout `/data/worktrees/3puw275b/r2-release-0120b0e` contains
2140 tracked file/symlink pins matching the candidate. The inert image env is
0600 and byte-identical to the previous prepared inert env. The accepted899
checkout's2137 pins were also rechecked unchanged. Current candidate images,
manifest, host health, protected output absence and current work remain UNKNOWN
until the approved fresh admission. Old image IDs are not successor evidence.

All paths below are relative to `target/r2-daily-beta/` and contain sanitized
metadata, source bindings and exact commands, never raw logs or credentials.

Final static reconciliation at09:00:38UTC matched all2140 source pins,11 control
pins/inverse bytes, six sequence references, inert env and prepared wholeflow
bindings: `wp-8/wave100/final-static-reconciliation.json`, SHA256
`f1bf75febb890ce0a9947989e2314659fb9175d78cd668e0c7bf76d0f3d5e574`.
It executed no project/control code and made no host reads.

| Receipt | SHA-256 |
| --- | --- |
| `ci/0120b0e/hosted-terminal-evidence.json` | `760384f837d279a8691882f6148695187966f38ef431cb38149a7bd2a9e7b73b` |
| `image-preparation-0120b0e/preparation.json` | `cae506281787bdeab99d764cafb3ffeadb3d247ce0ec8f1111dd91aa1a2b5a1b` |
| `wp-8/maintenance-0120b0e-a5/preparation.json` | `b5aa7c482585432cde5ca99c1bc29f28b41e5ea2b6a49b22142f3f30ad4cbfee` |
| `wp-8/maintenance-0120b0e-a5/command-ledger.json` | `51aaf94c79121653974f1d940825dd985be98feb19d60236b53c20bdc9eddeca` |
| `wp-8/wholeflow-0120b0e-a3/preparation.json` | `9553954ee2a26fa06c35d27c5fe84e3c81952c2db3450088b8891a5f8a158a31` |
| `wp-8/wholeflow-0120b0e-a3/command-ledger.json` | `b2049122f1c67954337f56df043fc58f8aa37cb607feb6ee76fcc2236c870d8e` |
| `wp-8/wave100/prepared-sequence.json` | `497227889887669e7ad5038d284aaa9ecee9e3b4fb31934dbbe8cc7faf6b4896` |

The11 image/recovery controls have only finite candidate/path/attempt/hash
substitutions from the prior controls. Inverse bytes, expanded argv, unit
properties and commands match; the three runtime/probe SQL files are identical.
The complete mechanical diff was reviewed. No control was installed or executed.

The prepared read-only wholeflow admission binds the accepted candidate's
root-ancestor contract using descriptor-relative no-follow traversal, root UID,
no special/group-other-write bits, no ACL, and exact dev/inode/UID/GID/mode
rechecks. Fixed data-root/content GIDs and exact service-parent/PG boundaries
remain unchanged. It also requires the new candidate's image closure and
restoration evidence. This admission preparation has static coordinator review
only; it was not run, parsed, compiled or independently reviewed. An initial
coordinator preparation syntax error occurred before evaluation and caused no
files or host commands; its correction is recorded in the private receipt.

## Ordered execution after a new explicit decision

1. Reconcile exact scope/time, all2140 source and11 control pins, inert env,
   fresh bounded host/no-clobber predicates and READ ONLY work settlement.
   Observe official plan/preflight/source-input hash afresh. Do not reuse an
   old attestation or consumed C3 wrapper. Install only the new root controls
   after admission, and verify independent recovery actually active before
   recording the single stop intent and sending normal SIGTERM. Exit0 remains
   mandatory; there is no forced-kill fallback.
2. Run the official image builder in background systemd with Nice10, idle I/O,
   CPU/I/O weights20, `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`.
   Invoke exactly one Compose service at a time in four planned batches of3.
   Between batches recover available memory/swap, OOM, health and prior process
   termination evidence. Stop on a failed gate. Recover all12 image IDs/OCI
   revisions, saved bytes/layout, source bindings and immutable manifest.
3. Recover ExecStopPost and independent recovery completion, successful actual
   terminal results and zero remaining processes. Verify the same Owner V2
   ID/image healthy, exact11 container identities/states and post-restoration
   READ ONLY settlement. A unit disappearing alone is insufficient evidence.
4. Only then perform a new five-minute bounded wholeflow admission using the
   accepted new manifest,12 exact image IDs,2140 source pins,11 critical pins,
   local PostgreSQL digest, fixed base selector, resources and11-state gates.
   Dispatch the isolated driver once. Recover actual first/subsequent admission,
   publication, HTTP, restart, capture/restore,62 migration checksums, unchanged
   base, cleanup and actual absence of every owned resource. Production11-state
   identity and current release must remain unchanged.

The image root control directory is
`/etc/lagrange/release-manifests/.r2-maintenance-0120b0e-a5-20261018`.
Build/recovery units are `lagrange-r2-build-0120b0e-a5.service` and
`lagrange-r2-recovery-0120b0e-a5.service`. The manifest path is
`/etc/lagrange/release-manifests/0120b0ea6d54198e030545c08276d257fd0da17d.manifest`.
The required Owner ID is
`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`,
image `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.

The shared image window starts at fresh root-window creation before recovery
launch. Stop is permitted only in its first hour. Build RuntimeMaxSec is10h;
the recovery watchdog stops the build at10.5h; attestation expires at11h.
There is no renewal. Wholeflow dispatch must occur by hour12 and complete
recovery reporting by hour13. Failure or uncertain cleanup is INCOMPLETE.

Wholeflow unit: `lagrange-r2-wholeflow-0120b0e-a3.service`; result:
`/var/lib/lagrange-r2-wholeflow-evidence/0120b0e-a3-20261018/result.json`.
Its a3 label is the third prepared packet; if dispatched it is the second
actual runtime attempt because899a2 never launched. Resolve the base only from
fixed container `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`'s
single read-only `/data/owner-beta-artifacts` mount. No historical-path fallback.
Use the local pinned PostgreSQL digest from the exact ledger, without pulling.
The driver has1200sec and2GiB snapshot bounds; the outer unit has50min runtime,
SIGINT,5min stop timeout and SendSIGKILL=no. Its private nonce-owned DB/network/
stages do not authorize provider/Auth0 calls or production SQL.

## Preserved history and continuation

Candidate899's12 images and restoration are accepted: closure
`d4ef8fa035e9189c364491f3e11845218dd81aa492be3213068fa8aecb05640a`,
manifest `1414f4e845a9358bedd72e8ae36a68dff68bc4c5f9704124933ad3866b350d86`.
Its conditional wholeflow admission blocked only the root-owned service-group
ancestor before dispatch: closure
`ee15d94e829bfc9ba6f04c9d5747c8a423862b4437f43829597b886bb2cf720e`.
This is not a failed runtime attempt and proves no later phase. Existing2a
images, its failed root-Git wholeflow, both ab48 image failures, all units/root
controls and the unaccepted image remain preserved. No host chmod/chgrp or
hardcoded982 bypass was introduced.

Same-agent heartbeat81b4d3b6 was confirmed at08:56:43.844UTC. Its bounded
handoff was persisted before bf7d2b09 deletion was confirmed. Renew by20:00UTC,
before20:56:43.841UTC expiry or the47th tick. Native Ralph remains blocked;
the heartbeat does not protect a daemon outage. No additional delegation.
Until a new decision, keep one question pending and perform only bounded
no-change reconciliation; no repeated production probes or documentation commits.

This packet does not complete R2. Actual current-candidate images and approved
base wholeflow, G4 consistent pre-upgrade recovery/backup transition, deployment,
Owner use, three consecutive trading days and finalC_R2 remain open. Continue
tracking them after this package rather than ending the overall objective.

## Wave100 explicit approval and fifth image dispatch — 2026-10-18

Owner의 “승인하니 이어서 작업 진행해”로 Wave100의 두 순차 단계가 승인됐다. 제5차 이미지 빌드를09:25:19UTC 정확히1회 dispatch했고, 독립 복구를 먼저 가동한 뒤 Owner V2 정상 exit0·읽기 전용 정산을 확인했다.09:33:36UTC 빌드와 복구가 실행 중이며 서비스5개 성공 기록·여섯 번째 recommendation-runner 진행을 관측했다. 전체12이미지·저장 바이트·manifest·동일ID복귀·복구 실제 종료를 인수한 뒤 이미 승인된 격리 통합검증1회를 진행한다. G4·배포·Owner·연속3거래일·C_R2는 미완료다.

Authorization receipt `wp-8/wave100/owner-authorization.json` SHA256
`3e9789ce509182313eda81243e0a0735eb6eef6c8f502679411b9b311e24696c` covers
both phases once; no repeat approval between them, no automatic retry.
Exact shared image window09:24:53.914836–21:24:53.914836UTC, independent recovery
active before stop. Build launch receipt `c765dcc8…`; corrected fresh admission
`ba91d079…`, READ ONLY work settlement `b652d3a5…`, official plan/preflight
`1d05ed9c…`. All2140 source and11 control pins matched. First coordinator-only
admission failure `18240649…` is preserved: normal waiting backup timers were
misclassified, while paired services had no processes. Only that coordinator
classification was corrected; no timer configuration or product guard changed.

Observation `build-observation-20261018T093336Z.json` SHA256
`69834339df3ecad0ed729405cb5efd88d1ef1091e289c17fe0450116b8756b2e`
shows active build/recovery, five success markers and recommendation-runner in
progress. These markers are not complete image acceptance. Manifest, all saved
bytes and actual restoration/recovery terminal outcomes remain pending.
Heartbeat179ec5a7 confirmed; handoff459bb290 persisted before predecessor81b4d3b6
successful deletion. Renew by20:15UTC. No new worker; native Ralph stays blocked.
