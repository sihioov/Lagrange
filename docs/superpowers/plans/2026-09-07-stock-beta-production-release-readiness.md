# Stock Beta 운영 배포 준비 기록

기준일: 2026-09-07. 해당 날짜의 확인 기록이며 실시간 운영 상태가 아니다.

## 결론

코드 QA와 운영 배포는 별개다. 새 Stock Beta 운영 배포, 마이그레이션 및 전체
이미지 manifest 확정은 아직 완료되지 않았다. Git main 통합·푸시도 설치된
릴리스나 권한의 고정 커밋을 자동으로 변경하지 않는다.

| 구분 | 확인된 상태 |
| --- | --- |
| 운영 current | `69ad55db478e16306c22bbe41affccc2907f8742` |
| 승인된 이미지 빌드 대상 | `14f5bd5048ed58877fa51e441bdd1a87b7fa7dab` |
| 코드 변경 | 차트, 구조 개선, QA 및 API artifact 읽기 전용 경계 검사 수정 커밋 완료 |
| 새 운영 릴리스 rollout | 미완료 |
| 전체 12개 이미지 manifest | 미생성/미검증 |
| 기존 research-worker | 기존 `PRICE_CURATION_FAILED`, exit 2 재시작 상태의 원인 추가 확인 필요 |
| 일일 KIS 수집 | 구형 timer 중지 확인; 교체 및 누락 세션 재수집 미완료 |

구조 개선과 UI QA는 [실행 계획](2026-09-07-stock-beta-structural-ui-remediation.md)과
[QA 기록](2026-09-07-stock-beta-structural-ui-remediation-qa.md)을 참조한다.
수집 장애 원인과 복구 조건은
[KIS stale-release incident](../../runbooks/kis-daily-stale-release-20260907.md)에 기록했다.

## 완료된 사전 조치

### 배포 경계 검사

`14f5bd5`에서 V2 runtime static/self-test와 runbook을 정합화했다.
API의 승인된 chart artifact 읽기 전용 mount는 허용하고, API의 Raw·자격 증명
접근과 artifact 쓰기 및 Web의 artifact 접근은 금지한다. 관련 7개 사전 검사 통과 기록이 있다.

### 운영 설정 및 백업 pin

- 기존 설치 릴리스 자체 `--check`는 PASS였다. 이전 manifest는 해당 릴리스의
  loader로 검사해야 한다. 이전 11개와 대상 12개 이미지의 schema 차이를
  대상 loader의 오류로 혼동하지 않는다.
- 대상 production config 검사는 PASS였다. pending 파일에 commit 키가 없더라도
  정확한 process commit을 공급하는 공식 dotenv 경로는 지원된다.
- 사용자가 active backup 설정의 commit/Compose file/Compose env 세 pin만
  current `69ad55d`에 맞춰 수정했다. 원본은 root 보호 rollback 사본으로 보존했다.
  이후 진단에서 active pin=previous와 backup check PASS를 확인했다.
- pending backup 설정과 retention은 수정하지 않았다. 설정 검사는 실제 백업·격리 복원
  검증이 아니다. 이번 작업으로 새 백업, 복원 또는 pruning을 실행하지 않았다.

### OOM 확인과 두 백테스트 worker 복구

- 과거 kernel 조회에는 OOM 관련 105줄, killed-process matching 35줄이 있었다.
  이는 105건의 사고라는 뜻이 아니다. 확인된 날짜는 8월 31일, 9월 1일, 9월 4일이며
  마지막 관련 기록은 9월 4일 04:39 UTC 부근이었다.
- 두 worker는 처음에는 healthy이면서 과거 `OOMKilled=true`가 남아 있었다.
  cgroup `oom=0`, `oom_kill=1/2`는 재측정에서도 증가하지 않았다.
  worker 관련 kernel 기록은 global OOM을 가리켰지만 각 줄을 새 종료 한 건으로 세지 않았다.
- 사용자가 두 worker의 순차 재시작을 별도로 승인했다. 각 재시작 직전 read-only DB
  검사에서 backtest RUNNING/QUEUED/locked CANCELED 작업 합계가 0임을 확인했다.
- 정확한 container/image를 확인한 뒤 worker 1은 09:39:11 UTC, worker 2는
  09:40:09 UTC에 순서대로 재시작했다. 각 worker의 healthy 상태를 확인한 뒤 진행했다.
  두 worker 모두 같은 이미지로 running/healthy/OOMfalse가 되었다.
- Web/API/PostgreSQL은 재시작하거나 설정을 바꾸지 않았다. 후속 kernel 조회에서
  새 matching OOM 기록은 없었다. 이는 해당 관측 시점의 결과이지 이후 무사고 보장이 아니다.

## 제한형 권한과 이미지 빌드 기록

사용자 인증으로 설치한 권한은 고정 커밋의 status/진단, 네 개 순차 이미지 빌드 묶음,
공식 manifest 확정 및 revoke에 한정된다. 만료는 **2026-09-10 14:16:53 KST**다.
임의 shell, 운영 설정 수정, 재시작, backup/provider 실행, migration/rollout은 포함하지 않는다.
일회성 인증 payload와 임시 staging 파일은 저장소의 배포 수단이 아니다.

