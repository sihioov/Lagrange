# R1 C3 이미지 빌드 인계

상태: **준비 완료, 실제 빌드는 외부 전제에 BLOCKED.** 2026-10-13 KST.
검증 후보는 `862fa954c16d28a617271fe6f613d34226e2d558`(C3)다.
[동일 커밋 CI·기능 smoke](2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance)는
통과했다. 기존 image-only 범위의 구체적인 인계를 완료했으며 실제 C3 이미지 ID와
immutable manifest는 아직 없다. R1-09와 전체 R1은 미완료다.

2026-10-13 `1fe09ef`까지의 R1 작업을 main에 fast-forward로 통합하고 진행 문서를 갱신했다.
[통합 기록](2026-10-12-r1-acceptance.md#main-integration--2026-10-13)과 별개로 이 인계의
실제 빌드 대상은 검증된 C3 그대로다. 아래 운영 관측은 기록된 시점의 증거이며
main 통합에서 root·health 조건을 새로 검증하거나 해소한 것은 아니다.

## 고정한 입력과 완료된 준비

| 항목 | 고정 값·검증 |
| --- | --- |
| 빌드 소스 | `/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/release-c3-worktree` — detached HEAD C3, 실행 전후 clean |
| 공식 source-input SHA-256 | `78139d0cbf60a0d82e0cf040997bf6d31bc4ee670adcd2b0b7ce8c761d9b0b8c` |
| 입력 동일성 | release/core 12개·SQL 118개, 총 130개 이전 검토 입력과 내용 동일. 전체 825개 source/mode record 실행 전후 동일 |
| 공식 plan / preflight | detached C3에서 각각 exit 0; 실제 `--apply` 미실행 |
| 빌드 전용 inert 입력 | `/tmp/lagrange-r1-image-only-jug13bqi/compose.env`, parent 0700 / file 0600 / UID 1000, regular·non-symlink·조상 symlink 없음 |
| inert 입력 SHA-256 | `df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6` |
| 예정 systemd unit | `lagrange-r1-build-862fa954.service` — 관측 당시 not-found/inactive/dead |
| 예정 manifest | `/etc/lagrange/release-manifests/862fa954c16d28a617271fe6f613d34226e2d558.manifest` — 보호된 부모 신뢰·출력 부재 미검증 |

checkout과 inert 입력은 의도적으로 보존했다. 후속 문서 커밋이 `orange-dugong` HEAD를
바꾸더라도 빌드 소스를 그 HEAD로 바꾸지 않는다. 입력은 credential 없는 비활성
interpolation sentinel이며 운영 env로 사용하지 않는다. 실행 직전 경로·모드·입력 hash를
다시 확인한다. 임시 입력이 없어졌거나 변했으면 동일 계약으로 재준비한 근거를 남긴다.
이 문서와 CI 기록이 장기 근거이며 임시 파일 존재만으로 준비 상태를 인정하지 않는다.

원래 작업 checkout의 source-input hash는
`8ba48800d1460f7bc35136fb212abb8d39bd1e902368d830bd8326f5e4e0d8b9`였다.
825개 record의 종류·내용 hash는 같고 파일 모드가 0664/0775에서 새 checkout의
0644/0755로 달라졌다. [공식 hash](../../scripts/ops/lib/release-build-layout.sh)는
모드를 포함한다. 내용 변경이나 chmod 복구는 없었으며 후속 provenance에는 위
**detached checkout의 새 hash**를 사용한다.

## 외부 차단 조건과 담당

2026-10-12 **17:23:06 UTC**(2026-10-13 KST)의 고정 필드 읽기 전용 관측이다.
서비스 변경이나 빌드 시작 없이 다음을 확인했다.

| 조건 | 관측·해소할 사항 | 담당 |
| --- | --- | --- |
| root 인증·보호된 출력 | `sudo -n stat -c '%a %U:%G %n' /etc/lagrange/release-manifests` exit 1, interactive authentication required. 보호된 output/state 부모 신뢰와 C3 manifest 부재를 확인하지 못함 | root 실행 가능한 운영자/세션이 공식 보호 경로·no-clobber 조건 확인 |
| 빌드 health admission | 11개 container 중 10개 running/healthy. research-worker는 exited 2/unhealthy, restart count 7935, OOMKilled=false. Owner V2는 running | 운영자와 코디네이터가 의도된 lifecycle 및 기존 health 계약을 만족시키는 구체적인 조치 결정 |
| 자원·실행 직전 상태 | MemAvailable 8,047,004 kB, SwapFree 332 kB, disk free 602,937,503,744 bytes. Docker/containerd active. 실제 OOM/compiler/timer/one-off admission 미실행 | 공식 builder의 fresh 검사와 batch 사이 검사; 관측값을 후속 실행 승인으로 재사용하지 않음 |

ordinary all-healthy 조건이 충족되지 않았고 Owner V2가 실행 중이므로 두 reader를
정지한 대체 조건도 성립한다고 볼 수 없다. daily 모드에서는 상주 research-worker의
정지가 정상일 수 있으므로 운영 장애나 exit 2의 원인을 단정하지 않는다. 기존 예외·drain
attestation을 재사용하거나 서비스를 임의로 재시작·정지하지 않는다. 필요한 운영 조치는
대상과 영향을 정한 별도 결정으로 다룬다. image-only 범위 자체의 중복 승인 대기는 아니다.

암호·sudo 정책·보호 경로를 변경하지 않았다. 실제 빌드 시에는 unit·manifest 부재,
전체 health 목록, 소스와 env hash, 메모리·swap·OOM·기존 compiler, provider 관련
조건을 다시 확인해야 한다. 당시 `/opt/lagrange/current`의 link target은
`releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c`였으며 그 link만으로
DB·env·manifest·복구 호환성을 검증한 것은 아니다.

## 조건 해소 후 사용할 image-only 명령

아래는 **구문만 검증한 미실행 명령**이다. 위 root/health 조건이 해결되기 전에는
실행하지 않는다. 공식 [image builder](../../scripts/ops/build-production-images.sh)의
state·lock·자원·health 검사와 no-clobber manifest 생성을 유지한다.

```bash
sudo systemd-run --unit=lagrange-r1-build-862fa954.service \
  --service-type=exec --wait --collect \
  --working-directory=/data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/release-c3-worktree \
  --property=Nice=10 --property=IOSchedulingClass=idle \
  --property=CPUWeight=20 --property=IOWeight=20 \
  /usr/bin/env CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1 \
  LAGRANGE_CODE_COMMIT=862fa954c16d28a617271fe6f613d34226e2d558 \
  RELEASE_BUILD_SYSTEMD_UNIT=lagrange-r1-build-862fa954.service \
  RELEASE_BUILD_SYSTEMD_MANAGER=system \
  RELEASE_BUILD_HEALTH_UNITS=docker.service,containerd.service \
  RELEASE_BUILD_HEALTH_CONTAINERS=lagrange-station-postgres-1,lagrange-station-reverse-proxy-1,lagrange-station-api-server-1,lagrange-station-web-1,lagrange-station-research-worker-1,lagrange-station-recommendation-runner-1,lagrange-station-candidate-runner-1,lagrange-station-owner-beta-runner-1,lagrange-station-owner-equity-v2-runner-1,lagrange-station-nt-backtest-worker-1-1,lagrange-station-nt-backtest-worker-2-1 \
  RELEASE_BUILD_RESEARCH_EXCEPTION= \
  RELEASE_BUILD_DRAINED_READERS_ATTESTATION= \
  /bin/bash /data/worktrees/3puw275b/orange-dugong/target/r1-runtime-fxi6ltmv/release-c3-worktree/scripts/ops/build-production-images.sh --apply \
  --env-file /tmp/lagrange-r1-image-only-jug13bqi/compose.env \
  --manifest-file /etc/lagrange/release-manifests/862fa954c16d28a617271fe6f613d34226e2d558.manifest
```

systemd system service로 빌드 수명을 연결과 분리한다. Cargo jobs 2·Compose parallel 1,
Nice 10·idle I/O·낮은 CPU/I/O weight를 유지한다. 예외·attestation은 빈 값이다.
`--wait`의 원래 exit와 helper/journal·protected manifest 근거를 보존한다.

| 진행 보고 batch | 대상 서비스 — 각 Compose 호출에 정확히 하나씩 |
| --- | --- |
| 1 | `db-role-bootstrap`, `db-migrate`, `api-server` |
| 2 | `web`, `research-worker`, `recommendation-runner` |
| 3 | `candidate-runner`, `owner-beta-runner`, `owner-equity-v2-runner` |
| 4 | `nt-backtest-worker-1`, `nt-backtest-worker-2`, `paper-scheduler` |

공통 producer 준비와 checkpoint를 포함해 공식 순서를 따른다. batch는 보고 단위이며
Rust 병렬 빌드 허용이 아니다. 이전 빌드·compiler 종료와 exit·서비스 health·메모리/swap·
OOM을 확인한 뒤 다음 batch로 간다. OOM이나 서비스 종료가 생기면 중단하고 영향을 해결하며
전체 이미지 병렬 재시도는 금지한다. 의존성/base fetch가 필요한 빌드일 수 있다.

완료하려면 공식 도구로 12개 이미지의 실제 local ID·OCI revision·saved-image bytes와
최종 V2 manifest를 대조해야 한다. registry나 multi-platform image 증거가 아니다.
현재 이 task가 생성·검증한 C3 이미지 ID·saved-byte receipt·manifest는 **없다**.
이미지 생성은 release 설치·Compose up·운영 migration·provider 활성화를 허용하지 않는다.

## Migration·backup·rollback 인계 범위

130개 입력의 내용 동일성이 이전 독립 release source 검토를 C3에 연결한다.
아래는 소스 계약이며 운영 DB나 복구 훈련을 실행한 결과가 아니다.

- [Migration runner](../../deploy/db/migrate.sh)는 SQLx UP 뒤 catalog를 별도
  transaction으로 맞춘다. 전체 59개를 하나의 atomic 작업으로 취급하지 않는다.
  concurrent index migration 실패 시 ledger/index 상태를 먼저 확인하며 checksum을 고쳐 통과시키지 않는다.
- 0050은 기존 owner-beta run이 있으면 lineage를 만들어내지 않고 `55000`으로 거부한다.
  실제 데이터의 적용 가능성은 미확인이다. 0039/0040/0041/0053의 DOWN은 보존 의무가 있으면
  거부하고, 0059는 status row가 있으면 거부한다. 0049/0050·0054~0056은 파괴적 제거가 있어
  일반 운영 DOWN을 rollback 수단으로 제안하지 않는다.
- [Backup 도구](../../scripts/ops/run-production-backup.sh)와
  [runbook](../runbooks/production-release-and-backup.md)은 commit-pinned 보호 설정·암호화·hash
  검증을 요구한다. DB dump와 Raw/Curated archive는 순차 생성되며 하나의 DB+파일 snapshot
  fence나 WAL 연계 복구가 자동 증명되지 않는다. 제한된 disposable 복원 검사는 모든 59개
  schema·RLS·role·application 복구를 대신하지 않는다.
- [Installer](../../scripts/ops/deploy-production-release.sh)의 rollback은 검증된 이전
  release로 current link를 바꾸는 절차다. 이미지 전환·DB DOWN·데이터 복원·role/grant·secret·
  미완료 작업·health까지 자동 복원된다고 해석하지 않는다.

운영 적용 전에는 실제 ledger/checksum/index, 데이터 의존 upgrade 조건, DB+Raw/Curated
기준점 정합성·backup integrity, role/RLS·secret 복구와 이전 release 호환성을 별도로 확보해야
한다. R1의 격리 DB·CI 결과로 완료 처리하지 않으며 운영 적용·실제 데이터 확인은 R2에 남긴다.

## 보존 근거와 체크리스트

Private 인계: `target/r1-runtime-fxi6ltmv/release-readonly-862fa95/final-handoff/`.
14-entry `evidence.sha256`의 SHA-256:
`494ca1618c92941d515058c6d7949e3d491df44093f676700a21d20efdc28113`.
코디네이터가 모든 entry를 대조했다. 읽기 전용 child를 모두 회수했으며 이 준비 작업은
build/compiler/container/DB/systemd runtime을 만들지 않았다.

| 근거 | SHA-256 |
| --- | --- |
| official plan log | `a2a0970a56a5c84adc4ca4a31e39acba8355eedea712821f750603afa7149eb4` |
| official preflight log | `b4dc211c20a49e46e7166f540a212e847825583a719a26d3a3c39634f15b89fb` |
| syntax-checked `proposed-build.sh` | `815e3f53942aacbddc2729fa8dfdac90c0ccce16da459348ca449f41b7fb3334` |
| 118 migration sorted-pair manifest | `566511f58b9514d794247a1747907f71fa4c136e618409ad6c064eea179bf2b2` |

- [x] 동일 C3의 CI·기능 smoke 수용.
- [x] detached C3·inert 입력·공식 plan/preflight·정확한 명령 보존.
- [x] Migration·backup·rollback 소스 계약과 미검증 운영 범위 기록.
- [ ] root 인증·protected output/state 부모 신뢰·manifest 부재 확인.
- [ ] 실제 build-health admission 및 fresh 자원·OOM·compiler·timer 조건 충족.
- [ ] 서비스별 순차 빌드와 실제 C3 이미지 ID·revision·공식 manifest 검증.

마지막 세 항목이 충족돼야 [ROADMAP R1-09](../ROADMAP.md#5-r1--개발배포-기준-복구)를
완료 처리한다. 범위 안에서 준비 가능한 인계는 완료됐고 남은 외부 차단 조건은 위와 같다.

## 과거 준비 기록 — 2026-10-12 12:04 UTC

아래는 최초 관측 당시의 기록이며 이후 C3 검증·17:23 UTC 관측으로 현재 상태를 갱신했다.

# R1 release readiness — preliminary input review

2026-10-12. **Not accepted; no release build or production mutation performed.**
The candidate commit and its required hosted CI results remain pending. This
read-only preparation does not satisfy R1-09.

## Official build contract

Use `scripts/ops/build-production-images.sh` from a clean checkout of the exact
accepted commit. The image-only env file contains only the documented inactive
entitlement hash sentinel; never copy the operational Compose env. The official
root-only apply creates a new strict V2 manifest, validates all twelve image IDs
and OCI revisions, and builds one service at a time in its background systemd
unit. Cargo jobs2, Compose parallel1 and the documented health/resource gates
remain mandatory. No migration, release activation or provider call is authorized.

Source references: [builder](../../scripts/ops/build-production-images.sh),
[build gate](../../scripts/ops/lib/release-build-layout.sh), and
[release runbook](../runbooks/production-release-and-backup.md).

## Host observation at 12:04 UTC

The coordinator read only the exact `lagrange-station` Compose project's
container names/states/statuses and docker/containerd unit state. Ten monitored
containers were running and reported healthy, including
`lagrange-station-owner-equity-v2-runner-1`. The research worker was exited with
code2, reported as three weeks old. Both control units were active/running.
No container env, provider log or response body was read; no container or unit
was started, stopped, signaled or modified.

The ordinary build health predicate therefore has an unresolved research-worker
input. The existing legacy research exception is specifically a **running**
known-incident contract; it cannot cover this stopped worker. The optional
drained-reader contract requires both named readers stopped, with Owner V2 exit0
and research exit2, exact current observations, a commit-bound record and a
separately authorized bounded lifetime. The observed running Owner V2 does not
meet that contract. No old exception or attestation has been read, reused,
created or renewed.

Finish source validation and commit-pinned CI first. Then run the official
read-only plan/preflight against the clean candidate and record concrete build
inputs. A production lifecycle action or new exception is outside the current
R1 image-only execution scope and must not be inferred from this report. Leave
R1-09 open until its actual image/manifest evidence exists.
