# R2 image and conditional approved-base integration packet

R2-04/R2-09, Wave98. Candidate899f259f9c8872fb61f0c95745904f552c8750e9.
Status: **AUTHORIZED_IMAGE_ATTEMPT_RUNNING; CONDITIONAL_WHOLEFLOW_AUTHORIZED_NOT_YET_EXECUTED**.

Owner answered “이어서 진행해” after this concrete two-phase decision request.
Authorization was recorded2026-10-18T07:10:56.169455UTC. Both phases are approved
within this packet; do not ask again between accepted image restoration and
the conditional isolated wholeflow attempt. Earlier preparation flags below
are historical, not the current authorization state.

The candidate has complete [hosted acceptance](2026-10-18-r2-wholeflow-git-ci.md):
CI39098366992 and functional smoke39098368916 attempt1 SUCCESS;9full log
hashes/8actual checkouts/1192pins/521blobs,122required definitions,15image
outcomes and the actual publication/repeat/ten cleanup outcomes were recovered.
Terminal evidence SHA256:
c87097708cf803ce69ec0e6d63bfe2d4242e20c1d084e489e6da67c8fbd0d31b.

## One approved decision covering two ordered attempts

Owner approved one new image-maintenance attempt for this exact candidate, including
fresh bounded host/read-only work admission,official plan/preflight,independent
recovery installation,one normal SIGTERM of the original Owner V2 container,
the official serial twelve-image build and same-container restoration.

Only if all image evidence and restoration are accepted, the same decision
also authorizes one new isolated approved-base host integration attempt.
It uses the real approved historical base plus two explicitly synthetic
sessions and a private database. It does not interrupt production services.
No further permission question is needed between these two explicitly approved
phases; any failed prerequisite stops the second phase. No automatic retry or
window renewal is included.

The three earlier image permissions and first wholeflow permission each covered
one attempt and are consumed. The third image attempt succeeded for2a165fe;
its first wholeflow attempt failed before functional phases at the Git ownership
boundary. The899f259 correction passed four actual-root ownership cases and
the complete23-case wholeflow fixture. This evidence supports a new attempt;
later host phases have not yet been proven.

Owner V2 daily/signal updates and existing quote collection pause during the
image phase. Restoring the same container resumes its existing behavior.
Research-worker stays stopped; the other nine serving containers must remain
healthy. Wholeflow may start only after ten serving containers,including the
same original Owner V2,are healthy and post-restoration work settles.

## Exact prepared bindings

| Binding | Value |
| --- | --- |
| Clean detached source | /data/worktrees/3puw275b/r2-release-899f259 |
| Source inventory | 2137 tracked file/symlink bytes and Git blob/mode identities |
| Inert env | target/r2-daily-beta/image-preparation-899f259/image-only.env;0600,byte-identical to the prior inert input |
| Image controls | target/r2-daily-beta/wp-8/maintenance-899f259-a4/ |
| New root controls | /etc/lagrange/release-manifests/.r2-maintenance-899f259-a4-20261018 |
| Image unit | lagrange-r2-build-899f259-a4.service |
| Independent recovery unit | lagrange-r2-recovery-899f259-a4.service |
| New manifest | /etc/lagrange/release-manifests/899f259f9c8872fb61f0c95745904f552c8750e9.manifest |
| Wholeflow unit | lagrange-r2-wholeflow-899f259-a2.service |
| New root result | /var/lib/lagrange-r2-wholeflow-evidence/899f259-a2-20261018/result.json |
| Owner V2 container | f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4 |
| Owner V2 image | sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50 |
| Approved-base selector | owner-beta-runner container1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5,one read-only bind at /data/owner-beta-artifacts |
| Isolated PostgreSQL | local postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a;no pull |

At preparation, image IDs,manifest hash,protected output/unit absence,host
health and work settlement were UNKNOWN. The fresh admission below now supplies
protected output/unit absence,host health and work settlement; final image IDs
and manifest remain pending. A new actual image closure must
bind all twelve IDs/revisions/saved bytes/layout and the immutable manifest.
Old2a165fe images or manifest are not substituted. The base physical path is
bound only from the fresh exact selector observation; the historical path is
not reused as a current observation.

## Bounds and execution gates