1. 최초 batch 1 호출은 `REQUIRED_SERVICE_UNHEALTHY`로 차단되어 빌드를 시작하지 않았다.
2. 두 worker 복구 후, 기존 research-worker 실패만 감시하는 승인된 이미지 빌드 예외로
   `lagrange-release-build-14f5bd5-batch-1.service`가 시작되었다.
3. 중간 확인에서 db-role-bootstrap과 db-migrate의 대상 revision을 검증했다.
   이후 unit은 inactive/dead, Result=success, ExecMainStatus=0으로 관측되었다.
   완료 transient unit이 정리된 뒤 absent로 보이는 것은 실패 증거가 아니다.
4. 다만 private batch PASS marker와 최종 API 이미지 검증은 이 기록에서 확정하지 않는다.
   Batch 2–4와 manifest finalize는 시작하지 않았다. 전체 이미지 빌드 완료로 보고하지 않는다.

재개 시 묶음당 2–3개 서비스, 실제 Compose 호출은 한 번에 정확히 한 서비스다.
`COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`와 낮은 CPU/I/O 우선순위를 유지하고,
각 묶음 사이 완료 상태·메모리/swap·OOM·서비스 health 및 잔여 compiler를 확인한다.
기존 research 실패를 허용하는 image-only 진단 결과는 release-ready 승인이 아니다.

## 수집 장애 상태와 진단 정정

2026-09-07 21:28 KST 확인에서 구형 `lagrange-kis-daily-66b2a8c.timer`는
inactive/disabled이며 다음 trigger가 없었다. 해당 구형 작업은 실행 중이 아니었고,
기존 failed 이력은 보존했다. 교체 timer는 아직 설치하지 않았다.

구형 66b release의 recovery가 corporate-action-only Raw batch를 EOD bundle로
정규화하려는 원인은 확인했다. 기존 수정 `db7de37`은 current 69ad와 대상 14f에
이미 포함되어 있으며 provider-free recovery 회귀 테스트 9개가 통과했다.
실제 수집 복구는 올바른 immutable image와 timer 연결 및 누락 세션 publication 검증이 필요하다.

scheduled 출력의 detail 부재는 progress relay의 의도된 필드 제거로 설명된다.
이를 구형 바이너리의 증거로 삼지 않는다. 별도 장기 실행 research-worker의
curation 실패는 여전히 추가 진단 대상이며 timer 수정으로 해결됐다고 보고하지 않는다.
OCI revision label만으로 바이너리·소스 정합성을 확정하지 않는다.

## 남은 운영 단계

1. incident runbook의 공식 daily installer 절차와 16:30 KST 이전 guard를 지켜
   현재 설치 릴리스 기반의 timer 교체를 준비한다. 누락 날짜, 요청 예산, token manager와
   기존 daemon의 충돌 및 permanent-failure 상태를 검토한 뒤 승인된 재수집을 진행한다.
   Raw 삭제나 PUBLISHED 상태 수동 덮어쓰기로 복구를 가장하지 않는다.
2. 별도 research-worker curation 실패 원인을 확인하고 배포 health 조건을 충족한다.
3. current와 일치하는 암호화 PostgreSQL/Raw/Curated 백업 및 격리 복원의 실제 증빙을 확보한다.
4. 고정 대상 커밋에 대해 나머지 순차 이미지 빌드와 12개 이미지 manifest 검증을 완료한다.
5. protected 설정·entitlement·V2 artifact 권한·rollback 조건을 확인하고, 별도의 운영 권한으로
   공식 immutable installer, migration, rollout 및 실제 서비스 확인을 수행한다.

Git 정리 작업은 위 운영 단계를 자동 실행하거나 기존 승인 범위를 확대하지 않는다.

## Git 통합 전 재검증 — 2026-09-07

통합 커밋 `f04929b`의 tracked tree는 검토 대상 `14f5bd5`와 동일함을 확인했다.
아래 검사는 운영 변경 없이 이 통합 checkout에서 다시 실행했다.

- OpenAPI check: exit 0, 85개 operation 및 generated TypeScript 계약 정합성 통과.
- Web typecheck: exit 0. Web lint: exit 0, 기존 경고 4개와 deprecated config info 1개.
- 전체 Web Vitest (`--configLoader runner`): 37개 파일, 271개 테스트 통과, 2.22초.
- V2 runtime static/self-test, production image build static/self-test,
  production ops static/self-test, migration static 검사: 총 7개 exit 0.
- 독립 변경 범위 검토 및 `git diff --check`: 통과. main push workflow에 운영 자동 배포
  단계가 없음을 확인했다. 실제 원격 CI 결과는 push 이후 별도로 확인해야 한다.

이번 재검증에서는 production build, browser E2E, DB 통합 검사 및 PNG 재렌더를
다시 실행하지 않았다. 기존 실행 증거와 이번 정적 검토를 재사용했으며,
이를 이번에 새로 실행한 검사로 합산하지 않는다.
