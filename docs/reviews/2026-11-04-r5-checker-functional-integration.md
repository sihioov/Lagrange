# R5-02 실제 checker 격리 기능 통합 — 2026-11-04

실제 PG에서 32단계 기능 통합을 통과했다. 정상 3개 fixture 세션은 `Eligible`, 수동·변조·다른 입력은 각각 정확한 기대 사유로 거절됐다. 구현 통합의 검증이며, 실제 무인 Root Origin·운영 권리·활성화 인수는 별도다.

대상은 `a30edacff987d55b5be7cca25bed7faeec589629`의 작업 트리와 [Source3 전체 핀·인수 기록](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/integration-resume-20261104/root-runtime/functional-acceptance.json)이다. Git/OCI 불변 릴리스로 주장하지 않는다. 분류는 `SYNTHETIC_TEST_AUTHORITY_NOT_ORIGIN`, Source 예약은 `UNKNOWN`이다.

| 실제 checker 케이스 | 결과 |
| --- | --- |
| 2026-08-31, 09-01, 09-02 정상 세션 | `Eligible`, 정확한 3개 날짜와 유효 TTL |
| 수동 Source 입력 | `Ineligible / ORIGIN_UNPROVEN` |
| hash chain 내부 바이트 변조 | `Ineligible / ORIGIN_UNPROVEN` |
| 다른 Daily config 입력 | `Ineligible / COHORT_CHANGED` |

같은 public `check_paper_eligibility`를 호출했으며, 음성 케이스는 기대 사유 하나와 정확히 일치해야 통과한다. 각 케이스는 실제 DB·login role·nonce·버전을 확인하고, checker가 빌린 pool을 닫지 않았음을 확인한 뒤 소유 pool의 종료를 기다렸다.

32단계는 실제 token issuer/manager/client/parser와 RawStore, 승인된 V3 base의 역할별 읽기, 실제 Daily producer/admitter, sealed replay, scheduler, `run_once_with_daily` 계산·발행, public Source SQL 연결, checker 네 케이스를 포함한다. 성공한 job·factor·target 값을 직접 채우지 않았다. Token과 Transport는 sentinel/scripted 테스트 인증이며 외부 KIS 인증 증거가 아니다.

과거 날짜는 해당 테스트 프로세스의 realtime만 제어했다. [공식 libfaketime](https://github.com/wolfcw/libfaketime/tree/v0.9.13)의 고정 소스와 사전 검증을 사용했고, MONOTONIC·BOOTTIME·stat·호스트 시각·정식 SQL guard는 유지했다. 기존 `expected_at` 테스트 wrapper와 UNKNOWN Source slot seed는 명시적인 테스트 준비다. 최종 checker의 평가 시각은 실제 제어된 PG clock에서 읽었다.

실행 중 드러난 하네스 문제는 selector 이름, 메서드/시각 변수 충돌, E1 삭제 표식, 최종 PG readiness, 파일별 migration transaction·SQLx 이력, transaction-local RLS 범위, checkpoint의 실제 serde 바이트 순서, 음성 케이스 경로 이름이었다. 원래 거절 규칙은 유지했다. Rust 변경은 private 테스트 진단 두 문자열이며 집중 3개 테스트·strict job Clippy·rustfmt가 통과했다. Market fixture의 집중 1개 테스트·Clippy는 변경 없는 최종 입력의 기존 결과를 유지한다. 앞선 실패·부분 통과·정리 결과는 각 nonce receipt에 보존했다.

최종 nonce `16c0092436288c7fe6f264abc1653882`, parent session `66497`, terminal `4e3650 / exit0`. [실제 실행 receipt](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/integration-resume-20261104/runtime/16c0092436288c7fe6f264abc1653882/receipt.json)의 SHA-256은 `26b71ccc1bdcc1993bc10ec2c10593549a6ca13a841cef86bf6f08dbf909601b`이다. [Root 최종 대조](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/integration-resume-20261104/root-runtime/final-runtime-verification.json)에서 1290개 CLI 자식 terminal, 실행·actor 컨테이너119개와 role helper5개 제거, 두 소유 tmpfs volume 부재, 임시 credential 파일 제거, 선택 입력 불변을 확인했다. 기존 lifecycle UNKNOWN은 조사하지 않았으며 전역 quiescence 주장은 없다.

R5-02의 기능 통합 이정표는 PASS다. 전체 Origin/G2a·불변 Source/OCI·실제 권리/설치, G4 실제 선행 3세션 및 WP7 활성화 후 운영 3세션은 별도이며 OPEN을 유지한다. None4/default-unbound·production runtime CLOSED·WP3 선행 gate를 바꾸지 않았다. Production/provider/host-root/host-clock/service/install/commit/push/deploy 작업은 하지 않았다. Full workspace/hosted CI는 이번 집중 통합의 범위에 포함하지 않았다.

위임·회수는 [paseo-delegate](/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md)를 따랐다.
