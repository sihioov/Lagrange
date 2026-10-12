# Lagrange 개발 로드맵

> **앞으로의 개발 우선순위와 진행 상태는 이 문서를 기준으로 관리한다.**
> Owner가 2026-10-12에 아래 개발 방향과 지속적인 완료·변경 추적을 채택했다.
> 단계 이름 `R1`~`R5`는 이 실행 로드맵의 번호이며, 요구사항서의 Phase 0~4와 다르다.

| 관리 항목 | 현재 값 |
| --- | --- |
| 최종 갱신일 | 2026-10-12 (Asia/Seoul) |
| 최초 코드 기준선 | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` (2026-10-10) |
| 첫 출시 목표 | 매 거래일 최신 데이터로 종목과 ETF 전략을 확인할 수 있는 Owner 전용 연구 베타 |
| 현재 실행 우선순위 | R1 CI 복구 → R2 일일 수집·실제 Owner 사용 흐름 완성 |
| 상세 실행 계획 | [R1 Paseo 위임 계획](superpowers/plans/2026-10-12-r1-ci-recovery.md) — WP-1~WP-5 로컬 검증·독립 리뷰 수용 |
| 다음 착수 항목 | 후보 커밋 고정 → R1-08 동일 커밋의 CI·research smoke → R1-09 배포 후보 준비 |
| 진행 중인 구현 항목 | R1-02~R1-07 로컬 검증 통과; 최종 커밋의 hosted 수용 전까지 DOING. R1-08 진행 중 |
| 판단이 필요한 항목 | R4-01 전용 백테스트의 구체적인 시뮬레이션 계약 |

## 1. 목표와 문서의 역할

첫 출시에서 Owner는 정상 로그인 후 종목을 추가하고, 준비 상태를 확인하며, 최신 일봉·신호·ETF 추천을
볼 수 있어야 한다. 데이터가 늦거나 실패하면 기준일과 원인이 드러나야 하고, 다음 수집과 복구 결과를
추적할 수 있어야 한다. 그다음 실제 실시간 시세를 인수하고, 추천 전략의 성과를 같은 입력으로 검증하는
전용 백테스트, Paper 순으로 확장한다.

- 이 문서: 현재 우선순위, 작업 체크리스트, 완료 기준, 차단 사유, 변경 이력.
- [STATUS.md](STATUS.md): 과거 날짜별 구현·운영 기록. 본문의 “현재”는 해당 기록 당시를 뜻한다.
- [요구사항서](Lagrange_Station_Requirements_v1.1.md)와
  [시스템 설계서](Lagrange_Station_System_Design_v1.1.md): 장기 제품 목표와 설계 근거.
- `docs/decisions/`, `docs/contracts/`, `docs/runbooks/`: 개별 승인·계약·실행 절차의 근거.
  아래에서는 해당 문서를 링크하고 세부 계약을 중복 작성하지 않는다.

로드맵 채택은 개발 순서와 관리 방식의 채택이다. 아직 비어 있는 백테스트 규칙, 새로운 데이터 API·권리,
Paper의 운영 전환 조건, 실거래까지 일괄 확정한 것으로 해석하지 않는다. 이미 승인된 일일 KIS EOD와
[ADR-0009의 제한된 WebSocket 활성화](decisions/0009-kis-owner-directed-websocket-activation.md)는
그 범위 안에서 진행하며 같은 승인을 다시 요구하지 않는다. 소유자의 후속 명시적 결정은 관련 계약과
이 문서에 함께 반영한다.

## 2. 진행 상태와 완료 체크 규칙

| 상태 | 의미 | 체크박스 |
| --- | --- | --- |
| TODO | 착수 전 | 미체크 |
| DOING | 담당자와 작업 범위가 정해져 진행 중 | 미체크 |
| BLOCKED | 선행 조건·외부 입력 때문에 진행 불가. 해소 조건 필수 | 미체크 |
| DONE | 해당 항목의 검증과 필요한 운영·사용자 확인까지 완료 | 체크 |
| DEFERRED | 범위에서 유예. 이유·결정 근거·재개 조건 필수 | 미체크 |

1. **작업 ID를 유지한다.** 착수할 ID를 정하고 아래 체크리스트 항목에 `DOING`과 담당자 또는 작업 링크를
   붙인다. 아무 표시가 없는 미체크 항목은 `TODO`다. 다른 에이전트에게 넘겨도 ID를 바꾸지 않는다.
2. **변경과 기록을 함께 남긴다.** 구현·테스트 수정 커밋에 관련 체크리스트와 상단 현황을 함께 갱신한다.
   실제 배포·거래일 관찰처럼 커밋 후에 얻은 결과는 후속 문서 커밋으로 기록한다. 설치된 immutable
   release의 파일을 수정하지 않는다.
3. **완료에는 근거를 붙인다.** 체크된 항목에 완료일, 검증 대상 커밋, 테스트·리뷰·운영 기록 링크와 결과를
   남긴다. 같은 커밋에 추가하는 문서 자체의 완료 근거는 “이 기록을 추가한 커밋”으로 표시할 수 있다.
   자기 커밋 해시를 쓰기 위한 반복 amend는 필요 없다.
4. **검증 수준을 구분한다.** 소스 구현, 격리 환경 검증, 운영 배포, 실제 데이터 수신, Owner 사용 확인은
   서로 다른 증거다. 과거 테스트 통과나 컨테이너 healthy만으로 뒤 단계의 체크를 완료하지 않는다.
   실패·건너뜀·미실행을 각각 기록한다.
5. **단계 완료는 별도로 판단한다.** 각 단계의 필수 작업과 종료 조건을 모두 만족해야 단계 상태를 `DONE`으로
   바꾼다. 일부 항목 유예로 종료 조건이 사라지지 않는다. 서로 크기가 다른 항목 수로 완료율을 계산하지 않는다.
6. **수정·재발은 이력을 보존한다.** 완료 항목이 다시 깨지면 체크를 해제하고 `DOING` 또는 `BLOCKED`로
   되돌린다. 이전 성공 근거를 지우지 않고 재개 사유와 새 검증을 변경 이력에 추가한다.
7. **범위 변경은 드러내서 관리한다.** 작업을 분할하면 하위 ID를 추가하고 원래 ID에서 연결한다. 대체·유예한
   항목은 삭제하거나 ID를 재사용하지 않는다. 목표·완료 기준을 바꿀 때 이유, 영향, 결정 근거를 기록한다.
8. **증거는 작고 안전하게 남긴다.** 상세 결과는 날짜가 있는 `docs/reviews/` 문서나 CI 실행으로 연결한다.
   키·토큰·세션 쿠키·계좌 식별자·공급자 본문·원시 로그는 넣지 않는다. `/tmp`에만 있는 자료를 유일한
   장기 근거로 사용하지 않는다.

완료 항목의 기록 형식은 `완료 YYYY-MM-DD; 대상 <commit>; 검증 <명령/기록 링크>; 결과 <요약>; 한계 <있으면 기재>`다.
차단 항목은 `BLOCKED; 원인 <사실>; 해소 조건 <필요 입력/작업>; 후속 담당 <담당/역할>`을 덧붙인다.

- [x] **DOC-01 — 개발 로드맵과 유지 규칙 도입.** 완료 2026-10-12; 대상은 이 기록을 추가한 커밋;
  근거는 Owner의 로드맵 채택과 본 문서, [저장소 지침](../AGENTS.md), [상태 문서 안내](STATUS.md).
  문서 검증 결과는 도입 커밋의 설명에 남긴다. R1~R5의 기능 완료를 뜻하지 않는다.

## 3. 최초 기준선과 확인된 공백

이 절은 **고정된 출발점**이다. 아래 사실을 매번 “현재”로 재사용하지 않는다. 새로운 결과는 해당 작업과
변경 이력에 기록하고, 실행 우선순위는 상단 표를 갱신한다.

### 3.1 이미 구현된 범위

| 영역 | 확인된 구현·과거 검증 | 아직 구분해야 할 사항 |
| --- | --- | --- |
| ETF11 | 2016-08-29~2026-08-28 가격 데이터, 2,452 세션·26,972 bars의 v3 artifact 승인·배포 기록. 5개 기준 전략의 목표 비중 계산 | 역사 데이터 확보와 매 거래일 갱신은 별개. `PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false` 경계 유지 |
| Owner 종목 V2 | 종목 추가·상태조회·재시도·비활성화, 신호·필터·상세·일봉 차트. 최대 100 active, 목표 261·최소 121 관측값 정책 | 현재 READY 종목 수·최신 데이터 날짜·정상 로그인 화면은 운영에서 별도 확인 |
| 일반 백테스트 | 생성·조회·취소·성과·비교 API와 Nautilus/Python worker | Owner-beta 승인 artifact 전용 성과 백테스트와 입력 권한·계약이 다름 |
| 시장 WebSocket | 2026-10-04 합성 upstream으로 runtime→DB→인증 API→nginx→Chromium, 30종목·30분 soak 검증. 10-07 WS 전용 전환 | 실제 KIS 30종목 수신과 최종 운영 인수는 미확인. V2의 100종목 정책이 WS의 고정 30종목 범위를 늘리지 않음 |
| 웹·관측 | Lightweight Charts, Owner 서비스 상태 22행, 메인 대시보드 배치까지 기준 커밋에 반영 | 개별 기능 QA와 최신 전체 CI, 상태 패널과 실제 데이터 정상 여부는 별개 |
| Paper·실거래 | Paper 코드와 거래 안전장치 일부 구현 | Owner-beta Paper는 비활성. 실거래는 현재 읽기 전용 프로젝트 밖의 별도 프로젝트 |

근거:
[ETF11 기록](STATUS.md),
[추천 구현](../crates/job-queue/src/owner_beta/target.rs),
[V2 운영 계약](runbooks/owner-managed-equity-universe-v2.md),
[WS 통합 검증 기록](superpowers/plans/2026-10-01-kis-market-stream-completion.md),
[WS 전환 기록](handoffs/2026-10-07-kis-websocket-only.md),
[차트 리뷰](reviews/2026-10-07-stock-beta-lightweight-charts-review.md),
[서비스 상태 운영 문서](runbooks/service-status.md).

### 3.2 운영 관측 — 2026-10-11 KST

직전 프로젝트 점검에서 `git ls-remote`, `readlink`, Docker의 이름·상태·revision 등 고정 필드,
systemd의 상태·결과 필드, 본문을 버리는 HTTP health 요청으로 확인했다. 이 문서 작성 때문에
수집기를 실행하거나 설정을 변경하지 않았다.

- 로컬·원격 `main`과 `/opt/lagrange/current`가 기준 커밋 `fce9b90`에 일치했다.
- 실행 중인 Lagrange 컨테이너 10개는 healthy였다. Web·API·Owner V2는 기준 커밋이며 restart 0, OOM false였다.
  다른 실행 중인 worker까지 모두 같은 revision이라고 확인한 것은 아니다.
- 공개 `/healthz`는 HTTP 200이었다. 이는 로그인 이후 신호·차트 성공의 증거가 아니다.
- 서비스 상태 관측 타이머가 동작했고, 당일 백업·복원 검증 서비스는 exit 0이었다.
- 기준 릴리스의 KIS daily 서비스·타이머는 disabled/inactive, 실행 시각 없음이었다.
  이전 상주 research-worker는 종료 상태였다. **daily 모드에서는 상주 worker 정지가 정상일 수 있다.**
  복구 대상은 [모드 계약](contracts/service-status-v1.md)에 맞는 일일 실행과 실제 데이터 발행 경로다.
- 현재 운영 DB의 최신 기준일·READY 종목 수, 실제 broker 수신, 정상 Auth0 Owner 화면은 이 점검에서 확인하지 않았다.

### 3.3 CI 관측 — 2026-10-11 실행, 2026-10-12 재확인

기준 커밋의 [CI 실행 38136740553](https://github.com/sihioov/Lagrange/actions/runs/38136740553)과
[Research worker smoke 38136740521](https://github.com/sihioov/Lagrange/actions/runs/38136740521)는 실패했다.
아래는 실패 지점이며, 수정 방법이나 근본 원인이 모두 확정됐다는 뜻은 아니다.

| 실패 지점 | 확인된 증상 | 연결 작업 |
| --- | --- | --- |
| format / clippy | 포맷 차이, `kis-client` Clippy 오류 | R1-02 |
| workspace-tests | `market_stream_transport` 통합 테스트 컴파일 실패 | R1-03 |
| PostgreSQL 통합 | migration stage 38 이후 catalog sync의 `enqueue_auth_audit` 함수 해석 실패 | R1-04 |
| web | 단위 테스트에서 Chromium 실행 파일 없음. CI의 브라우저 설치는 그 뒤에 위치 | R1-05 |
| python-tests | 446 pass / 1 fail / 10 skip. smoke 계약 테스트가 기존 Raw 디렉터리 처리 문자열을 기대 | R1-06 |
| research-smoke | Raw init calendar harness의 로컬 Cargo registry cache 요구로 기능 smoke 진입 전 중단 | R1-06 |
| policy | candidate static check에서 Compose 필수 `RESEARCH_ENTITLEMENT_SHA256` 입력 누락 | R1-07 |

## 4. 단계와 의존 관계

| 단계 | 사용자에게 생기는 결과 | 초기 상태 | 선행 조건 |
| --- | --- | --- | --- |
| R1 | 검증 가능한 개발·배포 기준선 | DOING — 기준선 수용, 수정·검증 | 없음 |
| R2 | 매 거래일 사용할 수 있는 일봉·신호·추천 베타 | TODO | 진단은 즉시 가능. 수정 릴리스의 운영 인수에는 R1 통과 필요 |
| R3 | 실제 시장 시세가 전달되는 실시간 화면 | TODO | R1과 유효한 당일 calendar/window·권한·READY 종목 등 운영 전제 |
| R4 | 같은 추천 입력으로 재현 가능한 전략 성과 | BLOCKED — 구현 계약 미확정 | 결정안 작성은 즉시 가능. 구현은 R4-01의 구체적 승인, 출시에는 R1·R2 필요 |
| R5 | 검증된 전략의 매일 모의 운용 | BLOCKED — 선행 단계 대기 | R4 인수와 전용 Paper 입력·활성화 조건 충족 |

- **첫 출시 묶음은 R1 + R2다.** R2의 EOD 사용 인수는 R3의 WebSocket 인수와 독립적으로 완료할 수 있다.
- R3의 소스 점검과 R4의 결정안 작성은 R1·R2와 병행할 수 있다. R4는 WS 완료를 필수 선행 조건으로 두지 않는다.
- 실제 broker 확인은 거래시간과 당일 증거에 의존한다. 휴장·장 마감을 코드 실패나 수신 성공으로 처리하지 않는다.
- 일정은 단계의 종료 조건으로 관리한다. 3거래일 관찰처럼 실제 시간이 필요한 검증은 임의로 단축하지 않는다.
- 독립된 수정은 파일 범위를 나눠 진행한다. 공유 DB·대규모 테스트·운영 빌드는 자원 충돌을 피하도록 순차 실행한다.
  운영 빌드는 [AGENTS.md](../AGENTS.md)의 서비스별 순차 빌드·자원 제한을 따른다.

## 5. R1 — 개발·배포 기준 복구

**목표:** 깨끗한 환경에서 같은 소스의 필수 검증을 재현하고, 검증된 릴리스를 준비한다.

[상세 실행 계획](superpowers/plans/2026-10-12-r1-ci-recovery.md)은 `$paseo-delegate`로 실행할
7개 작업 패키지의 소유 파일·의존성·검증 기준을 정의한다. Owner의 실행 요청으로 WP-1부터 진행한다.
[실행 기록](reviews/2026-10-12-r1-execution.md)에 담당 에이전트와 수용 상태를 남긴다.

- [x] **R1-01 — 실패 기준선 재현·분류.** §3.3의 각 실패를 현재 작업 커밋에서 확인하고 제품 결함,
  테스트 계약 불일치, CI 환경 문제를 구분한다. 담당 범위와 실행할 최소 검증을 기록한다.
  완료 2026-10-12; 대상 `ac0a083`; [기준선·수정 계약](reviews/2026-10-12-r1-baseline.md).
  포맷·Clippy·기능 누락·Python·Compose 입력을 재현하고 feature-enabled transport 50개 통과.
  DB 원인은 source/동일 product baseline의 CI로 대조; 실제 SQL·기능 smoke는 Docker 접근 문제로 미검증이며 후속 검증에 남는다.
- [ ] **R1-02 — Rust format·Clippy 복구.** DOING — 로컬 수정·검증 수용, 최종 커밋의 hosted 검사 대기.
  workspace 포맷과 feature 조합을 포함한 strict Clippy 6개 선택이 모두 통과했다.
  [최종 Rust 검증](reviews/2026-10-12-r1-workspace-validation.md), [lint 수정·검증](reviews/2026-10-12-r1-rust-lint-validation.md).
- [ ] **R1-03 — Rust workspace 테스트 복구.** DOING — 로컬 전체 실행 통과, 동일 커밋의 hosted 검사 대기.
  CI는 `--features kis-client/test-support`를 명시한다. 최종 로컬 집계는 outer binary 2,833 pass,
  nested child 1 pass, doc-test 52 pass, 실패 0·기존 ignored 6·filtered 180이다.
  [집계·skip 사유·소스 동일성](reviews/2026-10-12-r1-workspace-validation.md),
  [C2 pure 70·relay 13·restart](reviews/2026-10-12-r1-c2-fixture.md),
  [실제 역할 runtime 18](reviews/2026-10-12-r1-runtime-validation.md).
  float_roundtrip의 명시적 의존성과 legacy 합성 snapshot의 바이트·hash·tamper 회귀도 통과했다.
- [ ] **R1-04 — PostgreSQL 초기화·업그레이드 검증.** DOING — 로컬 전체 validator `APPROVED`, hosted 검사 대기.
  staged 0038→0041, 빈 DB의 전체 59개 migration 및 checksum이 같은 no-op 재실행을 통과했다.
  DB-gated 10개 선택은 122 pass·실패/ignored/SKIP 0이다.
  [최종 PostgreSQL 검증](reviews/2026-10-12-r1-postgres.md), [DB 계약 수정](reviews/2026-10-12-r1-db-contract-repair.md).
- [ ] **R1-05 — Web 검증 환경 복구.** DOING — 설치 순서·소유 서버 실행기·현행 WS 테스트 계약의 로컬 검증 수용, hosted 검사 대기.
  lint·typecheck·unit·production build를 통과했고, 마지막 fixture/catalog 12개 검사도 통과했다.
  최종 공식 wrapper는 회귀 31개와 **브라우저 135/135회**를 통과했다(일반 121 + stream 13 + 별도 owner-empty 1).
  필수 synthetic identity는 134개이며 owner-empty만 두 모드에서 실행했다. 실패·skip·retry·flaky는 0이다.
  [전체 Web 검증](reviews/2026-10-12-r1-web-validation.md),
  [최종 소스 리뷰](reviews/2026-10-12-r1-web-final-repair-review.md),
  [별도 실제 API의 두 Chromium phase](reviews/2026-10-12-r1-actual-api-validation.md).
  이전 실패와 환경 오류는 해당 보고서에 보존했다.
- [ ] **R1-06 — Python 계약·research smoke 복구.** DOING — 로컬 Python 450 pass·기존 skip 10,
  backup 정책 6개와 전체 기능 smoke 통과; hosted 검사 대기.
  [전체 smoke](reviews/2026-10-12-r1-smoke-validation.md)는 script·supervisor exit 0, functional PASS,
  SKIP 0이며 새 캐시에서 Raw probe를 수행했다. 입력 933개의 동일성과 소유 자원 정리를 확인했다.
  [Python 결과·기존 skip 사유](reviews/2026-10-12-r1-runner.md). Windows parity는 미검증이다.
- [ ] **R1-07 — 정책 검사 입력 복구.** DOING — 격리 Compose 입력과 누락 거부 회귀를 로컬에서 통과했다.
  운영 credential 없이 검사하고 production의 필수 입력 규칙을 유지한다.
  [runner·정적 계약 검증](reviews/2026-10-12-r1-runner.md); 동일 후보 커밋의 전체 policy job 대기.
- [ ] **R1-08 — 통합 검증 확정.** DOING — 로컬 검증·독립 리뷰를 통과한 후보를 커밋한 뒤,
  **같은 최종 커밋**의 필수 CI와 research smoke 실행 URL·checkout SHA·단계별 결과를 확인한다.
  실패·기존 skip·미실행은 구분하고, 로컬 통과를 hosted 통과로 대신하지 않는다.
- [ ] **R1-09 — 배포 후보 준비.** 정확한 커밋·이미지·manifest, migration 순서와 기존 릴리스 복구
  절차를 확인한다. 실제 운영 적용·결과는 R2의 릴리스 증거에 연결한다.

**종료 조건:** R1-01~R1-09 완료, 필수 CI·smoke 통과, 동일 커밋의 검증·배포 후보 근거 확보.
규칙 변경이 필요하면 원래 검사의 목적을 충족하는 검증과 변경 이유를 함께 남긴다.

근거: [CI workflow](../.github/workflows/ci.yml),
[smoke workflow](../.github/workflows/research-smoke.yml),
[배포·백업 runbook](runbooks/production-release-and-backup.md).

## 6. R2 — 일일 연구 베타 완성

**목표:** 예약 수집부터 실제 Owner 화면까지 매 거래일 일관되게 동작한다.

- [ ] **R2-01 — 실행 경로 진단.** 보호된 릴리스와 설치된 모드에 맞는 daily unit·timer·작업 결과를
  확인한다. 비활성 이유와 마지막 성공·누락 구간을 구분하고 상주 daemon과 중복 실행하지 않는다.
- [ ] **R2-02 — 거래일·시간창 운영 책임 확정.** 공식 당일 calendar/window 증거의 공급 경로,
  확인 담당, 준비 마감과 부재 시 동작을 기록한다. 기존 단일 calendar 요청·캐시 정책을 유지한다.
- [ ] **R2-03 — 승인된 일일 실행 복구.** 원인 수정과 R1 검증 후 commit-pinned installer·공식 helper로
  일일 실행을 복구한다. 복구할 날짜·요청 예산을 먼저 확인하고 승인 범위 밖의 과거 일괄 수집은 추가하지 않는다.
- [ ] **R2-04 — ETF EOD와 추천 갱신 검증.** Raw, 정규화, Curated, 발행과 API의 기준일·출처를
  연결해 확인한다. Owner가 사용하는 ETF 추천도 매 거래일 승인된 입력으로 실행·성공·발행되며,
  입력 기준일과 추천 `as_of`가 기대 거래일에 일치하는지 대조한다. 봉인된 역사 artifact의 지원
  기준일 이후를 제공할 수 없다면 새 입력의 승인·갱신 경로를 먼저 확정하고 그동안 이 항목을
  `BLOCKED`로 남긴다. 역사 price-only artifact를 generic READY로 간주하거나 과거 추천을 최신으로 표시하지 않는다.
- [ ] **R2-05 — Owner V2 갱신 검증.** 정상 등록된 종목의 수집·generation·artifact·신호·차트가
  같은 입력을 가리키는지 확인한다. ETF 공용 기준일과 Owner 종목 기준일을 각각 검증한다.
- [ ] **R2-06 — 실제 Owner 사용 인수.** 정상 Auth0 로그인에서 종목 추가→상태 확인→READY→신호·차트,
  필터·상세, 재시도·비활성화, ETF 전략 설정·추천 조회를 확인한다. 사용자 세션을 추출하거나 가장하지 않는다.
- [ ] **R2-07 — 실패·만료 표시와 복구.** 누락·노후·미래 데이터, 수집 실패, 세션 만료가 화면에
  정확히 표시되는지 검증한다. 재실행·재시작 시 중복 발행 방지와 복구도 확인한다.
  장애 주입은 격리 환경에서 수행하고 운영 관찰과 구분한다.
- [ ] **R2-08 — 연속 3거래일 무인 관찰.** 일별 예약 실행, ETF 데이터·추천 실행·발행,
  등록 Owner 종목 갱신 결과와 입력·추천·API·화면 기준일을 기록한다.
  수동 보충 성공은 해당 일의 무인 성공으로 세지 않는다.
  하루라도 실패하면 원인 수정 후 연속 관찰을 다시 시작한다.
- [ ] **R2-09 — 첫 베타 릴리스 인수.** exact revision·manifest·모드, 운영 적용, health·백업/복구
  근거와 정상 Owner 사용 결과를 묶는다. 운영 확인 결과를 후속 문서 커밋으로 남긴다.

**종료 조건:** R2-01~R2-09 완료, 연속 3거래일 무인 데이터·추천 갱신과 실제 Owner 사용 확인.
컨테이너 health와 데이터·화면의 성공을 따로 기록한다. R3 미완료만으로 R2를 미완료로 되돌리지 않는다.

관찰 기록의 최소 열은 `거래일 / 대상 커밋·모드 / 예약 실행 결과 / ETF 데이터 발행 기준일 /
추천 실행·발행 결과 및 입력 기준일·as_of / Owner 갱신 커버리지·기준일 /
API·화면 확인 / 실패·복구 여부 / 안전한 증거 링크`다.
원본 데이터·인증정보는 기록에 포함하지 않는다.

근거: [일일 수집 설치](../scripts/ops/install-kis-daily.sh),
[일일 실행](../scripts/ops/kis-daily-production.sh),
[V2 runbook](runbooks/owner-managed-equity-universe-v2.md),
[서비스 상태 계약](contracts/service-status-v1.md).

## 7. R3 — 실시간 시세 운영 검증

**목표:** 기존 WS 구현을 실제 시장 수신과 인증된 화면까지 인수한다.

- [ ] **R3-01 — 기존 승인과 설치 전제 대조.** ADR-0009 범위에서 실제 Owner·entitlement·grant·
  persistent state·현재 릴리스·당일 calendar/window 증거를 검증한다. 소스 승인·설치·수신 상태를 분리한다.
- [ ] **R3-02 — READY 1종목 실제 수신.** 정상 Owner 수요로 구독하고 실제 ACK, 계약상 47필드,
  새 수신 시각과 해제·종료를 확인한다. 조용한 장에서 미관측한 결과를 성공으로 추정하지 않는다.
- [ ] **R3-03 — 인증된 화면과 수명주기 검증.** 수신→SSE→가격 표시, 새로고침·로그아웃·페이지 이동·
  숨김/복귀 시 구독과 화면 동작을 확인한다. 원본 브라우저 동작을 조작한 결과로 미검증 항목을 대체하지 않는다.
- [ ] **R3-04 — 고정 30종목 인수.** 정상 membership·READY·Owner 수요를 갖춘 승인된 30종목에서
  실제 admission/ACK와 표시를 확인한다. fixture나 수동 SQL로 운영 인수를 만들지 않는다.
- [ ] **R3-05 — 장애·노후화·장 경계 검증.** 연결 단절·제한된 재연결·throttling·가격 노후화·
  장 종료와 다음 거래일 증거 갱신을 검증한다. WS 장애에 REST 실시간 시세로 대체하지 않는다.
- [ ] **R3-06 — 남은 라이브 판정 근거 확보.** 최신 공식 용량·다른 클라이언트의 동일 키 사용 등
  아직 미확인인 사실을 확인한다. ADR-0009의 제한된 활성화 예외와 최종 인수 조건을 혼동하지 않는다.
- [ ] **R3-07 — 최종 인수 기록.** `SOURCE_LOCAL_ACCEPTED`, `INSTALLED_OFF`, 실제 연결·수신,
  `LIVE_ACCEPTED`, `FINAL_ACCEPTED`의 근거를 각각 기록하고 계약의 최종 matrix를 만족한다.

**종료 조건:** R3-01~R3-07 완료. 1종목 pilot 성공과 전체 30종목 인수를 분리해 기록한다.
여기서 `LIVE_ACCEPTED`는 **실시간 시장 데이터**의 인수 상태이며 실거래 승인이 아니다.
호가 채널, 일봉 마지막 캔들의 실시간 변경, 과거 분봉·틱 데이터는 이 단계에 포함하지 않는다.

근거: [ADR-0009](decisions/0009-kis-owner-directed-websocket-activation.md),
[WS 계약](superpowers/specs/2026-09-21-kis-market-stream-contract.md),
[설치·복구 runbook](runbooks/kis-market-stream-operations.md),
[실제 접속 준비 기록](runbooks/kis-market-stream-live-readiness.md).

## 8. R4 — 추천 전용 백테스트

**목표:** Owner-beta 추천에 사용한 승인 artifact와 신호로 전략 성과를 재현한다.

- [ ] **R4-01 — 시뮬레이션 결정안 확정.** P0 문서의 모든 필드에 권장안·대안·결과 영향을 작성해
  Owner가 구체적인 버전 계약을 결정할 수 있게 한다. 일정·warm-up, 신호와 수익 구간, 체결·평가 가격,
  분할 조정, 누락, 수량·현금, 비용·슬리피지, 벤치마크, 회전율, 시각, 지표, 결과 해시를 모두 확정한다.
  현재 로드맵 채택을 이 세부 값의 승인으로 대체하지 않는다.
- [ ] **R4-02 — 승인 입력·결과 계약 구현.** 기존 5개 pin과 `OWNER_ONLY`, `PRICE_RETURN_ONLY`,
  `vendor_snapshot=true`, `strict_pit=false`를 유지하는 전용 Rust 경로·결과 계약을 구현한다.
  generic Nautilus/등록 READY 결과를 추천 성과로 재사용하거나 표시하지 않는다.
- [ ] **R4-03 — 첫 기준 전략 검증.** 승인된 계약으로 전략 1개와 벤치마크 1개를 먼저 구현하고
  신호·목표 비중·현금·체결·비용·평가의 작은 수작업 기준값과 대조한다.
- [ ] **R4-04 — 재현성·미래정보·무결성 검증.** 동일 입력 결과, 미래정보 차단, 분할 중복 반영,
  반올림·잔여 현금·누락, pin 변조와 사용자 격리를 검증한다.
- [ ] **R4-05 — 전용 실행·조회 연결.** 작업 큐·저장·API·Web 결과를 전용 계약에 연결하고 실패 시
  부분 성과를 노출하지 않는다. 구조가 바뀌면 아키텍처 도표와 근거·PNG도 같은 변경에 갱신한다.
- [ ] **R4-06 — 기준 전략 확대·출시.** 검증된 첫 전략에서 기존 5개 기준 전략으로 확대한다.
  전략별 결과 근거, 승인된 지표·비용·벤치마크 설명, 운영 설치·Owner 확인까지 기록한다.

**종료 조건:** R4-01~R4-06 완료. 추천과 입력·신호의 일관성, 재현성, 지표 의미와 운영 결과 확인.
가격수익률 검증을 배당 포함 총수익이나 엄격한 PIT 데이터 검증으로 표현하지 않는다.
구현의 현재 차단 요인은 구체적인 P0 계약 미확정이다. 결정안 작성은 바로 진행할 수 있다.

근거: [P0 결정 문서](superpowers/plans/2026-08-28-owner-beta-backtest-p0-owner-decision.md),
[Phase A 분석](superpowers/plans/2026-08-24-owner-beta-backtest-phase-a.md).

## 9. R5 — Paper 운영

**목표:** 검증된 추천 전략을 내부 모의 원장에서 매일 운용하고 백테스트와 차이를 설명한다.

- [ ] **R5-01 — Paper 입력·운영 계약 정리.** 승인 추천의 5-pin 입력과 기존 Paper 경로의 차이를
  확인하고 필요한 연결·승격 조건을 확정한다. 기존 코드가 있다는 이유로 Owner-beta 활성화를 완료 처리하지 않는다.
- [ ] **R5-02 — 무인 세션 증거 checker 구현.** 기존 정책의 3회 연속 무인 EOD·추천 세션 증거를
  검증하는 경로를 구현하고, 실패·수동 실행·다른 입력을 성공으로 인정하지 않는지 확인한다.
  R2의 관찰 기록은 이 checker의 계약을 실제로 만족할 때만 재사용한다.
- [ ] **R5-03 — preview→apply→settle 검증.** Owner 1명의 내부 모의 원장에서 입력 계보,
  재실행 안전성과 잔고·성과 정합성을 검증한다. KIS 계좌·잔고·주문 API와 연결하지 않는다.
- [ ] **R5-04 — 운영 전환과 복구.** 선행 증거·정책을 충족한 뒤 지원되는 활성화 경로로 전환한다.
  재시작·백업 복원 시 중복 반영 방지와 기존 비활성 상태로의 복구를 검증한다.
- [ ] **R5-05 — 실제 무인 운용 인수.** 최소 한 전략의 일별 실행·성과를 관찰하고 백테스트 대비
  신호·체결·가격·비용 차이를 설명한다. 최소 3거래일 연속 무인 실행과 Owner 확인을 남긴다.

**종료 조건:** R5-01~R5-05 완료. Paper 활성화 전의 EOD·추천 증거와 활성화 후 모의 운용 증거를 구분한다.
실거래 전환은 이 단계의 자동 후속 작업이 아니다.

근거: [Owner 베타 출시 계획](superpowers/plans/2026-08-23-owner-readonly-beta-launch.md),
[운영 helper의 Paper 제한](../scripts/ops/README.md).

## 10. 후속 범위와 재개 조건

아래는 `DEFERRED` 목록이다. 실행 단계로 옮길 때 ID를 유지하고 관련 승인·계약·완료 기준을 추가한다.

| ID | 후속 범위 | 재개 조건 |
| --- | --- | --- |
| D-01 | 정식 KOSPI200/KOSDAQ150·PIT 재무 기반 후보 연구 | 실제 공급원·데이터 권리·과거 구성 및 시점 계약·운영 feed 확보. Owner V2와 별도 범위 |
| D-02 | 추가 가격 공급원 | 독립 교차검증/대체 목적·우선순위·실패 동작의 명시적 승인과 공식 문서·테스트 |
| D-03 | 과거 분봉·틱·호가 기반 전략 | 데이터 권리·수집 예산·보관 및 전략 계약 확정. 현재 시장 WS 승인으로 자동 허용되지 않음 |
| D-04 | LEAN·미국 시장 등 연구 엔진 확장 | 요구사항서의 도입 조건과 실제 제품 필요 확인 |
| D-05 | Owner 실거래 | Owner가 별도의 실거래 프로젝트를 명시적으로 시작하고 계좌·주문·리스크 범위를 새로 확정 |

모든 단계는 [저장소 안전 경계](../AGENTS.md)를 따른다. KIS/OpenDART allowlist, KIND 수동 캡처,
데이터 권리, 비밀 제외 원칙은 유지한다. 승인되지 않은 API나 벌크 수집을 로드맵 항목 해소 수단으로 추가하지 않는다.

## 11. 변경·완료 이력

작업 완료, 재개, 차단, 우선순위·완료 기준 변경 시 한 행을 추가한다. 상세 실행 기록은 링크로 남긴다.
이력의 순서는 날짜 오름차순이며, 이전 결과를 수정해 새 결과처럼 보이게 만들지 않는다.

| 날짜 (KST) | 작업 ID | 상태·범위 변경 | 이유·결정 | 커밋·검증 근거 |
| --- | --- | --- | --- | --- |
| 2026-10-12 | DOC-01 | TODO → DONE; R1~R5와 D-01~D-05 최초 등록 | Owner가 제안된 개발 순서와 변경·완료 추적을 채택. 상세 P0 값·추가 API·실거래 승인으로 확대하지 않음 | 이 기록을 추가한 문서 커밋; §3 기준선; AGENTS.md·STATUS.md 연결; 도입 커밋 설명의 문서 검증 |
| 2026-10-12 | R1-01~R1-09 | TODO 유지; 상세 실행 계획 추가 | Owner의 `$paseo-delegate-plan` 요청에 따라 7개 패키지·5개 wave와 소유 범위·검증 기준 작성. 구현·worker 실행은 미착수 | [R1 실행 계획](superpowers/plans/2026-10-12-r1-ci-recovery.md); 이 기록을 추가한 문서 커밋의 형식·링크·작업 ID 검증 |
| 2026-10-12 | R1-01 | TODO → DOING | Owner가 `$paseo-delegate` 실행 요청; WP-1 실패 재현·수정 명세 확정 시작 | 입력 `ac0a083`; [실행 기록](reviews/2026-10-12-r1-execution.md); 완료 근거는 아직 없음 |
| 2026-10-12 | R1-01; R1-02~R1-07 | R1-01 DONE; 수정 항목 DOING | WP-1 수용, 원인별 명세 확정. R1-03은 테스트 지원 feature를 CI에서 명시하며 실제 전체 테스트를 유지 | `ac0a083`; [기준선](reviews/2026-10-12-r1-baseline.md); [확정 실행 계약](superpowers/plans/2026-10-12-r1-ci-recovery.md); Docker/SQL/smoke와 최종 workspace 실행은 아직 미확인 |
| 2026-10-12 | R1-02~R1-07 | DOING 유지; 후속 Rust·DB·Web 범위 확정 | 개별 검증에서 추가 진단과 기존 테스트 계약 불일치 발견. DB 리뷰 결함 수정, 고정 QA 포트 일치, Web 서버 소유 증명과 WS 전용 회귀 보완. 전체 검사를 축소하지 않음 | `ac0a083` 기반 작업 트리; [실행·검증 기록](reviews/2026-10-12-r1-execution.md), [Rust](reviews/2026-10-12-r1-rust.md), [DB](reviews/2026-10-12-r1-postgres.md), [runner](reviews/2026-10-12-r1-runner.md); 동일 최종 커밋·필수 CI·smoke는 아직 미확정 |

| 2026-10-12 | R1-02~R1-08 | 수정·로컬 검증과 독립 소스 리뷰 수용; 동일 후보 커밋의 hosted 검증으로 이동 | Rust·DB·Python·Web 135회·기능 smoke 통과. 기존 실패·skip 이력은 보고서에 보존하고 R1 완료 체크는 hosted 결과 이후 판단 | 이 기록을 추가한 후보 커밋; [실행 기록](reviews/2026-10-12-r1-execution.md), [Web](reviews/2026-10-12-r1-web-validation.md), [workspace](reviews/2026-10-12-r1-workspace-validation.md), [smoke](reviews/2026-10-12-r1-smoke-validation.md) |

후속 행에는 `날짜 / ID / 이전 상태 → 이후 상태 / 이유·결정 / 대상 커밋·검증 링크·결과`를 넣는다.
문서를 갱신한 뒤 상단의 최종 갱신일·다음 착수 항목·진행 중인 항목, §4 단계 상태, 해당 체크박스와
차단 조건이 서로 일치하는지 확인한다.