| Bound | Rule |
| --- | --- |
| Image maintenance window | One shared12hours,recorded before independent recovery starts |
| Normal shutdown | Within the first hour;exact Owner V2 normal exit0 required |
| Image build | Runtime10hours;watchdog requests stop at hour10.5;attestation expires hour11 |
| Resources | Background systemd,Nice10,idle I/O,CPU/IO weights20;one Compose service per invocation,Compose parallel1/Cargo jobs2;four batches of3 |
| Image completion/failure | Restore immediately after actual build/compiler termination;prove same ID/image healthy and independent recovery successful terminal |
| Wholeflow admission | Fresh5minutes after accepted image/restoration closure;no active compiler/build,healthy exact containers and resource gates |
| Wholeflow execution | One dispatch,driver1200seconds,file snapshot limit2147483648bytes;outer Runtime50min,SIGINT,TimeoutStop5min,SendSIGKILL=no |
| Wholeflow completion | Full evidence within60minutes of dispatch;start no later than image-window hour12,overall reporting by hour13;no renewal |

Fresh source/env/control pins,root access,protected parents,no-clobber outputs,
resource/OOM/compiler state and read-only runtime/aggregate SQL settlement are
mandatory before image action. Official plan/preflight must establish the
actual current source-input hash. No old attestation/hash or C3 wrapper is
reused. Independent recovery must be actually active before stop intent.
Normal exit0,settled work and all official resource/health/termination guards
remain intact. Any mismatch stops the attempt; uncertain recovery is INCOMPLETE.

Wholeflow additionally requires the accepted image manifest/IDs freshly
re-read,the exact fixed base selector/safe metadata,at least4GiB available
memory,10GiB free below /var/lib,PSI some/full avg10 at most1.0 and zero OOM
events in a readable preceding30-minute kernel interval. This bounded interval
is not full-history evidence. Historical active-exited units are judged by
actual PIDs/cgroup processes; they are never reset merely to pass admission.

The unchanged wholeflow driver creates only nonce-owned private containers,
internal networks and a root-private stage. Each container is limited to1GiB,
one CPU and256PIDs,with read-only root,dropped capabilities and no-new-privileges.
The producer has network none; no published ports,image pulls,provider or
Auth0 calls are included. SQL/bootstrap/migration apply only to the private DB.
Actual base bytes must remain unchanged. Recover first/subsequent admissions,
publication,real HTTP results,restart replay,private file/DB capture and restore,
all62 migration checksums,cleanup and actual absence of exact owned resources.
Exit0 or healthy alone is insufficient.

If either phase fails, preserve its unit,state,intents and finite diagnostics.
Do not relaunch an old unit or remove foreign/uncertain resources. Raw command
logs remain in memory; persist sanitized metadata,hashes and actual outcomes.
The image recovery service and wholeflow cleanup survive a chat disconnect;
neither guarantees recovery during a host outage.

## Preparation evidence

| Private receipt | SHA256 |
| --- | --- |
| image-preparation-899f259/preparation.json | 9049fd7571658f979fbd9fa430981c54f0e4bd5154a6ed60ee22348189dcbc04 |
| maintenance-899f259-a4/preparation.json | b8608270cc6316ceae190eb53f7a06a261b7036e981b61e80b56dcaa0dd854be |
| maintenance-899f259-a4/command-ledger.json | d9a5410f25cb8ff19ee41690d56c4d38aadbd5925eccd5363de726ef4ec41816 |
| wholeflow-899f259-a2/preparation.json | e0caa57840b057af10de8a3554ed6b62319b3bd9d72bbf8bf940bef5518578a3 |
| wholeflow-899f259-a2/command-ledger.json | e672d9ac4e2ff6a63254673428fffd720881f6743ad7eb9698ac793d09d02ad6 |
| wave98/prepared-sequence.json | 3a6adae3ebabf9082f3e5d7d44f94c953cfa5eccd32f47d396be2db0b9bb71c6 |

All receipt paths are under target/r2-daily-beta;maintenance,wholeflow and wave98
are under wp-8. Eleven controls are finite candidate/path/unit/hash substitutions
from the accepted preceding controls. All inverse bytes match;three runtime/
probe SQL files are byte-identical. The203-line diff and expanded argv/service
properties were reviewed. The wholeflow ledger pins eleven critical source
paths and the2137-path source receipt. No new independent review or local
project test/parser/helper execution is claimed. No root control installation,
host probe,SQL,Docker lifecycle or provider request occurred in preparation.

Production deployment,migration,research-worker activation,new provider scope,
root-policy changes and trading remain outside this packet. The observed
sudo lease expiry2026-10-20T17:00:57Z grants access,not maintenance consent.
G4 consistent pre-upgrade recovery/backup-version transition/deployment,actual
Owner use,three consecutive trading days and final C_R2 remain open.
Native Ralph is blocked with no agent resume API; bounded same-agent
continuation remains responsible for the overall task,without delegation.


