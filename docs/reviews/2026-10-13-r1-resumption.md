# R1 재개 — main 실행 회수와 C3 빌드 전제

2026-10-13 KST. 대상 작업은 **R1-09**이며 검증·빌드 후보는
`862fa954c16d28a617271fe6f613d34226e2d558`(C3)로 유지한다.
**R1-09와 전체 R1 DONE — 2026-10-13 KST.** 후속 Owner 승인에 따른 실제 C3 이미지 12개·immutable manifest와 기존 Owner V2 복귀·작업 정산을 [최종 검증](2026-10-13-r1-c3-images.md)한 뒤 완료 처리했다.
R2와 개발 속도 개선 5개 항목은 착수하지 않았다.

## 인계와 최신 main 대조

`git fetch origin main` 후 `origin/main`과 작업 HEAD는 모두
`b0a2579bef472ae6b604405d0ab6f50255786dd4`였고 작업 트리는 깨끗했다.
인계의 `b0a2579` 이후 원격 변경은 없었다. C3와 main 사이 차이는
ROADMAP·STATUS·인수·실행·배포 준비·계획의 문서 6개뿐이다.
main의 문서 커밋을 C3 대신 이미지 revision으로 사용하지 않는다.

## 기존 main CI 회수

기존 [CI 38336666454](https://github.com/sihioov/Lagrange/actions/runs/38336666454)와
[smoke 38336666468](https://github.com/sihioov/Lagrange/actions/runs/38336666468)는
`push/main`, attempt 1, head `b0a2579bef472ae6b604405d0ab6f50255786dd4`다.
둘 다 02:23:38 UTC에 생성됐다. **CI는 02:44:34 UTC (11:44:34 KST) completed/success**이며
`format`, `clippy`, `policy`, `workspace-tests`, `python-tests`, `web`,
`postgres-integration-validation`, `required` 8개 job이 모두 성공했다.
**별도 smoke도 03:16:46 UTC (12:16:46 KST) completed/success**다.
smoke job은 02:23:41~03:16:46 UTC에 실행됐고 모든 step이 성공했다.
새 dispatch·rerun·cancel은 수행하지 않았다. 기존 C3의 통과 근거는
[C3 인수 기록](2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance)에 별도로 보존한다.

코디네이터는 8개 CI job의 원본 job-log API 응답을 메모리에서 hash하고,
checkout을 수행한 7개 job의 실제 `git log -1 --format=%H` 바로 다음 출력이
모두 main SHA인지 직접 대조했다. `required`는 checkout 없는 집계 job이다.
workspace 명령의 출력 235개 summary 합계는 2,886 pass·0 fail·6 ignored·180 filtered이며
별도 chart HTTP 6개와 구분했다. 이는 고유 테스트 개수가 아니다.
Python은 482 pass·10 skip·7 warning, DB는 `APPROVED`, Web wrapper는
`state=passed exit=0 coverage=complete`를 실제 출력했다. 실행되지 않는 DB 실패 진단 step의
조건부 skip은 검증 step 실패나 DB 테스트 skip으로 세지 않는다.
기존 ignored/skip의 source 근거와 hosted 세부 출력 한계는 C3 기록에 보존하며
이 main run이 출력하지 않은 사유·브라우저별 횟수·운영 결과는 새 관측으로 주장하지 않는다.

smoke 원본 job-log API도 별도로 대조했다. `git log -1 --format=%H`와 바로 다음
main SHA 출력은 117~118행, 실제 `RESEARCH_WORKER_SMOKE: functional PASS`는
1722행에 있다. CI 7개와 smoke 1개를 합친 실제 checkout 8개가 모두 main과 일치한다.
smoke 원본 응답은 139,065 bytes, SHA-256
`14ea3ee4d9bb8c29fe8905fd69e99c8bf8e875826dc2aedcafcaaf674891dbf7`이며
raw log를 저장하지 않고 hash·고정 필드만 보존했다. worker의 `gh run view --log` 출력은
포맷이 달라 별도 hash를 가지므로 원본 API bytes와 혼용하지 않는다.
기능 PASS는 개별 cleanup receipt나 운영 이미지·manifest 검증을 대신하지 않는다.
이 두 기존 실행의 최종 결과 회수에는 미해결 항목이 없다.

## 첫 재개 당시 root·서비스·자원 관측 — 11:38 KST

**2026-10-13 02:38:25~26 UTC (11:38:25~26 KST)**에 고정 필드만 읽었다.
Docker env·provider 로그·응답 본문·운영 DB·비밀 파일은 읽지 않았다.

| 조건 | 새 관측 | 판정과 한계 |
| --- | --- | --- |
| root 실행 | UID 1000; `sudo -n stat -c '%a %U:%G %F %n' /etc/lagrange/release-manifests` exit 1, `interactive authentication is required` | root 조건 미충족. 보호된 output/state 부모 신뢰와 C3 manifest 부재는 미확인 |
| control units | Docker·containerd 모두 loaded/active/running, ExecMainStatus 0, NRestarts 0 | 관측 당시 두 unit의 고정 필드만 확인 |
| build unit | `lagrange-r1-build-862fa954.service` not-found/inactive/dead, MainPID 0 | 이 작업은 build unit을 시작하지 않음 |
| serving inventory | 지정한 11개 모두 조회 성공. research-worker 외 10개는 running/healthy, exit 0, restart 0, OOMKilled=false | ordinary container predicate는 research-worker에서 미충족 |
| research-worker | exited, exit 2, unhealthy, restart 7935, OOMKilled=false. StartedAt `2026-09-19T03:45:11.883708031Z`, FinishedAt `2026-09-19T03:45:12.078101835Z` | 과거 보고를 재사용하지 않고 현재 Docker 필드를 읽음. 종료 원인·의도된 설치 모드는 판정하지 않음 |
| Owner V2 | running/healthy, exit 0, restart 0, OOMKilled=false | 두 reader가 모두 정지해야 하는 기존 drained-reader 계약은 성립하지 않음 |
| memory / swap / PSI | MemAvailable 6,432,112 KiB; SwapFree 34,348 KiB; PSI some/full avg10 각각 0.00 | 같은 C3의 resource-policy 함수에 이 관측을 넣은 결과 stop reason 없음. SwapFree는 기록 값이며 독립 중단 임계값으로 바꾸지 않음 |
| disk / compiler | `/data` available 602,886,848,512 bytes; `ps -eo pid=,comm=`에서 cargo/rustc/rustdoc 없음 | 시점 관측이며 build lock·다른 Docker 작업 부재의 증명은 아님 |
| kernel journal | 현재 boot의 kernel probe 1개 읽기 성공. 02:08:25~02:38:25 UTC range 0개, OOM pattern 0개, 명령 exit 0 | 이 30분 구간만 확인. 공식 persistent gate의 origin·전체 build 수명 검사는 미실행 |
| C3 이미지 | 공식 서비스 12개의 exact C3 tag를 각각 inspect; 전부 `No such image`, exit 1 | C3 local image ID·saved-byte receipt 없음. 보호된 manifest 부재 확인으로 확대하지 않음 |
| current link | `releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c` | link target만 확인. 운영 env·DB·설치 모드·복구 호환성 검증 아님 |

02:38:56 UTC의 보충 관측에서 설치된 `lagrange-kis*.timer` 10개는 모두
disabled/inactive/dead였다. current-link revision의
`lagrange-kis-daily-fce9b90.service`도 disabled/inactive/dead, MainPID 0이었다.
프로젝트 inventory 11개 중 Compose oneoff label이 true인 container는 없었다.
이것은 향후 timer/one-off 동시 실행 부재를 보장하지 않는다. 새 build 직전과 종료 후에
공식 runbook의 해당 검사를 다시 해야 한다.

daily 모드에서는 상주 research-worker 정지가 정상일 수 있다는
[서비스 상태 계약](../contracts/service-status-v1.md)과
[C3 공식 빌드 계약](../runbooks/production-release-and-backup.md)은 구별한다.
이번 관측은 종료 원인이나 운영 장애의 증거가 아니다. 동시에 C3 builder의 exact 11개
ordinary health 조건에서 임의로 research-worker를 제외할 근거도 아니다.
과거 running-research 예외나 drained-reader attestation을 읽거나 재사용·갱신하지 않았다.

## 승인 범위 안에서 마친 준비

- 보존된 detached C3 checkout과 inert env 경로·hash·모드를 재검증했다.
  checkout/env 조상에 symlink가 없고, env parent UID 1000/mode 0700,
  regular file UID 1000/mode 0600 및 hash `df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6`가 유지됐다.
- 정확한 C3의 공식 `--plan` → `--preflight` → `--plan`은 모두 exit 0이었다.
  앞뒤 source-input hash는 모두
  `78139d0cbf60a0d82e0cf040997bf6d31bc4ee670adcd2b0b7ce8c761d9b0b8c`다.
  manifest 인자를 생략한 read-only 검사이므로 protected output 검증을 포함하지 않는다.
- 인계의 `proposed-build.sh`는 `bash -n` exit 0이고 기존 hash와 일치했다.
  이 파일은 실행하지 않았다. 인계 `evidence.sha256`의 14개 entry를 worker와
  코디네이터가 각각 대조했다. 입력·source checkout은 전후 clean이었다.
- [고정 명령과 순차 batch](2026-10-12-r1-release-readiness.md#조건-해소-후-사용할-image-only-명령),
  migration·backup·rollback 소스 검토는 그대로 유효하다. 운영 검증으로 승격하지 않는다.

## 다음에 필요한 조치와 영향

1. **root 전제 해소:** 후속 Owner sudo 적용 뒤 UID 0·만료 timer·보호된 부모·state 경로와
   C3 manifest 부재를 아래 근거로 확인했다. 실행 직전 fresh admission을 다시 확인한다.
   암호·보호 경로 권한은 변경하지 않았고 image-only 작업은 이미 승인돼 있다.
2. **health 조건에 대한 별도 운영 결정:** 현재 ordinary 조건은 미충족이고 기존 stopped-reader
   대안도 미충족이다. 기존 대안을 택하려면 실행 중인
   `lagrange-station-owner-equity-v2-runner-1`의 처리 중 작업과 안전한 정지 가능성을 먼저 확인하고,
   정상 exit 0으로 정지한 뒤 이미 exited 2인 research-worker와 함께 새 C3 전용 attestation을
   승인된 유효기간으로 만들어야 한다. 이 조치는 Owner V2 수집·갱신 작업을 일시 중단시키며
   실행 중 작업 처리·재개 시점과 방법까지 결정해야 한다. 현재 요청은 이를 승인하지 않았다.
   research-worker를 health 때문에 시작하면 데이터 수집을 유발할 수 있으므로 시작하지 않는다.
3. **조건 충족 후 실제 빌드:** Cargo jobs 2·Compose parallel 1·Nice 10·idle I/O·CPU/IO weight 20을
   유지한 background systemd에서 공식 builder를 실행한다. 3서비스씩 4개 보고 batch 안에서도
   Compose는 한 번에 서비스 하나만 빌드한다. checkpoint마다 자원·OOM·이전 exit·compiler·health를
   확인하고 12개 image ID·revision·saved bytes와 공식 V2 manifest가 일치해야 R1-09를 완료한다.

승인 범위의 준비는 마쳤다. 실제 `--apply`, 서비스 start/stop/restart, 운영 배포·migration,
외부 데이터 API 호출은 수행하지 않았다. root와 health 조건을 우회하는 사용자 권한 prebuild나
수동 manifest 작성도 하지 않았다.

## 후속 Owner 요청 — 직접 실행할 72시간 sudo 설정 준비

Owner가 약 72시간의 sudo 권한 설정 스크립트를 요청하고 직접 실행하기로 했다.
준비한 파일은
`/data/worktrees/3puw275b/excited-dolphin/target/r1-resume-20261013/sudo72/enable-sudo-72h.sh`이며
SHA-256은 `11e91cfe73fa0c9fda9a3757e1e8364356bed2fe17b1115516c1b653f0cc0e01`이다.
이 파일은 제품 소스·C3 이미지 입력에 포함하지 않은 운영자용 보조 파일이다.

- 적용 대상은 `l1nnx`(UID 1000) 계정 전체의 passwordless sudo `(root)`다.
  기존 sudoers 파일을 덮어쓰지 않고 전용 파일과 만료용 systemd unit만 추가한다.
- 현재 설치된 sudo-rs `0.2.13-0ubuntu1.2`의 실제 visudo는 `NOTAFTER`를 거부했다.
  따라서 72시간 뒤 실행할 UTC calendar·monotonic timer를 먼저 등록·검증한 뒤 권한을 설정한다.
  Persistent timer가 중단 중 놓친 calendar 시각을 다시 처리한다. 호스트 정지·시간 변경·
  실행 지연까지 막는 절대적인 72시간 보장은 아니며 이미 시작한 root 프로세스를 종료하지 않는다.
- 만료 시 권한 파일을 권한 없는 marker로 원자 교체한다. 설치의 no-clobber publish와 결합해
  만료 후 늦게 재개된 설치가 권한을 되살리지 못하게 한다. 명시적 `revoke`는 marker와 unit을 정리한다.
- 실제 sudoers·만료 marker parser, 생성 unit·calendar 및 Bash 구문 검사와
  SIGKILL 두 경우를 포함한 비권한 격리 회귀 14개가 통과했다.
  Paseo `8689f94c-98ba-48dd-841d-ccfaa9c60915`(Sol/high)의 소스 리뷰에서
  초기 만료/설치 경합과 cleanup 보고 문제를 보완한 최종 delta에 남은 blocker가 없었다.

**스크립트 준비 완료 당시에는 root 설치·실제 72시간 만료가 미검증이었다.** 이후 Owner의
직접 적용과 root 확인은 다음 절에 기록한다. 이 설정 요청은 운영 서비스의
정지·재시작·배포·migration·외부 데이터 API 호출 승인을 추가하지 않는다.
상세 리뷰·격리 검증 영수증은 위 파일과 같은 `sudo72/` 디렉터리에 보존했다.

## Owner sudo 적용 후 root 전제 해소

Owner가 준비한 스크립트를 직접 실행하고 성공 출력을 전달했다. 코디네이터는
**2026-10-13 05:14:50~05:16:26 UTC (14:14:50~14:16:26 KST)**에 다음을 새로 확인했다.
`git fetch origin main` 결과도 계속 `b0a2579`이며 검증·빌드 후보 C3는 변경하지 않았다.

| 확인 대상 | 새 근거와 판정 |
| --- | --- |
| 비대화형 root | `sudo -k -n /usr/bin/id -u` exit 0, UID 0. 이전 root 인증 차단 해소 |
| sudo 만료 | `lagrange-r1-sudo72-l1nnx.timer` enabled/active/waiting, Persistent=yes. 다음 calendar 만료는 **2026-10-16 05:11:48 UTC / 14:11:48 KST**. 만료 service는 아직 inactive/dead이며 실제 72시간 만료는 미검증 |
| protected 부모 | `/`, `/etc`, `/etc/lagrange`, `/etc/lagrange/release-manifests` 모두 root 소유·non-symlink directory·group/world writable 아님. manifest 부모 0755, `.lagrange-build-state` root:root 0700 |
| C3 output/state | exact C3 manifest와 해당 commit의 state directory 모두 부재. root build lock directory는 root:root 0700으로 존재하나 lock을 획득하지 않았으며 현재 점유 여부는 미검증 |
| 공식 root 검사 | 보존한 detached C3와 inert env로 manifest 인자를 포함한 `--plan → --preflight → --plan` 모두 exit 0. 앞뒤 source-input SHA-256 `78139d0c…d9b0b8c`, checkout clean. state 초기화·apply는 미실행 |
| 현재 health | 지정 11개 중 10개 running/healthy. research-worker는 exited2/unhealthy, restart 7935, OOMKilled=false; Owner V2는 running/healthy. ordinary/drained 조건 모두 미충족 |
| 자원·control | MemAvailable 4,676,104 KiB, SwapFree 83,744 KiB, memory PSI some/full avg10=0.00. 동일 C3 resource-policy 함수의 stop reason 없음. Docker/containerd active/running, compiler 목록 cargo/rustc/rustdoc 없음 |
| OOM 관측 한계 | 읽기 가능한 현재 boot kernel journal의 04:44:50~05:14:51 UTC 구간은 row 0·OOM pattern 0. 전체 build 수명 관측이나 admission을 대신하지 않음 |
| 동시 활성화 관측 | 설치된 KIS 계열 timer 10개 disabled/inactive, 현재 release의 daily service inactive/MainPID 0, project container 11개 중 one-off 없음. 미래 동시 실행 부재의 보증 아님 |
| 실제 후보 artifact | exact C3 image tag 12개 전부 부재, protected manifest 부재. 예정 build unit도 not-found/inactive/dead. 실제 이미지 빌드·manifest 생성 미실행 |

이 확인은 root 전제와 보호 경로의 읽기 전용 준비를 완료한다. fresh 공식 gate는 실제
빌드 시 다시 실행해야 하며 **R1-09/R1은 health 차단으로 미완료**다. research exit2의
원인이나 의도된 설치 모드는 이번 고정 필드 관측으로 단정하지 않는다.

### 남은 운영 결정과 영향

Paseo `9640452a-386d-40b6-8165-c2c923844963`(Sol/high)의 제한된 소스 검토를 회수하고
코디네이터가 C3의 shutdown·attestation 코드와 대조했다. worker는 idle·pending 0이며
운영 작업을 실행하지 않았다. 기존 대안은 **기존 Owner V2 컨테이너만 정상 종료(exit 0)**하고,
이미 정지한 research-worker(exit 2)를 유지한 뒤 새 C3 전용 관측 기록을 만드는 경로다.
기록 유효기간은 소스 계약상 최대 12시간이며 운영 중단 시간의 승인을 뜻하지 않는다.
빌드 중 유효기간을 갱신하거나 reader를 재개하면 기존 attempt를 계속할 수 없다.

05:19:58 UTC에 Owner V2의 Docker 고정 필드를 추가 확인했다. 기존 컨테이너 ID는
`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`, 이미지 ID는
`sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`이며 OCI revision은
`fce9b90542cf9b1a911d46e4c4d3530db7efba4c`다. 실제 설정은 SIGTERM, StopTimeout=960초,
init=true, restart=unless-stopped다. 이는 C3 배포가 아니라 현재 컨테이너의 정지·복귀 대상을
고정하는 근거다. 해당 revision→C3의 runner binary 소스 diff도 대조했으며 shutdown 분기는
변경되지 않았다. 전체 런타임 설정·현재 작업·활성 quote mode를 검증했다는 뜻은 아니다.

승인이 필요한 범위는 이 한 컨테이너의 정상 정지, 승인된 시간 안의 C3 순차 빌드와
fresh attestation, 빌드 종료·중단 후 같은 컨테이너의 재개다. 중단 중 Owner V2 일봉·신호
갱신과 활성화된 실시간 수집이 영향을 받고 재개하면 기존 수집·DB recovery가 다시 동작할 수 있다.
정지 전에 현재 작업·lease와 복구 조건을 확인해야 하며, running/healthy만으로 idle이라고
판정하지 않는다. SIGTERM은 현재 작업을 기다리는 소스 경로지만 exit 0·작업 정산을 보장하지
않는다. 실제 정상 종료·정산을 확인하지 못하면 빌드로 진행하지 않고, 강제 종료로 조건을
맞추거나 research-worker를 시작하지 않는다. 운영 중단·복귀 절차와 한도는 아직 미승인이다.

이는 최초 요청의 서비스 재시작·외부 API 호출 금지 범위를 넘어서는 운영 결정이므로
sudo 설정 성공만으로 실행하지 않는다. 운영 배포·migration·R2·개발 속도 개선은 착수하지 않았다.
상세 소스 근거와 미검증 범위는 private `root-enabled/health-action-review.md`에 보존했다.

## 보존 근거

Private fixed-field receipts:
`/data/worktrees/3puw275b/excited-dolphin/target/r1-resume-20261013/`.
이 문서의 시각·값·판정·미검증 범위가 장기 기록이며 임시 receipt만을 유일한 근거로 삼지 않는다.

| Receipt | SHA-256 |
| --- | --- |
| `host-observation.json` | `1f08d69d1f805906a5dc5aa015b70a433fdceba2063eb90a9fca2ce4e6ae1f28` |
| `timer-observation.json` | `0eaf00f89a910db0af582f6cb6a33be7282986d719051ca249d4db8755d4892a` |
| `gate-assessment.json` | `55e8881b6e8255af7bf48db345bc6c25cd4f48db926d7782d40f66e6479cb242` |
| `preparation/official-plan-preflight.json` | `31ea68592f3ac8537665485b1943203eaa1c85c100f21ef85b902ca2fae8c8d3` |
| `preparation/handoff-check.json` | `55fe04163b0aa6ae20b609749c6b6aeed3100571bbc1ea918ef799f2b7d7ee17` |
| `preparation/report.md` | `44423a50bfd706a99bb857593c36177211d6d18e6944d5e4058bcadc09060b9c` |
| `coordinator-input-check.json` | `e0aa85cc8b912162cfa6f95af8960ec08cd50030845d9fc23f30f2c47b38e2e7` |
| `coordinator-ci-evidence.json` — 원본 job-log API bytes의 hash·정확한 checkout pair | `00767a539f9511b0f80e66c0de36eb45a40aa32945fba9bd526597c98b729054` |
| `coordinator-smoke-evidence.json` — 원본 smoke log의 checkout pair·functional PASS | `d21e815fbdb1f170f0c5b272dc3cb414df71e88f3a74d0cf3c1af192b3fc01b2` |
| `ci/ci-final.json` — CI terminal run/job/step | `903ec5d0b70ba4d68864f45bad4a62fc1c156219a28071e41e0f3f66b92acd29` |
| `ci/smoke-final.json` — smoke terminal run/job/step | `5127fb9220acf01ae4726d50335d566df81139b9352ed3675601540bc1cc06dc` |
| `ci/summary.json` — 두 최종 실행의 수집 결과 | `42713732a7c69ef8a90cda1a3fff4f4cb8569549b06abff78ad31131a44b82e2` |

별도 문서 대조는 수치·hash 불일치를 찾지 못했다. 그 대조의 제한된 scope 밖이었던
C3→main 문서 6개 목록과 코디네이터의 14-entry 독립 대조는 위
`coordinator-input-check.json`에 별도로 기록했다.

Owner sudo 적용 후 receipt는 위 private 경로의 `root-enabled/`에 별도로 보존했다.

| Receipt | SHA-256 |
| --- | --- |
| `host-observation.json` | `311240b90b685f66a40b66c3cb04b5056dd991065fcf8c21c44cd5f53c1fe224` |
| `protected-paths.json` | `df3be3c6d91aa7c2b83fcb2ec6ed15b75e47a60979dd73fa7832caa254e821e5` |
| `root-plan-preflight.json` | `378bcc9b815056ae6702e986067e56ece310b6bb6a4d911110d1a743d3a6888c` |
| `activation-observation.json` | `8eef005972c41afc3cd59f877925e2bdac63ed449a97d542258631c97e1cdf22` |
| `gate-assessment.json` | `a60d606cd093bac5cf0b9292b04017d681ec8bef22721cdc0a24e81e0041e168` |
| `owner-runtime-metadata.json` | `9515fc67ea48e4e53f0d350fdd7785605a9ea7f7c087fb3d6e43eafed3c328ed` |

## 승인된 Owner V2 유지보수와 C3 빌드

Owner는 작업 정산 확인 뒤 기존 Owner V2 한 컨테이너의 정상 정지, 최대 12시간의
C3 이미지 준비, 완료·중단 후 동일 컨테이너 재개를 제안한 질문에 “진행해”라고 승인했다.
앞 절의 미승인 상태는 이 후속 결정으로 대체한다. 승인 대상은 기존 `fce9b90` 런타임의
한시적 정지·복귀이며 C3 운영 배포·migration·research 시작·새 데이터 API 범위로 확대하지 않는다.

2026-10-13 05:42:39 및 05:43:48 UTC의 반복 읽기 전용 조회에서 Owner V2 job의
queued/running 및 running attempt는 모두 0이었다. 선택된 stream의 유효 producer lease,
유효 browser consumer, pending subscription operation도 모두 0이었다. 역사 job 실패 5개와
attempt 실패 7개는 보존하며 이번 유지보수의 실패로 해석하지 않는다. 조회는 repeatable-read
READ ONLY와 operator-wide RLS 가시성 guard를 확인했고 실제 정지 전후 다시 실행한다.

05:55:06 UTC 실행 전 확인에서 detached C3·inert env hash가 일치하고 root 공식
plan/preflight가 통과했다. protected output/state/유지보수 디렉터리와 예정 unit은 아직 없었다.
MemAvailable 4,829,756 KiB, SwapFree 364 KiB, PSI some/full avg10=0.00이며 같은 C3
resource-policy의 stop reason은 없었다. cargo/rustc/rustdoc 및 최근 30분 kernel OOM 기록은
없었고 Docker/containerd·나머지 serving health, KIS timer 비활성, Compose one-off 부재를 확인했다.

운영 보조 스크립트는 root 전용 디렉터리에 helper 세 파일과 함께 설치하도록 준비했다.
21개의 격리 fault test가 통과했으며 실제 lifecycle과 이미지 완료를 뜻하지 않는다.
독립 제어 리뷰와 fresh admission 후 systemd의 pre-start가 SIGTERM으로 정상 정지하고,
공식 builder가 순차 빌드를 수행하며, post-stop이 같은 container ID·image ID의 Owner V2를
재개한다. 독립 리뷰에서 post-stop 종료 뒤 늦은 정지·compiler 잔존 시 복귀 책임이 끊길 수 있는
문제를 발견해 별도 root recovery unit을 추가했다. 시작 20분·빌드 10시간·종료 20분·
복귀 20분은 660분의 명목 phase 합계이며 hard wall-clock 보장으로 주장하지 않는다.
독립 recovery와 drain은 recovery unit 시작 전 기록한 같은 12시간 window를 소비한다.
stop은 그 window의 첫 1시간 안에서만 허용하며 arming 직전 recovery active를 확인한다.
recovery는 intent가 없어도 원래 deadline 전에는 퇴장하지 않고, 필요하면
10시간 30분에 해당 빌드 unit만 중단해 복귀 시간을 확보한다. 늦은 정지를 직접 관측한
기록도 restore child 호출 전에 보존해 child timeout 뒤 복귀 완료를 다시 판단할 수 있다. Docker stop의
강제 kill 대체 동작은 사용하지 않으며 실패·복귀 미완료는 별도 실패로 기록한다.
attestation은 시작 기준 11시간까지만 유효하고 실행 도중 갱신하지 않는다.

**실제 실행 시작:** 최종 제어 SHA `5d3e83b5…a11c79`와 recovery SHA `12b47417…04840`에
대한 독립 소스 리뷰가 차단 사항 없음으로 끝났고, 같은 bytes의 격리 테스트는 각각
24개·16개가 별도 실행에서 통과했다. 이전 설치본은 보존하고 미실행 제어 파일만 교체했다.
06:22:43 UTC fresh root plan/preflight·입력 hash·자원·서비스 검사가 다시 통과했다.

독립 recovery unit을 먼저 활성화하고 startup receipt를 확인한 뒤 공식 build unit을 시작했다.
shared window는 **06:23:55.967950~18:23:55.967950 UTC**이며 attestation 만료는
17:23:55.967950 UTC다. Owner V2는 **06:24:01.999737018 UTC exit 0**으로 정상 정지했고
정지 전후 running job/attempt, 정지 후 유효 producer lease·epoch·pending subscription은 모두
0이었다. research의 기존 stopped 상태와 나머지 serving identity는 유지됐다.
06:24:02.382965 UTC의 fresh attestation 이후 실제 C3 공식 빌드가 시작됐다.
서비스 한 개씩 Cargo jobs2·Compose parallel1, Nice10·idle I/O를 실제 unit 속성과 대조했다.
이 실행 시작 시점에는 완료된 이미지·최종 manifest·Owner V2 복귀가 후속 검증 대상이었다.
private 영수증은 `target/r1-resume-20261013/maintenance/`에 보존한다.

## 실제 이미지 검증과 R1 완료

07:10:57 UTC 공식 builder가 12개 이미지·V2 manifest PASS를 출력했다. 서비스별 순차 실행과
49개 gate(배치 B1~B4·final-bytes 포함)가 모두 통과했고 gate OOM은 0이었다.
manifest SHA-256 `041fdaf1ddfe8746df3e4eacf3a180435a0cd2364952e3036da8c3e0e4ccb64f`,
root:root 0600, canonical 14행을 확인하고 실제 tag ID·revision·saved-byte 기록과 대조했다.
07:11:03 같은 Owner V2 컨테이너가 healthy로 복귀하고 systemd-run wait exit 0을 회수했으며
독립 recovery도 07:11:08 복귀를 확인했다. 07:11:55의 별도 runtime·READ ONLY work 조회는
원래 운영 identity·current link 유지, 작업·stream 미정산 0을 확인했다.

[최종 이미지·manifest·복귀 기록](2026-10-13-r1-c3-images.md)에 시각·ID·hash·관측 범위와
검증 한계를 보존한다. 앞 절의 차단·미실행 상태는 각 과거 시점의 기록이며 이 결과로 대체한다.
동일 C3의 필수 CI·smoke와 migration/복구 소스 검토까지 연결해 WP-7/R1-09 및 전체 R1을 완료했다.
R2, 운영 C3 설치·migration·새 provider 범위, 개발 속도 개선 5개 항목은 착수하지 않았다.
