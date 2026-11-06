# R5-03 → WP-4/WP-5 인계

2026-11-04, 대상 `a30edacff987d55b5be7cca25bed7faeec589629` 위 미커밋 후보.
R5-03은 2026-11-05 코드·격리 통합 범위에서 DONE이다([최종 인수](../reviews/2026-11-05-r5-paper-acceptance.md)). 이 문서는 구현된 인터페이스와 후속 책임을 고정하며,
배포·활성화 또는 운영 인수를 뜻하지 않는다. 검증 현황은
[진행 기록](../reviews/2026-11-04-r5-paper-ledger-integration.md)을 따른다.

## WP-4: Owner 화면

서버 계약은 [OpenAPI](../../apps/api-server/openapi.json)와
[생성 TypeScript](../../apps/api-server/generated/openapi.ts), 세부 의미는
[API 인계](../../target/r5-paper/coordinator/r5-03-20261104/api-contract-handoff.md)에 있다.
인증된 Owner 전용 `/api/v1/paper/owner-beta` 아래 state, consent, previews,
preview apply, pending target 기준 session 상세를 제공한다.

- 비활성 상태 조회는 DB·보호 설정을 읽지 않고 `DISABLED`를 반환한다.
  `ENABLED` 조회도 현재 보호 런타임/Current가 맞지 않으면 응답을 `BLOCKED`로
  표시한다. 이 조회는 저장된 제어 상태나 epoch를 변경하지 않는다.
- 일반 계정의 bind-strategy API는 beta 계정의 변경 경로가 아니다. 0067의 beta 보호 guard가 해당 직접 쓰기를 거절한다. 현재 beta 동의 grant는 자동 적용 binding을 켜며, 별도의 공개 수동/자동 전환 API를 제공한다고 표시하지 않는다.
- 동의는 활성화와 구분한다. 요청이 계좌·금융 정책·소스 핀·실행 파일 경로를
  선택하지 않는다. 변경 요청에는 CSRF와 `Idempotency-Key`가 필요하다.
- Apply 재실행의 영속 identity는 preview/token/account/epoch이다.
  헤더 키별 별도 body map은 없으며, 불확실한 커밋은 `PAPER_COMMIT_UNKNOWN`을
  반환한다. 이 경우 상태를 다시 조회하고 이미 확정된 결과를 확인한다.
- 금액·수량·비중은 decimal 문자열이다. 신호일 T, 경제적 실행 세션 E,
  실제 가격 가용·정산·평가 시각을 각각 표시한다. 지연 시뮬레이션을
  실제 E일 09:00 체결로 표현하지 않는다.
- 비교는 현재 `NOT_COMPARABLE / NO_MATCHED_BACKTEST_EXECUTION_CONTRACT`이다.
  기존 백테스트 성과를 Paper 성과로 표시하지 않는다.

## WP-5: 설치·CLI·drain/restore

[런타임 검증](../../crates/job-queue/src/owner_beta/paper/runtime.rs)은 고정된
보호 설정과 실제 실행 파일의 revision/hash를 확인하며,
[action scope](../../crates/job-queue/src/owner_beta/paper/action.rs)가 Current와
설정의 수명을 경제적 작업 완료까지 유지한다. API bool이나 shell의 성공만으로
실행 권한을 만들 수 없다. 설정 설치와 불변 release 연결은 WP-5 책임이다.

[0067 SQL](../../migrations/0067_owner_beta_paper_ledger_v1.up.sql)의 기존 제어
전이 ABI와 [SQL 인계](../contracts/owner-beta-paper-sql-v1.md)를
사용한다. `OFF`와 `RESTORED_OFF` 전이는 보호 drain/restore receipt 계약이 아직
연결되지 않아 `55000`으로 거절한다. 이를 직접 테이블 수정으로 우회하지 않는다.
WP-5에서 소유 작업의 종료·확정 상태 회수와 보존 증거를 연결해야 한다.

디버그 native fixture는 큰 실행 파일의 반복 무결성 검증 비용 때문에 기존 `with_deadlines` 설정으로 operation/cycle90초를 사용한다. 제품 기본15초의 운영 성능을 검증한 결과로 재사용하지 않는다. WP-5는 실제 설치 실행 파일과 자원 한도에서 구성된 cycle/action 기한을 검증해야 한다.

금융 입력 검증은 읽기 트랜잭션을 유지하는 동안 별도 heartbeat 연결이 필요하다. 실제 runner의 worker pool 기본값은4이며, 단일 연결로 축소한 fixture에서는 연결 대기 실패가 lease 상실로 분류될 수 있다. WP-5는 실제 연결 용량과 lease 갱신을 함께 검증해야 한다.

실행 중인 blocking callback의 취소는 협조적이며, timeout 응답만으로 해당 callback의 즉시 종료나 커밋 부재를 단정하지 않는다. CommitUnknown은 영속 상태 readback이 필요하고, WP-5 drain은 실제 작업 종료·pool 정리까지 확인해야 한다.

beta preview heartbeat는 현재10초이며 기존 기본 lease는60초다. WP-5의 runner 설정 검증은 사용자 지정 lease/heartbeat와 beta 실행 경로의 실제 적용 관계를 확인해야 한다. 일반 preview의 heartbeat 인자가 beta의 고정 주기를 바꾼다고 가정하지 않는다.

worker는 due 세션의 정산 또는 평가를 완료하면 그 회차의 결과를 반환한다. 새 추천의 AUTO queue/claim은 다음 정상 회차에서 새 보호 설정·Current와 갱신된 계정 상태를 검증한 뒤 진행한다. 운영 스케줄러는 한 회차의 경제적 진전과 후속 추천 생성이 서로 다른 회차에서 완료될 수 있음을 반영해야 한다.

기존 RECOVERY는 가장 이른 만료·미완료 세션만 받는다. 이미 정산됐지만 평가되지
않은 세션도 명시 복구할 수 있으며, 이 경우 정산 영수증과 원장을 보존하고 평가만
재개한다. 설정/권리/epoch 변경 뒤 새 경제적 쓰기는 실행 전 재검증한다.
이미 확정된 동일 영수증의 no-write replay는 새 실행 허가와 구분한다.

None4/default-unbound/production CLOSED를 유지한다. 운영 Origin, G4 선행 실제
세션, 활성화 후 WP7 운영 관측은 별도 인수 항목이다. KIS 계좌·잔고·주문 API,
새 provider 수집, 배포 또는 활성화 권한은 이 인계에 포함되지 않는다.