Continuation: same-agent heartbeat5f3cfda2 confirmed2026-10-18T07:11:51.874Z.
The persisted authorized successor handoff preceded confirmedfc826f21 deletion.
Renew by18:10UTC,before19:11:51.871UTC expiry and47thtick. Native Ralph remains
blocked; this heartbeat does not protect daemon outage. The actual independent
root recovery service protects this admitted image attempt.


## Actual authorization and first build observation — 2026-10-18

Owner의 새 “이어서 진행해” 지시로 Wave98 두 단계 각1회를 승인했다. 후보899f259의2137소스pin·11제어파일·fresh 호스트/READ ONLY 정산·공식 plan/preflight를 대조하고 독립 복구를 가동한 뒤07:18:07.982728UTC 이미지 빌드를 정확히 한 번 시작했다.07:19:15UTC Owner V2 정상종료0·작업 정리·DB 이미지2개 성공과 API 이미지 진행을 관측했다. 전체12이미지/manifest·같은 컨테이너 복귀·복구 정상 종료는 회수 중이며, 모두 인수한 뒤 이미 승인된 격리 통합 검증1회를 이어간다. G4·운영 배포·Owner·연속3거래일·C_R2는 미완료다.

Fresh root read-only admission07:15:58UTC found the exact eleven identities:
ten serving containers healthy/restart0/no OOM, research stopped2/restarts7935.
No actual compiler/build or producer unit execution was observed. Historical
active-exited units had zero PIDs/cgroup processes and were not reset. Available
memory11171700KiB,swapfree824552KiB,PSI some/full0.0;the readable preceding
30-minute kernel range contained zero rows/OOM. This is not full-history proof.
Current remained releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c.
READ ONLY runtime/aggregate settlement passed with zero running jobs/attempts.
Official plan bound all12services and freshly observed source-input SHA
454cad1f9cb8f12ddafca6729897acc9ef2eccd5947adf40942020b1e9a1cf76;preflight passed.

The shared root window is07:17:15.544164–19:17:15.544164UTC. Normal stop must
start by08:17:15.544164;watchdog17:47:15.544164;attestation18:17:15.544164.
No renewal. Wholeflow can dispatch only after accepted closure and no later
than19:17:15.544164;full recovery within60minutes and by20:17:15.544164UTC.
Independent recovery was actually active with its startup receipt before the
single build dispatch07:18:07.982728UTC. The normal stop exited0 and the
post-stop work probe passed. At07:19:15UTC build/recovery were active/running,
DB-role-bootstrap and DB-migrate emitted success and api-server had started.
Manifest was absent. These markers do not establish full image acceptance.

Private receipts under target/r2-daily-beta/wp-8:

| Receipt | SHA256 |
| --- | --- |
| wave98/owner-authorization.json | acca88680165af54c38fdd775d51c0b3e10fd80974151ba06c23e11369c84145 |
| maintenance-899f259-a4/authorized-static-reconciliation.json | 4aa30c3403a83ab934921ea98018fd04a71a1943a93171c36692152cea16ed37 |
| maintenance-899f259-a4/authorized-host-admission.json | 0a314abd888b0717a2fa9095dd54e086d99d9fae2e6e7059b5bfa9e9951ebed6 |
| maintenance-899f259-a4/authorized-prelaunch-settlement.json | 91955a77f83c0f7b4b4a7221be7f844696e278ea071e9918cbdd31c12188f498 |
| maintenance-899f259-a4/official-prelaunch-evidence.json | e1ccd63789f85bcf7c6afc11713f403800ca44d687399fd8fde19e6f9b9e807f |
| maintenance-899f259-a4/control-installation.json | 9fbcabd58134110062ba8e400704e6faddd02262f7495bae3555926a209d46ae |
| maintenance-899f259-a4/recovery-launch.json | f09bc8834a9432f30866c6c35b62a7a916614463e99aea088136643bab4b192a |
| maintenance-899f259-a4/recovery-ready.json | f92d995c8327d8317c2f395e4d849ac29363a9d917790739cf955f6f1f472562 |
| maintenance-899f259-a4/build-launch.json | d1c65116875fbd94cf5309356f30db5965cb27bdd1cc30b2ff093510a00bedea |
| maintenance-899f259-a4/initial-build-observation.json | 82ed9da816867568654a4cc08074406e7aba8fc11cd26139a0572d2c76628fd3 |

Full twelve-image saved-byte/layout/OCI/manifest recovery, same-ID restoration,
actual successful build/recovery terminal and postrestore settlement remain
required. Then execute the already-authorized conditional wholeflow once.
Source and accepted hosted evidence remain unchanged. No new local test or
independent-review result is claimed; approved official host commands are
recorded above. Production deployment/G4/Owner/three trading days/finalC_R2
remain open and outside this packet.
