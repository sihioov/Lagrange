# R1 재개 — main 실행 회수와 C3 빌드 전제

2026-10-13 KST. 대상 작업은 **R1-09**이며 검증·빌드 후보는
`862fa954c16d28a617271fe6f613d34226e2d558`(C3)로 유지한다.
실제 이미지·immutable manifest 근거가 없어 R1-09와 전체 R1은 **BLOCKED**다.
R2와 개발 속도 개선 5개 항목은 착수하지 않았다.

## 인계와 최신 main 대조

`git fetch origin main` 후 `origin/main`과 작업 HEAD는 모두
`b0a2579bef472ae6b604405d0ab6f50255786dd4`였고 작업 트리는 깨끗했다.
인계의 `b0a2579` 이후 원격 변경은 없었다. C3와 main 사이 차이는
ROADMAP·STATUS·인수·실행·배포 준비·계획의 문서 6개뿐이다.
main의 문서 커밋을 C3 대신 이미지 revision으로 사용하지 않는다.

## 기존 main CI 회수

회수 중: 기존 [CI 38336666454](https://github.com/sihioov/Lagrange/actions/runs/38336666454)와
[smoke 38336666468](https://github.com/sihioov/Lagrange/actions/runs/38336666468)는
`push/main`, attempt 1, head `b0a2579bef472ae6b604405d0ab6f50255786dd4`다.
둘 다 02:23:38 UTC에 생성됐다. 이 절은 terminal 결과를 회수한 뒤 확정한다.
새 dispatch·rerun·cancel은 수행하지 않았다. 기존 C3의 통과 근거는
[C3 인수 기록](2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance)에 별도로 보존한다.

## 현재 root·서비스·자원 관측

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

1. **root 실행 가능한 운영자/세션:** C3 manifest의 no-clobber 출력과 보호된 부모·state 경로를
   확인하고, fresh admission이 충족될 때 보존된 exact-C3 공식 명령을 실행한다.
   password·sudo 정책·경로 권한 변경으로 대체하지 않는다. image-only 작업은 이미 승인돼 있다.
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

별도 문서 대조는 수치·hash 불일치를 찾지 못했다. 그 대조의 제한된 scope 밖이었던
C3→main 문서 6개 목록과 코디네이터의 14-entry 독립 대조는 위
`coordinator-input-check.json`에 별도로 기록했다.
