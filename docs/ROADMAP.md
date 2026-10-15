# Lagrange 개발 로드맵

> **앞으로의 개발 우선순위와 진행 상태는 이 문서를 기준으로 관리한다.**
> Owner가 2026-10-12에 아래 개발 방향과 지속적인 완료·변경 추적을 채택했다.
> 단계 이름 `R1`~`R5`는 이 실행 로드맵의 번호이며, 요구사항서의 Phase 0~4와 다르다.

| 관리 항목 | 현재 값 |
| --- | --- |
| 최종 갱신일 | 2026-10-16 (Asia/Seoul) |
| 최초 코드 기준선 | `fce9b90542cf9b1a911d46e4c4d3530db7efba4c` (2026-10-10) |
| 첫 출시 목표 | 매 거래일 최신 데이터로 종목과 ETF 전략을 확인할 수 있는 Owner 전용 연구 베타 |
| 현재 실행 우선순위 | R1 완료 → R2 완료까지 Ralph 지속 실행(Owner 2026-10-15 지시). 작은 패키지 완료 후에도 다음 의존 작업을 이어가며 R3로 우선순위를 전환하지 않는다. 실제 Owner 확인·연속 3거래일은 증거가 있어야 완료로 판정 |
| 상세 실행 계획 | [R2 Paseo 위임 계획](superpowers/plans/2026-10-13-r2-daily-research-beta.md) — F1/F2 `f8b9296` CI·리뷰 수용. WP-4는 [U2 후속 계획](superpowers/plans/2026-10-14-r2-daily-input-implementation.md)으로 재분해. [실행 기록](reviews/2026-10-13-r2-execution.md). [R1 계획](superpowers/plans/2026-10-12-r1-ci-recovery.md)은 완료 |
| main 통합 | R1 재개 기준은 `b0a2579`(C3와 문서 6개 차이). 2026-10-14 재조회한 remote main은 별도 개발 속도 개선의 `1286e73`로 전진. R2는 `b0a2579`에서 분기한 `excited-dolphin`에서 검증하며 그 개선 작업의 통합은 이번 범위 밖 |
| 다음 착수 항목 | Wave58 superseded 취소 소스 검토·567pins·새14개 실제DB/총24 runner 고정. exact-candidate CI를 한 번 실행해0061·기존 gate 전체 대조 후 producer/admitter 연결. I5–I7·운영·Owner·3거래일·최종 C_R2 계속 진행. |
| 진행 상태 | R1-01~09 및 전체 R1 DONE. C3 `862fa95`의 CI·smoke, 실제 12개 이미지·V2 manifest, 기존 Owner V2 복귀 검증 완료. main `b0a2579`의 기존 CI·smoke도 최종 성공 회수 |
| 판단이 필요한 항목 | R2 U2 계약은 2026-10-14 채택됨. 실제 catch-up 범위·예산과 G4 운영 적용 조건을 구체화해야 함. 기존 KIS 개인 이용 권리는 재확인하지 않음. R4-01 전용 백테스트의 시뮬레이션 계약은 미확정 |

## 1. 목표와 문서의 역할

첫 출시에서 Owner는 정상 로그인 후 종목을 추가하고, 준비 상태를 확인하며, 최신 일봉·신호·ETF 추천을
볼 수 있어야 한다. 데이터가 늦거나 실패하면 기준일과 원인이 드러나야 하고, 다음 수집과 복구 결과를
추적할 수 있어야 한다. 그다음 실제 실시간 시세를 인수하고, 추천 전략의 성과를 같은 입력으로 검증하는
전용 백테스트, Paper 순으로 확장한다.

- 이 문서: 현재 우선순위, 작업 체크리스트, 완료 기준, 차단 사유, 변경 이력.
- [STATUS.md](STATUS.md): 최신 진행 요약과 과거 날짜별 구현·운영 기록. 과거 기록의 “현재”는 해당 기록 당시를 뜻한다.
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

| 단계 | 사용자에게 생기는 결과 | 현재 상태 | 선행 조건 |
| --- | --- | --- | --- |
| R1 | 검증 가능한 개발·배포 기준선 | DONE — 2026-10-13, C3 `862fa95` | [동일 후보 CI·이미지·manifest·복귀 근거](reviews/2026-10-13-r1-c3-images.md) 확보 |
| R2 | 매 거래일 사용할 수 있는 일봉·신호·추천 베타 | DOING — 진단·F1/F2 검증 완료, U2 채택·설계 구현 | R1 통과. 실제 입력·운영·Owner 사용·3거래일 관찰은 별도 인수 |
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
- [x] **R1-02 — Rust format·Clippy 복구.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted workspace format과 all-target/all-feature strict Clippy가 통과했다.
  로컬 feature 조합 6개 검사와 보충 검증도 수용했다.
  [최종 인수](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance),
  [Rust 검증](reviews/2026-10-12-r1-workspace-validation.md), [lint 수정·검증](reviews/2026-10-12-r1-rust-lint-validation.md).
- [x] **R1-03 — Rust workspace 테스트 복구.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted `cargo test --workspace --locked --no-fail-fast --features kis-client/test-support` 통과.
  235개 결과 요약 합계는 2,886 pass·실패 0·기존 ignored 6·filtered 180이며, 고유 테스트 수가 아니다.
  chart HTTP 6개 통과는 별도 실행이다. 기존 ignored의 이름·사유와 추가 역할 검증은
  [최종 인수](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance),
  [workspace 집계](reviews/2026-10-12-r1-workspace-validation.md),
  [C2 pure 70·relay 13·restart](reviews/2026-10-12-r1-c2-fixture.md),
  [runtime 18](reviews/2026-10-12-r1-runtime-validation.md)에 구분했다.
- [x] **R1-04 — PostgreSQL 초기화·업그레이드 검증.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted validator `APPROVED`, DB-gated 10개 선택과 no-SKIP guard 통과.
  staged 0038→0041, 전체 59개 migration·동일 checksum·no-op 재실행과 역할 경계를 검증했다.
  로컬 후속 126 pass·실패/ignored/SKIP 0은 별도 근거이며 hosted 개별 테스트 수는 공개되지 않았다.
  [최종 인수](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance),
  [PostgreSQL 검증](reviews/2026-10-12-r1-postgres.md), [DB 계약 수정](reviews/2026-10-12-r1-db-contract-repair.md).
- [x] **R1-05 — Web 검증 환경 복구.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted clean install·Chromium·OpenAPI·lint·typecheck·61파일/487개 unit·build와 공식 E2E wrapper 통과.
  회귀 32개 및 `state=passed exit=0 coverage=complete`를 확인했다.
  같은 소스의 로컬 wrapper는 135회(121+13+별도 owner-empty 1), 134개 고유 synthetic identity,
  실패/skip/retry/flaky 0을 검증했다. hosted 개별 실행 행·재시도 수는 업로드되지 않아 별도 관측으로 주장하지 않는다.
  [최종 인수·이전 실패](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance),
  [Web 상세](reviews/2026-10-12-r1-web-validation.md),
  [별도 격리 실제 API 검증](reviews/2026-10-12-r1-actual-api-validation.md).
- [x] **R1-06 — Python 계약·research smoke 복구.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted pytest 482 pass·기존 skip 10·warning 7, backup 정책 단계 및 별도 research smoke functional PASS.
  실행기 소유권 수정은 standalone 32개·공유 multiprocessing 34개와 독립 리뷰로 검증했다.
  skip 사유는 불변 source의 Windows 2개·root metadata 8개와 대응하며 quiet run에서 새로 출력된 사유는 아니다.
  로컬 cold-cache smoke·자원 정리는 별도 검증했다. hosted smoke의 개별 자원 삭제는 독립적으로 관측되지 않았다.
  [최종 인수·한계](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance),
  [로컬 기능 smoke](reviews/2026-10-12-r1-smoke-validation.md), [Python 기록](reviews/2026-10-12-r1-runner.md).
- [x] **R1-07 — 정책 검사 입력 복구.** 완료 2026-10-13; 대상 C3 `862fa95`.
  hosted policy의 모든 단계와 CI 계약 10개 통과. 격리 Compose 입력과 누락 거부 회귀를 유지하고
  운영 credential·production 필수 입력 규칙을 바꾸지 않았다.
  [최종 인수](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance), [정적 계약](reviews/2026-10-12-r1-runner.md).
- [x] **R1-08 — 통합 검증 확정.** 완료 2026-10-13;
  대상 `862fa954c16d28a617271fe6f613d34226e2d558`.
  [CI 38278947953](https://github.com/sihioov/Lagrange/actions/runs/38278947953)의 8개 job과
  [research smoke 38278966315](https://github.com/sihioov/Lagrange/actions/runs/38278966315) 모두 success.
  같은 SHA의 checkout·실제 테스트 실행·기능 PASS를 확인했다. C/C2 실패·취소·기존 skip과
  세부 출력 한계는 [후보별 인수 기록](reviews/2026-10-12-r1-acceptance.md)에 보존했다.
  이 결과를 추가하는 후속 문서 커밋은 검증 대상 C3와 구분한다.
  2026-10-13 `1fe09ef`까지의 작업을 main에 fast-forward로 통합했다.
  [통합 기록](reviews/2026-10-12-r1-acceptance.md#main-integration--2026-10-13)의 문서 갱신은
  기존 C3 검증 대상이나 R1-09의 실제 이미지·manifest 완료 조건을 바꾸지 않는다.
  같은 날 main `b0a2579`의 기존 CI `38336666454`(8개 job)와 smoke
  `38336666468`(functional PASS)도 최종 성공으로 회수했다. 실제 checkout SHA와
  원본 로그를 [재개 기록](reviews/2026-10-13-r1-resumption.md#기존-main-ci-회수)에
  별도로 대조했으며 C3의 검증·빌드 revision은 유지한다.
- [x] **R1-09 — 배포 후보 준비.** 완료 2026-10-13; 대상 C3 `862fa954c16d28a617271fe6f613d34226e2d558`.
  공식 순차 빌드 12개·gate 49개 모두 PASS, 실제 image ID·revision·saved-byte 검증 기록과
  root:root 0600 V2 manifest를 대조했다. manifest SHA-256은
  `041fdaf1ddfe8746df3e4eacf3a180435a0cd2364952e3036da8c3e0e4ccb64f`다.
  16:10:57 KST 빌드 PASS, 16:11:03 기존 Owner V2 healthy 복귀와 16:11:55 작업 정산을
  [최종 근거](reviews/2026-10-13-r1-c3-images.md)로 확인했다. migration/복구 소스 검토도 수용했다.
  검증 커밋을 보존한 별도 checkout과 정확한 명령·출력 경로·해소 조건을
  [배포 준비 인계](reviews/2026-10-12-r1-release-readiness.md)에 정리했다.
  담당: 운영 결정의 Owner와 실행 코디네이터. 승인된 정지·정산과 fresh C3 attestation으로
  build admission을 충족하고 서비스별 순차 빌드·공식 manifest 검증을 수행했다.
  기존 예외 재사용·서비스 전환·운영 migration은 수행하지 않았다. 실제 운영 적용·결과는 R2에 남는다.
  2026-10-13 11:38 KST [재개 관측](reviews/2026-10-13-r1-resumption.md): sudo 인증 실패,
  research-worker exited2/unhealthy·나머지 10개 running/healthy 및 Owner V2 실행 중을 새로 확인했다.
  종료 원인은 미판정이다. exact-C3 plan/preflight와 보존 증거 14개 검증은 통과했으나
  당시 C3 image tag 12개 모두 부재, protected manifest 부재는 미확인이었다.
  **14:14~14:16 KST 후속 확인:** Owner가 72시간 sudo 설정을 직접 적용했고 비대화형 UID 0,
  만료 timer(10월 16일 14:11:48 KST), 보호된 부모·state 경로와 C3 manifest 부재를 확인했다.
  manifest 경로를 포함한 root plan/preflight도 통과했다. 새 관측에서도 research-worker exited2와
  Owner V2 running으로 당시 health 조건은 미충족이었다. 그 시점에는 이미지 12개·manifest가 없었으며
  [sudo 적용 후 근거](reviews/2026-10-13-r1-resumption.md#owner-sudo-적용-후-root-전제-해소)를 보존한다.
  후속 진행 승인과 읽기 전용 작업·lease 확인은 [유지보수 실행 기록](reviews/2026-10-13-r1-resumption.md#승인된-owner-v2-유지보수와-c3-빌드)에 보존한다. 위 완료 판정은 실제 이미지·manifest와 복귀 검증 후 내렸다.

**종료 조건:** R1-01~R1-09 완료, 필수 CI·smoke 통과, 동일 커밋의 검증·배포 후보 근거 확보.
규칙 변경이 필요하면 원래 검사의 목적을 충족하는 검증과 변경 이유를 함께 남긴다.

근거: [CI workflow](../.github/workflows/ci.yml),
[smoke workflow](../.github/workflows/research-smoke.yml),
[배포·백업 runbook](runbooks/production-release-and-backup.md).

## 6. R2 — 일일 연구 베타 완성

**목표:** 예약 수집부터 실제 Owner 화면까지 매 거래일 일관되게 동작한다.

[상세 실행 계획](superpowers/plans/2026-10-13-r2-daily-research-beta.md)을 2026-10-13에 작성했다.
`$paseo-delegate`로 실행할 10개 패키지의 소유 범위·의존성·초기 복잡도·모델·검증 기준을 정의했다.
첫 묶음은 읽기 전용 현재 상태 수집과 소스 계약 대조이며, 결과 통합 뒤 수정 범위와 운영 조치를 확정한다.
계획 작성 후 Owner가 실행을 요청했다. WP-1A/2를 수용하고 독립 소스 수정에 착수했으며 [실행 기록](reviews/2026-10-13-r2-execution.md)에
범위·담당·수용 상태를 남긴다. 새 입력 계약이나 추가 운영 조치의 승인은 기존 권한과 구분한다.

- [ ] **R2-01 — 실행 경로 진단.** 보호된 릴리스와 설치된 모드에 맞는 daily unit·timer·작업 결과를
  확인한다. 비활성 이유와 마지막 성공·누락 구간을 구분하고 상주 daemon과 중복 실행하지 않는다.
  DOING — 2026-10-14, 10월 13일 current `fce9b90`/`owner_daily`, daily timer 비활성, 실행 manifest 대상 3개 일치·5개 불일치 관측. 23:51 KST READ ONLY 집계·날짜 artifact의 28세션 누락과 별도 8월 31일 발행 복구 후보 검토를 수용. [DB 기록](reviews/2026-10-13-r2-db-baseline.md), [복구 범위·한계](reviews/2026-10-13-r2-catch-up-readiness.md); 실행 직전 상태·범위 판정은 남음.
- [ ] **R2-02 — 거래일·시간창 운영 책임 확정.** 공식 당일 calendar/window 증거의 공급 경로,
  확인 담당, 준비 마감과 부재 시 동작을 기록한다. 기존 단일 calendar 요청·캐시 정책을 유지한다.
  DOING — 2026-10-13, WP-2에서 날짜 artifact·credentialed calendar·당일 window의 기존 경로와 책임을 대조.
- [ ] **R2-03 — 승인된 일일 실행 복구.** 원인 수정과 R1 검증 후 commit-pinned installer·공식 helper로
  일일 실행을 복구한다. 복구할 날짜·요청 예산을 먼저 확인하고 승인 범위 밖의 과거 일괄 수집은 추가하지 않는다.
  DOING — 2026-10-14, WP-3/F1: pre-close 필터 소실 수정, fake-host·hostile 환경 회귀 PASS. 보정본 `f8b9296`의 [CI](https://github.com/sihioov/Lagrange/actions/runs/38420816275) 성공·[독립 P2 리뷰](reviews/2026-10-14-r2-fixture-review.md) 종료; 실제 수집·운영 활성화 미실행.
- [ ] **R2-04 — ETF EOD와 추천 갱신 검증.** Raw, 정규화, Curated, 발행과 API의 기준일·출처를
  연결해 확인한다. Owner가 사용하는 ETF 추천도 매 거래일 승인된 입력으로 실행·성공·발행되며,
  입력 기준일과 추천 `as_of`가 기대 거래일에 일치하는지 대조한다. 봉인된 역사 artifact의 지원
  기준일 이후를 제공할 수 없다면 새 입력의 승인·갱신 경로를 먼저 확정하고 그동안 이 항목을
  `BLOCKED`로 남긴다. 역사 price-only artifact를 generic READY로 간주하거나 과거 추천을 최신으로 표시하지 않는다.
  DOING — 2026-10-14, Owner가 [U2 계약](reviews/2026-10-13-r2-daily-input-decision.md)을 채택했다. [D5/D6 설계](reviews/2026-10-14-r2-daily-input-amendments-review.md)와 [고정 wire 명세](contracts/owner-beta-daily-etf11-input-v1.md)를 기준으로 I1a/I0a 및 I0b/I1b-S1·호출부 보정을 진행했다. 후보c0ac599의 [최종 CI](reviews/2026-10-14-r2-caller-role-raw-ci.md)에서7개 실행 job/required,49개 Rust·chart6 직접 PASS, Python suite485 pass/10 skip/7 warnings, Web/PG59 통과를 대조해 부분 소스와 hosted 검증을 인수했다. 과거929fcd6 부분 성공과 f9 실패·DB3/Raw5 및 후속 parser 지적/수정은 실행 기록과 변경 이력에 보존한다. 직접 로그인 helper의 actor GUC·연결 수·권한 거부, negative bootstrap/rollback,0060·전체 snapshot/입력·운영·Owner 인수는 남는다. 다음 bounded I0c/S2-D는 [후속 계획](superpowers/plans/2026-10-14-r2-daily-input-implementation.md)을 따른다.
  이전 검증 — 2026-10-15, I2 migration0060 구현·격리 hosted 검증 완료 — `dee78c4`의 [CI 38696988981](reviews/2026-10-15-r2-daily-sql-c03-ci.md) attempt1 전체 성공. 249pins/204blobs/7checkout, DB32·세부73·실제 DB12단계와 회귀를 대조해 [I2를 범위 한정 인수](reviews/2026-10-15-r2-daily-sql-acceptance.md)했다.
  이전 검증 — 2026-10-15, `302f29a`의 [I1b-S4 복원 인수](reviews/2026-10-15-r2-daily-provider-reconstruction-acceptance.md) 완료: CI38725299504 attempt1 전체 성공, 새32개·회귀·PG·strict lint와276pins/223blobs/7checkout 대조. 다음은 전체 소스 탐색/변경 감지, protected prefix와 artifact/admission/supply. 앞선 `eb0e839`의 [I1b-S3 원본 추적·달력 증명](reviews/2026-10-15-r2-daily-evidence-lineage-acceptance.md) 구현·hosted 검증 인수. CI 38711965495 attempt1의 8개 job 성공, 262pins/215blobs/7checkout·새22개·기존 정규화37개와 I2/R2 회귀를 대조했다. 다음은 가격·기준정보·기업행사 복원, 전체 source frontier와 artifact/admission/supply다. 전체 I1b, I3–I7·최종 독립 통합 리뷰·운영 UID/ACL·installer 파일시스템 rollback·Owner·연속 3거래일·최종 C_R2는 미완료이며 관측 기간은 아직 시작하지 않았다.
  최신 검증 — 2026-10-16, `bd33596` / CI38760423300 attempt1 전체 성공. [I3·S5c/def 범위별 인수](reviews/2026-10-16-r2-test-correction-ci-acceptance.md): 423pins/302blobs/7checkout, 일일 계산21·artifact/continuation63·서비스 상태38·기존 회귀·PG13단계와 발행8개 대조. 실제 승인 base 전체 통합·독립 검토는 별도이며 I4a 예약/승인 cycle 연결을 계속한다. 운영·Owner·3거래일은 미시작.
  후속 — 2026-10-16, [Wave53 전체 회수](reviews/2026-10-16-r2-daily-settlement-ci.md): 일일 스케줄14·실패 정산/복구12·실제DB16단계 통과, 웹 E2E 실패로 전체 후보는 미인수. [Wave54 성공 저장·진단 보완](reviews/2026-10-16-r2-daily-success-source.md)은 `47c7dbc` / CI38771904166 전체 성공으로 [범위별 인수](reviews/2026-10-16-r2-daily-success-ci.md). 다음은 한도가 있는 봉인 입력 체인 로더와 전체 런타임 연결이다.
  이전 검증 — 2026-10-16, [Wave56 전체 회수](reviews/2026-10-16-r2-daily-runner-ci.md): 후보9343706의 새12개·기존 필수 Rust·PG17개 lane 통과, strict lint와 collector DB 초기화1개 실패로 전체 미인수. [Wave57 테스트 보정](reviews/2026-10-16-r2-daily-runner-lint-correction.md)은 운영 소스 변경 없이 재검증한다. 이전 Wave55 인수를 보존하며 전체 I4·실제 승인 base 통합·독립 리뷰·운영·Owner·관찰은 미완료.
  최신 검증 — 2026-10-16, [Wave57 인수](reviews/2026-10-16-r2-daily-runner-correction-ci.md): c4f35bb의 8jobs/logs·548pins/328blobs/7checkout 및 runner12·collector 실제DB3·이전 Rust/PG/Web 전부 통과. Wave58 안전한 superseded 취소를 진행한다. 전체 I4/실제 승인 입력 통합/독립 리뷰/운영/Owner/3거래일은 미완료.
- [ ] **R2-05 — Owner V2 갱신 검증.** 정상 등록된 종목의 수집·generation·artifact·신호·차트가
  같은 입력을 가리키는지 확인한다. ETF 공용 기준일과 Owner 종목 기준일을 각각 검증한다.
  DOING — 2026-10-14, WP-5/F2 독립 분기: 60초 cadence·동시/late-calendar 회귀와 권리 기간 fixture 보정본 `f8b9296` CI 성공·독립 P2 리뷰 종료. 10월 13일 운영 READY 1개·DB 신호 기준일 10월 6일 관측; 실제 갱신·API/Owner 화면 인수와 WP-4 통합은 남음.
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
  DOING — 2026-10-15, [I2 인수](reviews/2026-10-15-r2-daily-sql-acceptance.md)를 보존하고 `eb0e839`의 [I1b-S3 인수](reviews/2026-10-15-r2-daily-evidence-lineage-acceptance.md)를 추가했다. CI 38711965495 attempt1 전체 성공과 새22개·기존 정규화37개·DB32/73 등 회귀를 확인했다. 남은 I1b 복원/admission/supply, I3–I7·최종 독립 통합 리뷰·운영 UID/ACL·installer 파일시스템 rollback·Owner·연속 3거래일·최종 C_R2는 미완료다. 운영 관측 기간은 아직 시작하지 않았다.

  이전 검증 — 2026-10-16, [Wave51 인수](reviews/2026-10-16-r2-test-correction-ci-acceptance.md)를 추가했다. Wave53 실제 역할 스케줄/실패 복구 검증은 통과했으나 웹 E2E 실패가 남았다. Wave54 일일 성공 저장과 진단 보완을 진행하며 최종 릴리스·Owner·3거래일 인수는 열려 있다.

  이전 검증 — 2026-10-16, Wave54/55 범위별 인수는 보존. [Wave56](reviews/2026-10-16-r2-daily-runner-ci.md) 전체 실패를 회수하고 Wave57 테스트 fixture 보정을 진행한다. 최종 릴리스, 실제 승인 입력 전체 경로, I5–I7, 독립 통합 리뷰·운영·Owner·3거래일은 계속 열려 있다.

  최신 검증 — 2026-10-16, [Wave57 인수](reviews/2026-10-16-r2-daily-runner-correction-ci.md): c4f35bb의 8jobs/logs·548pins/328blobs/7checkout 및 runner12·collector 실제DB3·이전 Rust/PG/Web 전부 통과. Wave58 안전한 superseded 취소를 진행한다. 전체 I4/실제 승인 입력 통합/독립 리뷰/운영/Owner/3거래일은 미완료.

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
| 2026-10-12 | R1-04·R1-05·R1-07·R1-08 | 첫 후보 hosted 실패로 제한된 후속 수정; DOING 유지 | 깨끗한 checkout의 누락 입력·target 생성 전제와 DB rollback 오류가 로컬 통과 뒤 발견됨. Python은 runner shutdown으로 결과 미완료 | `79da119`; [후보별 인수 기록](reviews/2026-10-12-r1-acceptance.md). 기존 로컬 성공과 hosted 실패를 구분 |
| 2026-10-12 | R1-03~R1-08 | 후속 수정의 로컬 검증·리뷰 수용; hosted 검증 대기 | 7개 소스 파일 보정, DB 126개·runner 31개·artifact library 196개·CI 계약 10개 및 해당 strict Clippy 통과. 이전 실패 원인의 관측 한계 유지 | 이 기록을 추가한 후보 C2; [후보별 인수 기록](reviews/2026-10-12-r1-acceptance.md) |
| 2026-10-13 | R1-02~R1-08 | C2 결과 회수; Python·Web 후속 수정으로 DOING 유지 | Rust·DB·policy 및 기능 smoke 통과. Python은 중단, Web은 cleanup 회귀 실패. 신호를 차단한 격리 재현으로 실행기 소유권 결함 확인 후 두 Python 파일 수정·리뷰·검증 진행 | `f5dff91`; [C2 결과와 후속 근거](reviews/2026-10-12-r1-acceptance.md), [실행 기록](reviews/2026-10-12-r1-execution.md) |
| 2026-10-13 | R1-09 | BLOCKED; 실제 이미지·manifest 생성 전 외부 전제 기록 | 공식 plan·preflight는 통과했으나 root 인증과 운영 health 조건 미충족. 새 후보의 CI를 먼저 완료하고 구체적인 실행안을 인계 | C2의 읽기 전용 준비 결과; [배포 전제](reviews/2026-10-12-r1-acceptance.md); 운영 변경·예외 승인 없음 |
| 2026-10-13 | R1-05·R1-06·R1-08 | 두 파일 후속 수정의 로컬 검증·독립 리뷰 수용; C3 hosted 검증으로 이동 | runner32·공유34·전체 Python482/skip10·backup6과 새 Web135회 통과. 기존 실패와 skip/환경 한계 보존 | 이 기록을 추가한 후보 C3; [로컬 인수·hash 근거](reviews/2026-10-12-r1-acceptance.md) |
| 2026-10-13 | R1-02~R1-08; R1-09 | R1-02~08 DONE, R1-09 BLOCKED; 전체 R1 미완료 | 같은 C3의 CI 8개 job·기능 smoke 성공. 기존 skip·로그 한계를 구분하고, 실제 이미지·manifest는 root 인증·빌드 health 해소 전 미생성 | `862fa95`; [최종 인수](reviews/2026-10-12-r1-acceptance.md#candidate-c3-hosted-acceptance), [배포 준비 인계](reviews/2026-10-12-r1-release-readiness.md); 이 결과는 후속 문서 커밋에 기록 |
| 2026-10-13 | R1-01~R1-09 | main 통합 완료; R1-01~08 DONE·R1-09 BLOCKED 유지 | Owner 요청으로 `1b8dca3` → `1fe09ef` fast-forward 후 진행 문서 갱신. C3 코드·workflow·다이어그램과 검증 근거를 보존하며 운영 빌드·배포는 미실행 | [main 통합](reviews/2026-10-12-r1-acceptance.md#main-integration--2026-10-13), [실행 기록](reviews/2026-10-12-r1-execution.md#main-통합과-진행-문서-갱신--2026-10-13); 문서 갱신은 이 기록을 추가한 커밋 |
| 2026-10-13 | R1-09 | BLOCKED 유지; C3 입력·공식 plan/preflight 재검증과 현재 운영 전제 기록 | 최신 main `b0a2579`는 인계와 동일. root 인증·ordinary health는 현재도 미충족, 12개 C3 image tag 부재. reader 정지 경로의 작업 중단·재개 영향과 별도 운영 결정 필요를 구체화 | [R1 재개 근거](reviews/2026-10-13-r1-resumption.md); 대상 C3 `862fa95`, 11:38 KST 고정 필드 관측, 인계 14개 hash 대조; build/apply·운영 변경·R2 미착수 |
| 2026-10-13 | R1-08·R1-09 | 기존 main CI·smoke 최종 결과 회수 완료; R1-09 BLOCKED 유지 | `b0a2579`의 CI 8개 job과 별도 functional smoke 모두 성공. 재실행 없이 실제 checkout 8개·원본 로그 대조. C3 이미지·manifest 검증 근거는 여전히 미확보 | [main 실행 인수](reviews/2026-10-12-r1-acceptance.md#resumption--existing-main-push-runs), [재개 기록](reviews/2026-10-13-r1-resumption.md); 두 worker 실제 idle·pending0 및 heartbeat 종료 확인 |
| 2026-10-13 | R1-09 | BLOCKED 유지; Owner 직접 실행용 72시간 sudo 설정 스크립트 준비 | 현재 계정 전용 권한과 만료 timer, 원자적인 만료 처리·철회 기능을 준비하고 실제 parser·구문·격리 14회귀·독립 소스 리뷰 수용. 실제 root 설정은 미적용 | [후속 sudo 준비](reviews/2026-10-13-r1-resumption.md#후속-owner-요청--직접-실행할-72시간-sudo-설정-준비); C3·운영 범위 유지, 실제 설치·72시간 만료 미검증 |
| 2026-10-13 | R1-09 | root 인증·보호 경로 전제 해소; health로 BLOCKED 유지 | Owner의 sudo 적용 후 UID 0·활성 만료 timer·보호 부모·C3 manifest/state 부재 확인. exact-C3 root plan/preflight 통과; 현재 research exited2·Owner V2 running으로 ordinary/drained 조건 모두 미충족 | [14:14~14:16 KST 후속 확인](reviews/2026-10-13-r1-resumption.md#owner-sudo-적용-후-root-전제-해소); 이미지 12개 부재, build·서비스 변경·R2 미착수 |

| 2026-10-13 | R1-09 | BLOCKED → DOING; 최대 12시간의 유지보수 승인 | 현재 작업·lease 정산 확인 뒤 기존 Owner V2만 정상 정지하고 fresh C3 attestation으로 순차 빌드, 완료·중단 뒤 같은 컨테이너 재개. research 시작·운영 배포·migration·새 provider 범위는 제외 | [유지보수 실행 기록](reviews/2026-10-13-r1-resumption.md#승인된-owner-v2-유지보수와-c3-빌드); 종료 증거 확보 전 미완료 |
| 2026-10-13 | R1-09·전체 R1 | DOING → DONE; R1-01~09 모두 완료 | 같은 C3의 실제 12개 이미지·공식 V2 manifest, 49개 gate PASS/OOM 0, 동일 Owner V2 정상 복귀와 READ ONLY 정산 확인. R2·개발 속도 개선은 미착수 | `862fa95`; [이미지·manifest·복귀 최종 인수](reviews/2026-10-13-r1-c3-images.md), 16:10:57 빌드 PASS·16:11:55 최종 상태 확인; 후속 문서 커밋은 이미지 revision과 구분 |
| 2026-10-13 | R2-01~R2-09 | TODO 유지; Paseo 실행 계획 작성 | 현황·계약 대조→조건부 수정→동일 후보 검증·리뷰→이미지/운영 준비·적용→실제 Owner·3거래일 관찰의 10개 패키지와 판정 단계를 정의. 실행 worker·운영 진단·코드/서비스 변경은 미착수 | 기준 `0519c03`; [R2 계획](superpowers/plans/2026-10-13-r2-daily-research-beta.md), 문서 형식·의존성·범위·링크 검사; 대상은 이 기록을 추가한 커밋 |
| 2026-10-13 | R2-01/02·전체 R2 | TODO → DOING | Owner 실행 요청으로 WP-1 호스트 증거 수집·DB 조회 준비와 WP-2 소스 계약 분석 시작. 새 SQL은 코디네이터 검토 전 미실행; 코드/운영 변경·provider 호출 없음 | 시작 `cea7baa`; fetch한 main은 `b0a2579`, 차이는 문서 9개. [R2 실행 기록](reviews/2026-10-13-r2-execution.md) |

| 2026-10-13 | R2-01/03/04/05 | G1 부분 수용; R2-03/05 DOING, R2-04 BLOCKED | current daily 비활성·실행 이미지 3일치/5불일치, 과거 obsolete timer 비활성 기록 대조. F1/F2 국소 수정 착수, 새 ETF 자동 승격·추천 계약 결정안 요청. 운영 변경 없음 | [실행 기록](reviews/2026-10-13-r2-execution.md), [운영 기준선](reviews/2026-10-13-r2-runtime-baseline.md), [U2 결정안](reviews/2026-10-13-r2-daily-input-decision.md); 소스/DB 검증은 진행 중 |

| 2026-10-13 | R2-03/05 | 소스 수정·부분 검증; DOING 유지 | F1 실제 backfill 선택 경로 재현과 fake-host self-test 통과. F2 monotonic 60초 재평가·cadence/DB 회귀 추가. production DB는 별도 READ ONLY 집계만 승인 | 이 소스 수정 커밋; [F1](reviews/2026-10-13-r2-daily-repair.md), [F2](reviews/2026-10-13-r2-owner-and-web.md), [조회 검토](reviews/2026-10-13-r2-db-probe-review.md). CI·독립 리뷰·운영 인수 미완료 |

| 2026-10-13 | R2-01/03/04/05 | 현재 DB 기준일·28세션 누락 확인; F1/F2 부분 CI·테스트 보정 진행 | READ ONLY 도구 문법 오류를 고쳐 59 migration 일치·ETF 8/28·Owner 10/6 집계 확보. 독립 리뷰 P2 두 건 수용; U2 채택·운영 복구는 대기 | `3ee453f` [CI](https://github.com/sihioov/Lagrange/actions/runs/38415230019), [실행 기록](reviews/2026-10-13-r2-execution.md), [DB 기록](reviews/2026-10-13-r2-db-baseline.md); 실제 배포·provider 미실행 |

| 2026-10-14 | R2-03/05 | 부분 후보 CI 성공; P2 fixture 두 건 보정 | 7개 실행 job·required, late-calendar/동시 DB와 cadence 회귀 통과. 환경 격리·연도 경계 보정의 후속 CI는 별도 | `3ee453f` [CI 근거](reviews/2026-10-13-r2-f1-f2-ci.md), [리뷰 판단](reviews/2026-10-13-r2-f1-f2-review.md); 운영 인수 미완료 |

| 2026-10-14 | R2-01/03/04/05 | DB 집계·정확한 복구 후보·효과 분석 수용; DOING/BLOCKED 유지 | 10월 13일 READ ONLY 결과·59 source checksum·28개 수집 후보와 8월 31일 발행 복구 후보를 오프라인 검증. 요청량 계산은 실행 승인·강제 quota가 아님 | [DB 기록](reviews/2026-10-13-r2-db-baseline.md), [복구 준비안](reviews/2026-10-13-r2-catch-up-readiness.md), [수용 판단](reviews/2026-10-13-r2-execution.md); fresh G4·U2 채택 남음 |

| 2026-10-14 | R2-03/05 | 보정본 소스·CI·독립 리뷰 수용; 운영 항목은 DOING 유지 | `f8b9296` CI 7개 실행 job·required 성공, workspace 2,892 pass/실패0/기존 ignored6, chart6 및 새 날짜 경계·late-calendar·cadence5 통과. P2 두 건 종료 | [CI 인수](reviews/2026-10-14-r2-fixture-ci.md), [독립 리뷰](reviews/2026-10-14-r2-fixture-review.md); 대상은 `f8b9296`, 이 결과를 기록한 문서 커밋과 구분. U2·최종 후보·운영·Owner·3거래일은 미완료 |

| 2026-10-14 | R2-04 | BLOCKED → DOING; U2 계약 채택 | Owner의 “그걸로 진행해”에 따라 기존 v3를 보존하는 독립 검증 일일 입력과 전략별 opt-in 자동 추천 설계·구현 시작. 운영 조치·실제 입력·Owner·3거래일 인수는 후속 | [채택 기록](reviews/2026-10-13-r2-daily-input-decision.md), [WP-4 후속 계획](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 기준 `619634d`, 기록을 추가한 커밋 |

| 2026-10-14 | R2-04 | DOING 유지; 독립 설계 리뷰 수용·영향 범위별 보완 | 반복 휴장일 action/calendar 공급, grant/epoch, 발행 후 불변성, harness, byte 계약 6개 finding 수용. 순수 계약 I1a 준비를 DB/운영 설계와 분리 | D1 `e74934b7…` 및 [D3](reviews/2026-10-14-r2-daily-input-design-review.md); [후속 분해](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 소스 실행·DB·운영 성공을 주장하지 않음 |
| 2026-10-14 | R2-04 | DOING 유지; 입력 계약 수용·첫 구현 전환 | D4의 SinglePage marker·witness 집계·legacy hash 접점 보정, 7개 golden·17개 링크 직접 확인. I1a 순수 코드와 D6 교정안 검토 분리 | D4 `308b9909…`, D5 `8a8796d9…`; [launch 명세](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). Rust/DB/운영 검증 전 |
| 2026-10-14 | R2-04/09 | DOING 유지; D6 인수·격리 harness 준비 | 설계 finding 5개 해소 수용. 빈 cash code P2는 기존 validator와 대조해 `[A-Za-z0-9_-]{0,32}` 보정 예약. I1a 실행을 유지하고 별도 I0a source 착수 | [D6](reviews/2026-10-14-r2-daily-input-amendments-review.md) `b95b5e65…`; [정확한 follow-up·I0a 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). Rust/DB/운영 미검증 |

후속 행에는 `날짜 / ID / 이전 상태 → 이후 상태 / 이유·결정 / 대상 커밋·검증 링크·결과`를 넣는다.
문서를 갱신한 뒤 상단의 최종 갱신일·다음 착수 항목·진행 중인 항목, §4 단계 상태, 해당 체크박스와
차단 조건이 서로 일치하는지 확인한다.

| 2026-10-14 | R2-04/09 | DOING 유지; I0a 소스 회수·독립 리뷰 착수 | 역할 속성/멤버십 검사·scratch DB 전용 grant·직접 로그인 helper 네 파일을 회수하고 hash 고정. I1a와 D6-01 보정 순서는 유지 | [실행 기록](reviews/2026-10-13-r2-execution.md), [I0a-R 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 소스 인수·컴파일·SQL·실제 역할 검증은 미완료 |

| 2026-10-14 | R2-04/09 | DOING 유지; 미완료 worker 재개 | I1a/I0a-R idle 전환에 최종 보고가 없어 인수하지 않고 동일 scope로 재개. 고정 계약·네 harness hash 유지 | [회복 기록](reviews/2026-10-13-r2-execution.md); 원인 미확인, 소스/실행 인수 추가 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; I0a 소스 검토 수용·I1a-P2 시작 | I0a 네 파일 독립 리뷰 finding 없음 수용. I1a 원본 회수 후 빈 cash code 계약을 보정하고 7개 기존+515-byte 추가 Python golden 확인. 실제 Rust/DB는 미실행 | [I0a-R](reviews/2026-10-14-r2-isolated-role-harness-review.md) `6ad13d8a…`, D4 `fa052903…`, [실행 기록](reviews/2026-10-13-r2-execution.md); 해당 소스·진행 기록을 추가한 커밋, 운영 인수 아님 |

| 2026-10-14 | R2-04/09 | DOING 유지; pure wire 부분 후보 검증 전환 | I1a의 타입·canonical parse/encode/hash와 7개 기존+cash fixture 회귀 소스 구현. D6-01 소스 보정 확인, 독립 코드 리뷰 및 exact 후보 CI 준비 | 이 소스 커밋; [정확한 4-file hash·검증 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md). Rust·CI·최종 R2 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; 정확한 부분 후보 hosted CI 시작 | `779b3fa` feature push 후 같은 SHA 기존 실행 없음 확인, ci.yml 1회 dispatch. 독립 소스 리뷰와 결과 회수 병행 | [CI 38500926595](https://github.com/sihioov/Lagrange/actions/runs/38500926595), [실행 기록](reviews/2026-10-13-r2-execution.md); 16개 새 테스트·I0a 실행 결과는 아직 미확인 |

| 2026-10-14 | R2-04/09 | DOING 유지; 부분 후보 실패·6개 리뷰 finding 보정 | `779b3fa` CI clippy/workspace/PG/required 실패, 신규 16개 테스트 미실행. 역사 prefix 집계·timestamp·typed date·lint·경계 회귀·seal 크기 제한의 수정 범위를 확정하고 두 파일 보정 및 기존 실패 로그 진단 착수 | [CI 근거](reviews/2026-10-14-r2-daily-wire-ci.md), [독립 리뷰](reviews/2026-10-14-r2-daily-wire-review.md), [실행 기록](reviews/2026-10-13-r2-execution.md); source acceptance 보류, 중복 CI·운영 실행 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; CI 실패 원인 보강 수용 | clippy·chart·PG api-tenancy-rls 모두 같은 네 컴파일 오류. SQLSTATE는 terminal 원인이 아님을 phase·로그 hash·candidate source로 대조. I1a-F 수정 범위 유지 | [진단 근거](reviews/2026-10-14-r2-daily-wire-ci-diagnostics.md), [실행 기록](reviews/2026-10-13-r2-execution.md); 신규 테스트·후속 DB 단계 미실행, 운영 인수 아님 |

| 2026-10-14 | R2-04/09 | DOING 유지; 6개 finding 수정본 회수·새 후보 검증 전환 | 두 파일 hash를 고정하고 전체 manifest 집계·nanosecond·typed date·lint·경계·seal 크기 보정을 직접 대조. 24개 통합+5개 내부 회귀는 작성 상태 | 이 소스 커밋; [고정 입력·검증 brief](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md). 독립 리뷰·CI·운영 인수 미완료 |

| 2026-10-14 | R2-04/09 | DOING 유지; 보정 후보 CI 1회 시작 | `02a400e` remote SHA 및 기존 실행 부재 확인 후 ci.yml dispatch. Sol/high 독립 보정 리뷰·Luna/medium CI 회수 병행 | [CI 38506113191](https://github.com/sihioov/Lagrange/actions/runs/38506113191), [실행 기록](reviews/2026-10-13-r2-execution.md); 검증 대상은 source commit이며 후속 문서 커밋과 구분 |

| 2026-10-14 | R2-04/09 | DOING 유지; 보정 리뷰 인수·잔여 네 finding 재보정 | `02a400e`의 R01/02/03/06 소스 해소만 수용. R04/R05의 날짜 off-by-one, 공유 파일 fixture, test lint, 출력 guard 회귀를 별도 작업 사본의 Sol/medium으로 국소 수정 | [리뷰](reviews/2026-10-14-r2-daily-wire-corrections-review.md) `72784303…`, [실행 기록](reviews/2026-10-13-r2-execution.md); CI 실행 결과·소스 최종 인수는 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; 실제 CI 실패 회수·Rust 최종 보정 통합 및 소스 검토 수용 | `02a400e`는 lint·test compile·Python 실패, chart6/PG 부분 통과. CR-01~04와 추가 compiler 진단을 수정한 두 파일 통합·독립 검토 finding 없음. Python 준비 대기는 test-only 보정 중 | 이 Rust 소스 커밋; [최종 소스 리뷰](reviews/2026-10-14-r2-daily-wire-final-review.md), [CI](reviews/2026-10-14-r2-daily-wire-corrections-ci.md), [Python 진단](reviews/2026-10-14-r2-python-ci-diagnostics.md). 30개 Rust 테스트·새 후보 전체 CI·운영 인수 미완료 |

| 2026-10-14 | R2-04/09 | DOING 유지; Python readiness 소스 보정 검토·새 후보 준비 | test-only 10초 monotonic deadline·child exit 진단과 결정론적 3개 회귀 추가. 기존 thread helper와 SIGTERM 이후 assertions/cleanup의 보존을 AST·diff로 확인 | 이 Python 소스 커밋; [원래 실패 진단](reviews/2026-10-14-r2-python-ci-diagnostics.md), [코디네이터 검토](reviews/2026-10-13-r2-execution.md). pytest/Rust 실행과 새 후보 전체 CI는 후속; 원인/flaky 단정 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; 최종 보정 후보 CI 시작 | `6429d33` push·remote SHA·exact SHA 기존 실행 없음 확인 후 03:59:36 UTC ci.yml 1회 dispatch. 검토된 Rust/Python 보정의 실제 실행 검증 | [CI 38509087712](https://github.com/sihioov/Lagrange/actions/runs/38509087712), [실행 기록](reviews/2026-10-13-r2-execution.md); source 후보와 이 후속 문서 커밋을 구분, 아직 전체 CI 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; 30개 Rust 회귀 PASS·Clippy 한 곳 보정 | `6429d33`의 6개 실행 job 성공, Clippy/required만 실패. 30개 직접 ok·Python suite 485 pass와 개별 node 로그 부재를 구분. test helper 중첩 조건만 의미 보존해 결합 | 이 test 소스 커밋; [CI 근거](reviews/2026-10-14-r2-daily-wire-final-ci.md), [진행 판단](reviews/2026-10-13-r2-execution.md); 새 후보 strict Clippy/required·운영 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; lint 보정 후보의 전체 CI 시작 | `d6bb23a` remote SHA·기존 실행 없음 확인 후 ci.yml 1회 dispatch. 이전 `6429d33` 30개 직접 PASS와 새 후보 실행을 구분 | [CI 38511544759](https://github.com/sihioov/Lagrange/actions/runs/38511544759), [실행 기록](reviews/2026-10-13-r2-execution.md); 새 source의 strict lane/required 결과 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; 30개 회귀 재통과·공용 harness lint 국소 보정 | `d6bb23a`의 6개 실행 job 성공과 Clippy/required 실패 회수. 각 integration binary가 별도 컴파일하는 공용 helper 한 메서드에 이유를 명시한 `allow(dead_code)` 추가; 본문·역할·권한·wire 코드 유지 | 이 harness 소스 커밋; [CI 근거](reviews/2026-10-14-r2-daily-wire-lint-ci.md), [판단과 검증](reviews/2026-10-13-r2-execution.md). 새 후보 strict lane/required와 helper 실제 동작·0060·운영 인수는 후속 |

| 2026-10-14 | R2-04/09 | DOING 유지; 공용 harness lint 보정 후보 검증 시작 | `929fcd6` fast-forward push·remote SHA·exact SHA 기존 실행 없음 확인 후 ci.yml 1회 dispatch. 이전 실패 보고와 모든 역할/동작 미검증 한계 보존 | [CI 38514204062](https://github.com/sihioov/Lagrange/actions/runs/38514204062), [실행 기록](reviews/2026-10-13-r2-execution.md); 소스 후보와 이 후속 문서 커밋을 구분 |

| 2026-10-14 | R2-04/09 | DOING 유지; wire/I0a 전체 CI 부분 인수, 다음 소스 범위 확정 | `929fcd6` run38514204062의7개 실행 job/required·strict Clippy·Rust30개·chart6 성공 확인. I0b 역할/secret 배선과 I1b-S1 원본 manifest 읽기를 분리; 운영/0060 실행 없음 | [CI 보고](reviews/2026-10-14-r2-harness-lint-ci.md), [실행 기록](reviews/2026-10-13-r2-execution.md), [고정 구현 brief](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 대상은 이 기록을 추가한 문서 커밋과 별도 소스 후보929fcd6 |

| 2026-10-14 | R2-04/09 | DOING 유지; 두 소스 패키지 실행 시작 | I0b 역할·secret 배선과 I1b-S1 Raw manifest 원본 읽기를 각각 Luna/max에 위임. 수정 파일 분리, 실제 startup/pending0와 notified 완료 감시 확인 | 명세 커밋 `0a98b9f`; [실행 계획](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md); 결과·리뷰·hosted 검증 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; DB 배선·Raw reader 소스 회수와 독립 리뷰 시작 | 두 worker actual idle/pending0/full final, 11개 파일 범위·정적 검사·9개 참조 보존 확인. 보고 hash 한 값의 형식 오류를 보존하고 실제 bytes 고정 | 이 소스 인계 커밋; [인계·한계](reviews/2026-10-14-r2-role-and-raw-source-handoff.md), [리뷰 계획](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 독립 리뷰·컴파일·SQL·runtime·최종 R2 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; 독립 리뷰 8개 지적 수용·국소 보정 | DB fixture/인벤토리/실패 로그와 Raw 오류 redaction/type/fixture/errno 문제를 직접 대조. 두 보고 actual idle/pending0/full final 확인 후 3개 shell·2개 Rust 파일 보정으로 분해 | 소스 기준f1dfb91; [DB 리뷰](reviews/2026-10-14-r2-daily-role-wiring-review.md), [Raw 리뷰](reviews/2026-10-14-r2-raw-manifest-evidence-review.md), [보정 명세](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 관측된 유출/운영 실패나 테스트 결과를 뜻하지 않음 |

| 2026-10-14 | R2-04/09 | DOING 유지; 두 국소 보정 작업 시작 | 명세03811d0 기준 I0b-F 3개 shell·I1b-S1-F 2개 Rust 파일에 Luna/max notified follow-up. 06:23:29 UTC actual running/pending0 확인 | [실행 기록](reviews/2026-10-13-r2-execution.md), [정확한 보정 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 결과·독립 재검토·새 후보 CI 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; DB·Raw 8개 지적의 보정본 회수·재검토 준비 | actual idle/full finals와 다섯 파일 hash 고정, inventory·로그 gate·redacted 오류·타입·fixture·NOTDIR 수정 대조 및 정적 검사 통과 | 이 소스 커밋; [실행 기록](reviews/2026-10-13-r2-execution.md), [독립 재검토 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 컴파일·테스트·운영 인수 미완료 |

| 2026-10-14 | R2-04/09 | DOING 유지; Raw 보정 독립 리뷰 수용 | 5개 source finding 해소·새 delta finding 없음, actual idle/full final과 고정20개 입력 확인. 19개 정의/Unix18개 선택 예정 회귀는 아직 미실행. DB 리뷰 계속 | [Raw 보정 리뷰](reviews/2026-10-14-r2-raw-manifest-corrections-review.md) `f65bb3e3…`; source `de1889b`. 전체 source·CI·운영 인수 미완료 |

| 2026-10-14 | R2-04/09 | DOING 유지; DB 원지적 해소·신규 parser P2 보정 | DB 리뷰 actual idle/full final과20개 입력 확인. 기존 R01–03 소스 해소, 확장 문법을 literal로 받는 반례를 inert 계산으로 확인해 한 파일 고정 문법 보정 확정 | [DB 보정 리뷰](reviews/2026-10-14-r2-daily-role-corrections-review.md) `52d7400d…`, [후속 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). source/CI/운영 인수 보류 |

| 2026-10-14 | R2-04/09 | DOING 유지; literal parser 보정본 회수·독립 검토 준비 | actual idle/pending0에서 메모리 전용 callback을 실제 전체 결과로 보완. 한 파일 768행/332324fb…와19개 참조 hash·shell bytes 보존·bash-n/AST/diff 확인 | 이 소스 커밋; [보정 인계·후속 검토](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md). parser 회귀·CI·운영 인수 미실행 |

| 2026-10-14 | R2-04/09 | DOING 유지; literal wrapper 개행 지적 수용 | 독립 리뷰 actual idle/full final 및21개 입력·26개 링크 확인. 토큰 문법은 부분 해소, splitlines/기본 newline 변환이 주석을 가상 mapping으로 바꾸는 P2 수용. 동일 파일의 물리 LF·wrapper 회귀 보정 | [literal 리뷰](reviews/2026-10-14-r2-inventory-literal-review.md) `34095bee…`; 기준2593fbe. 로컬 parser/test/새 CI 미실행, 소스·운영 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; 물리 LF 보정본 회수·독립 검토 준비 | I0b-F3 actual idle/full final, 한 파일1023행/0bede130…,19개 참조·review 고정 및 shell/기존 AST 보존 확인 | 이 소스 커밋; [인계와 독립 검토 범위](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 회귀·새 CI·운영 인수 미실행 |

| 2026-10-14 | R2-04/09 | DOING 유지; DB·Raw 소스 리뷰 지적 해소·hosted 검증 준비 | 물리 LF 리뷰 actual idle/pending0/full final 회수,21개 입력·29개 링크와 실제 reader/wrapper 대조. 후속 parser P2 해소·새 finding 없음 수용, 기존 DB3/Raw5 소스 해소 유지 | source1599e09; [독립 리뷰](reviews/2026-10-14-r2-inventory-physical-line-review.md) c86b2422…, [CI 패키지](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 회귀 실행·새 CI·운영 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; 역할·Raw 후보 CI 1회 시작 | 후보f9dfffb fast-forward push·remote SHA·기존 실행 부재 확인 후ci.yml dispatch. Luna/medium collector actual startup07:48:54 UTC/pending0/notified,09:18:54 UTC 보고 경계·bounded heartbeat 설정 | [CI38530582876](https://github.com/sihioov/Lagrange/actions/runs/38530582876), [실행 기록](reviews/2026-10-13-r2-execution.md);30wire+18Raw·shell/PG/Clippy/required 결과 대기, source 후보와 후속 문서 커밋 구분 |

| 2026-10-14 | R2-04/09 | DOING 유지; 역할·Raw CI 실패 회수·연동 보정 | f9dfffb의 E0004 세 match와 bootstrap 기대값7 오류 확인. wire30/Raw18 미실행. 세 파일 보정과 기존 PG 로그·receipt 진단을 분리하고 source 인수 보류 | [CI 보고](reviews/2026-10-14-r2-role-raw-ci.md), [Wave17](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 기존 리뷰·929fcd6 성공 보존, 새 CI·운영 실행 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; 세 파일 보정·공통 compiler 진단 회수 | 실제 idle/pending0/full final, 세 파일 hash·23참조·원본 로그 hash 대조. PG api-tenancy-rls도 동일 E0004; 신규 회귀1개 작성, source 인수는 독립 검토·CI 후 판단 | 이 소스 커밋; [보강 진단](reviews/2026-10-14-r2-role-raw-ci-diagnostics.md), [Wave18](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 기존 실패/부분 성공 보존 |

| 2026-10-14 | R2-04/09 | DOING 유지; caller 보정 독립 리뷰 수용·hosted 검증 준비 | 실제 idle/pending0/full final,32입력·17링크·소스 대조. 세 파일 보정과 신규 회귀에 새 finding 없음; compiler/tests는 아직 미실행 | source8de8cff; [리뷰](reviews/2026-10-14-r2-caller-integration-review.md) eb2bef7c…, [Wave19](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 새 후보49개 Rust·Clippy/shell/PG 검증 필요 |

| 2026-10-14 | R2-04/09 | DOING 유지; caller 보정 후보 CI 1회 시작 | c0ac599 fast-forward push·remote SHA·기존 실행 부재 확인,08:37:46UTC dispatch. Collector actual08:38:54.122UTC/pending0/notified,90분 보고 경계·bounded heartbeat 설정 | [CI38536460035](https://github.com/sihioov/Lagrange/actions/runs/38536460035), [실행 기록](reviews/2026-10-13-r2-execution.md).49개 Rust와 Clippy/shell/PG 실제 결과 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; caller/role/Raw 부분 CI 인수·Wave20 준비 | c0ac599의7개 실행 job/required,49개 Rust와chart6 직접 통과를 실제 idle/최종 회수 후 대조. I0c helper 회귀와 S2-D snapshot 계약을 분리; 첫0060/운영 인수 아님 | [최종 CI](reviews/2026-10-14-r2-caller-role-raw-ci.md), [Wave20](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 소스26개 hash·원본 로그 및 기존 실패 증거 보존 |

| 2026-10-14 | R2-04/09 | DOING 유지; Wave20 두 작업 실제 착수 | I0c 네 파일 회귀 소스와 S2-D 단일 계약 문서; 실제 startup·모델·부모·pending0 확인. 60/45분 보고 기한과 heartbeat c6e1b334를 기록; 새로운 실행/운영 인수 없음 | [Wave20 실행](reviews/2026-10-13-r2-execution.md), [고정 brief](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); brief commit1a2321b |

| 2026-10-14 | R2-04/09 | DOING 유지; I0c 소스 회수·독립 리뷰 준비 | 실제 idle/pending0/full final, 네 파일/27참조 hash와 원본 보존 확인. 직접 로그인 회귀2개 작성, 소스·실행 인수 보류; S2-D는 계속 | [Wave21](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md); rustfmt/bash-n/diff만 통과 |

| 2026-10-14 | R2-04/09 | DOING 유지; I0c 독립 리뷰 실행·모니터 인계 | source f0c6590, reviewer a4b5c1b9 actual09:30:26.183UTC/pending0,30분 보고 기한. S2-D 원래 범위·기한 유지; heartbeat1c5b74e2로 명시적 인계 후c6e1b334 삭제 | [Wave21](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md); 소스 인수·새 CI는 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; helper 리뷰 보정·snapshot 구현 계약 확정 | 두 worker 실제 idle/pending0/full final 회수. I0c 네 receiver만 보정,33pins/42links·정적 보존 확인. S2 제안17pins/67links와 Linux 전용 RawDir 경계 대조 후 P10 채택 | 이 후보 커밋; [helper 리뷰](reviews/2026-10-14-r2-daily-admitter-helper-review.md), [snapshot 계약](reviews/2026-10-14-r2-raw-snapshot-implementation-contract.md), [Wave22/23](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 새 CI·snapshot 소스/운영 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; helper 후보 CI·Linux snapshot 구현 착수 | a888cd6 remote SHA·기존 실행 부재 확인 후 단일 dispatch. Collector09:56:59.924UTC/S2worker09:56:31.201UTC actual running/pending0/notified, 각각90분 보고 경계와 heartbeat1d43b527 설정 | [CI38545976070](https://github.com/sihioov/Lagrange/actions/runs/38545976070), [Wave22/23](superpowers/plans/2026-10-14-r2-daily-input-implementation.md).51개 회귀 결과·snapshot 소스/운영 인수 대기 |

| 2026-10-14 | R2-04/09 | DOING 유지; I0c 부분 인수·snapshot 소스 회수와 등록 | 두 owned turn과 예상 밖 collector 자식의 actual idle/pending0/full final 회수. a888cd6의51Rust/chart6·28pins·strictClippy/required·PG 근거 직접 대조. snapshot 두 파일과 mod 등록 고정,34정의 중 Linux33/non-Linux1 미실행 | [CI 보강·위임 이탈](reviews/2026-10-14-r2-admitter-helper-ci-recovery.md), [소스 인계](reviews/2026-10-14-r2-snapshot-source-handoff.md), [Wave24 독립 리뷰](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 대상은 이 인계 커밋; 새 CI·운영 실행 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; 등록된 snapshot 독립 리뷰 실제 착수 | source4113268·29pins 고정. reviewer a4b5c1b9 Sol/high actual10:46:22.467UTC/pending0/notified,60분 보고 경계와 heartbeat aeaa4fa6 설정 | [Wave24](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md); 새 source/compiler/test/CI 인수 전 |

| 2026-10-14 | R2-04/09 | DOING 유지; snapshot 리뷰 회수·16개 지적 보정 준비 | actual idle/pending0/full final,29pins·81links와 소스 대조.4P1/12P2 소스/회귀 지적 수용; snapshot 인수 보류, 두 파일 고정 첫 보정 | [리뷰496행](reviews/2026-10-14-r2-raw-snapshot-source-review.md)60970b3a…, [Wave25](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 기존a888 부분 성공 보존·새 CI/운영 실행 없음 |

| 2026-10-14 | R2-04/09 | DOING 유지; snapshot 첫 보정 실제 착수 | Luna/max3591c5d4 actual11:44:10.164UTC/pending0/notified, 두 파일/30pins 고정과90분 보고 기한13:14:10.164UTC·heartbeat57665dd0 설정 | [Wave25](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [실행 기록](reviews/2026-10-13-r2-execution.md); source/테스트/새 CI 인수 전 |
| 2026-10-14 | R2-04/09 | DOING 유지; snapshot 보정 회수·독립 재검토 준비 | 실제 idle/pending0/full final,28참조·2원본·94링크 대조. 두 파일 보정과 코디네이터 C01,45개 정의 미실행; 소스 인수 보류 | 이 소스 인계 커밋; [보정 인계](reviews/2026-10-14-r2-snapshot-corrections-handoff.md), [Wave26](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 이전 CI 보존·새 CI/운영 실행 없음 |
| 2026-10-14 | R2-04/09 | DOING 유지; snapshot 보정 독립 리뷰 시작 | source7545094와38개 입력 고정, Sol/high a4b5c1b9 actual running/pending0·notify 확인, 12:38:23.234UTC 시작/13:38:23.234UTC 기한. heartbeat b6352577로 범위 교체 | [Wave26](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [보정 인계](reviews/2026-10-14-r2-snapshot-corrections-handoff.md);45개 정의 미실행, source/CI 인수 보류 |
| 2026-10-14 | R2-04/09 | DOING 유지; 보정 리뷰 회수·두 번째 한정 보정 준비 | actual idle/pending0/full final,38pins·119links 확인.2P1/8P2 수용; E01 경로15개 후속 목록 보정, FD 관측 경계 명시. 반복 검증 실패로 Luna/max→Sol/max 한 단계 모델 상향 | [543행 리뷰](reviews/2026-10-14-r2-raw-snapshot-corrections-review.md)ebe51618…, [Wave27 범위](reviews/2026-10-14-r2-snapshot-second-correction-scope.md);45개 정의 미실행, source/CI 인수 보류 |
| 2026-10-14 | R2-04/09 | DOING 유지; snapshot 두 번째 보정 실제 시작 | Sol/max00e0a229 actual13:47:13.127UTC/pending0/notified,48pins·두 파일 고정.90분 기한15:17:13.127UTC 및 전용 heartbeat1f6d4170 | [Wave27](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [보정 범위](reviews/2026-10-14-r2-snapshot-second-correction-scope.md); 기존45개 정의 미실행·소스/새 CI 인수 전 |
| 2026-10-14 | R2-04/09 | DOING 유지; 두 번째 snapshot 보정 회수·독립 리뷰 준비 | actual idle14:50:24.974UTC/pending0와 전체176행 결과 회수,46pins·2원본·8보존 영역·정확한 두 파일 대조. 기존45개+추가4개, 총49개 정의 미실행. 감시1f6d4170 종료 | 이 소스 인계 커밋; [보정 인계](reviews/2026-10-14-r2-snapshot-second-corrections-handoff.md), [Wave28](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 소스/새 push/CI/운영 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; 두 번째 보정 독립 리뷰 실제 시작 | 소스 bbac2e9, 입력 56개와 정의 49개 고정. Sol/high a4b5c1b9 actual 10-14 15:05:18.207 UTC/pending0/notified, 60분 기한 16:05:18.207 UTC. 감시 6a6fca7d | [Wave28](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [보정 인계](reviews/2026-10-14-r2-snapshot-second-corrections-handoff.md); 소스·컴파일·테스트·새 CI 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; 두 번째 리뷰 회수·C02 포맷 보정 | actual idle15:31:42.846UTC/pending0/full final,475행/56pins/124links/49정의 대조. 기존 소스 지적 종결만 수용. inherited2024 포맷 check1을19hunk 보정 후 check0; 독립 C02-R 준비 | 이 소스 인계 커밋; [Wave28 리뷰](reviews/2026-10-14-r2-raw-snapshot-second-corrections-review.md), [C02 인계](reviews/2026-10-15-r2-snapshot-format-handoff.md). 컴파일/테스트/새 CI/운영 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; C02 포맷 독립 검토 실제 시작 | bcc66d3·24pins·49정의 고정,54개 이전 비변경 입력/2원본 재확인. Sol/high actual15:47:11.426UTC/pending0/notified,20분 기한16:07:11.426UTC, 감시f18a17dc | [Wave29](superpowers/plans/2026-10-14-r2-daily-input-implementation.md), [C02 인계](reviews/2026-10-15-r2-snapshot-format-handoff.md);49개 미실행, 새 push/CI·운영 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; C02 독립 리뷰 종결·snapshot hosted 후보 준비 | actual idle15:56:08.135UTC/pending0/full final,282행/24pins/104links/49정의/19hunk 대조. 구체적 잔여 지적 없음; Linux99개와 chart6개 새 후보 결과 필요 | 이 후보 준비 커밋; [C02 리뷰](reviews/2026-10-15-r2-snapshot-format-review.md), [Wave30](superpowers/plans/2026-10-14-r2-daily-input-implementation.md). 소스 검토 종결만 수용, 실행/운영 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; snapshot 단일 후보 hosted CI 시작·결과 회수 | `245f4d9` 원격 SHA·기존 실행 부재 확인 후 CI를 한 번 dispatch. Luna/medium `6c5cc4eb` 실제 시작 10-14 16:04:01.894 UTC, pending 0·완료 알림 확인; 90분 기한과 감시 `de67d496` 기록 | [CI 38594682124](https://github.com/sihioov/Lagrange/actions/runs/38594682124) attempt 1, [실행 기록](reviews/2026-10-13-r2-execution.md); 51개 고정 입력·Rust 99개/chart 6개 목록 보존, 컴파일·테스트·운영 인수 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; snapshot CI 실패 메타데이터 확인 | 후보 245f4d9/run38594682124 attempt 1의 8개 job 종결 확인. clippy/workspace/PG/required 실패; 수집 담당자는 running/pending 0, 전체 보고·직접 결과 회수 중 | [단일 CI](https://github.com/sihioov/Lagrange/actions/runs/38594682124), [실행 기록](reviews/2026-10-13-r2-execution.md); 원인 추정·소스 수정·재실행 없이 감시와 기존 증거 보존 |
| 2026-10-15 | R2-04/09 | DOING 유지; 실패 전체 회수·C03 컴파일 결함 확정 | actual idle/pending 0/full final 회수 후 감시 종료. 원본 보존, 8개 log/7개 checkout/51pins/59 migration checksum 보강. E0599 두 곳·E0425 한 곳 확인; Rust 99/chart 6 미실행 | [보강 회수](reviews/2026-10-15-r2-snapshot-ci-recovery.md), [CI 38594682124](https://github.com/sihioov/Lagrange/actions/runs/38594682124); 소스 보정·후속 인수 미완료, 재실행 없음 |
| 2026-10-15 | R2-04/09 | DOING 유지; C03 한정 소스 보정 | errno 상수 두 곳과 private C+7 oracle 보정, 3 hunk·50개 불변 pin·49개 S2 정의 확인, scoped rustfmt/whitespace 통과 | [C03 기록](reviews/2026-10-15-r2-snapshot-compile-corrections.md); 새 후보 hosted 검증 전, 이전 실패 보존 |
| 2026-10-15 | R2-04/09 | DOING 유지; C03 단일 후보 hosted CI 시작 | d00b475, 기존 exact-head 실행 0·원격 SHA 확인, 38659690888 attempt 1을 한 번 dispatch | [CI](https://github.com/sihioov/Lagrange/actions/runs/38659690888); 코디네이터 직접 회수, 추가 위임·로컬 실행 검증 없음 |
| 2026-10-15 | R2-04/09 | DOING 유지; C03 직접 테스트 회수·C04 한정 lint 보정 | d00b475/38659690888 attempt1: 99 Rust·6 chart·PG59 및 6개 실행 job PASS, Clippy/required FAIL; 전체 회수 후 2 hunk 보정 | [C03 CI](reviews/2026-10-15-r2-snapshot-c03-ci.md), [C04](reviews/2026-10-15-r2-snapshot-lint-corrections.md); 새 후보 검증 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; C04 단일 후보 CI 시작 | d883c17/38662224666 attempt1, 기존 실행 부재 확인 후 한 번 dispatch; 최초 빈 조회 뒤 재요청 없음 | [CI](https://github.com/sihioov/Lagrange/actions/runs/38662224666); 62 pins, 99 Rust/6 chart/2 exclusions, 직접 회수 중 |
| 2026-10-15 | R2-04/09 | DOING 유지; C04 전체 회수·C05 테스트 lint 보정 | 99 Rust/6 chart/PG59 PASS, Clippy 두 지적·required FAIL. 전체 회수 뒤 두 파일 각 1 hunk 보정, 49개 정의·60개 불변 입력 유지 | [C04 CI](reviews/2026-10-15-r2-snapshot-c04-ci.md), [C05](reviews/2026-10-15-r2-snapshot-test-lint-corrections.md); 새 후보 검증 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; C05 단일 후보 CI 시작 | 7cdf3a6/38664342216 attempt1, 원격 SHA·기존 실행 0 확인 후 한 번 dispatch; 최초 빈 조회 뒤 재요청 없음 | [CI](https://github.com/sihioov/Lagrange/actions/runs/38664342216); 69 pins, 99 Rust/6 chart/2 exclusions, 직접 회수 중 |
| 2026-10-15 | R2-04/09 | DOING 유지; S2 Linux 소스·hosted 검증 부분 인수 | 7cdf3a6/38664342216 attempt1 전체 8개 job SUCCESS, 69pins/7checkout/99 Rust·6 chart/strict Clippy/PG59·cleanup 직접 확인 | [C05 인수](reviews/2026-10-15-r2-snapshot-c05-ci.md); 48 Linux S2 PASS·1 non-Linux 제외, 후속 R2 게이트와 이전 실패 보존 |
| 2026-10-15 | R2-04/09 | DOING 유지; I0d 실제 실패 경로 검증 작성 | 운영 bootstrap 불변, 83개 역할·권한·8개 secret·잠금 timeout rollback 사례와 named evidence 연결 | [소스 검토](reviews/2026-10-15-r2-bootstrap-negative-source.md); 로컬 문법·정적 검사만 통과, hosted 실행 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; I0d 단일 후보 CI 시작 | dd1b92f/38670368360 attempt1, 76pins·83새사례·99Rust·6chart 고정 | [CI](https://github.com/sihioov/Lagrange/actions/runs/38670368360); 직접 회수 중, 추가 위임·로컬 실행 없음 |
| 2026-10-15 | R2-04/09 | DOING 유지; I0d 83개 PASS·전체 실패 회수, 테스트 fixture 보정 | dd1b92f/38670368360 attempt1 전체 회수, Rust99/chart6/PG59 통과. 기존 WebSocket Closed 실패를 제한된 handshake 보정으로 대응 | [실패 회수](reviews/2026-10-15-r2-bootstrap-negative-ci.md), [I0d-C01](reviews/2026-10-15-r2-bootstrap-ci-fixture-correction.md); 새 후보 검증 전 |
| 2026-10-15 | R2-04/09 | DOING 유지; I0d-C01 단일 후보 CI 시작 | 57ca4d1/38672658570 attempt1, 85pins/60blobs·영향받은 transport3·bootstrap83·Rust99·chart6 고정 | [CI](https://github.com/sihioov/Lagrange/actions/runs/38672658570); 이전 실패 보존, 직접 회수 중 |
| 2026-10-15 | R2-04/09 | DOING 유지; I0d bootstrap/rollback 및 C01 한정 인수 | 57ca4d1/38672658570 attempt1 전체 8개 job SUCCESS,85pins/7checkout/83bootstrap/3transport/99Rust/6chart/strictlint/PG59·cleanup 대조 | [인수 기록](reviews/2026-10-15-r2-bootstrap-negative-acceptance.md); 0060과 이후 R2 게이트는 미완료 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2 구현 계약 고정 | 0060의 SQL transport/receipt/오래된 scan 차단/policy projection/backoff/direct-login 6개 접점과 32개 실제 역할 검증 요구를 확정. 다음은 완전한 SQL·테스트·harness 후보 구현 | [I2 계약](reviews/2026-10-15-r2-daily-sql-implementation-contract.md), [Wave36](superpowers/plans/2026-10-14-r2-daily-input-implementation.md); 이 문서를 추가한 커밋. 문서 정합성 확인만 수행, SQL·컴파일·테스트·신규 CI·운영 인수 없음. 기존 57ca4d1 부분 성공과 모든 실패·미검증 기록 보존 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2 완전한 소스 후보 작성 | 0060 UP/DOWN·32개 DB 정의·73개 고정 하위 사례·순차 harness, 이전118 migration과 기존 테스트 본문 보존 | [소스 판정](reviews/2026-10-15-r2-daily-sql-source.md); 이 기록의 커밋. 로컬 포맷·셸 문법·정적 대조만 통과, SQL·컴파일·테스트와 hosted 인수 대기 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2 첫 단일 후보 CI 시작 | f5122dd/38689151958 attempt1; 224pins/195blobs/32definitions/73subcases, 기존 실행 부재 확인 후 한 번 dispatch | [회수 기록](reviews/2026-10-15-r2-daily-sql-ci.md); 실행 증거 대기, 기존 성공·실패 보존 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2 실패 전체 회수와 C01 한정 보정 | 첫 후보8jobs/84steps/224pins/195blobs/7checkout 회수. SQL IF/CASE grouping과 테스트 helper 3곳 보정 | [CI 실패](reviews/2026-10-15-r2-daily-sql-ci.md), [C01](reviews/2026-10-15-r2-daily-sql-c01.md); 새 후보 검증 전 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2-C01 단일 후보 CI 시작 | 6fbc976/38690484091 attempt1,230pins/197blobs, exact-head query0 후 단일 dispatch | [회수 기록](reviews/2026-10-15-r2-daily-sql-c01-ci.md); 이전 실패·부분 성공 보존 |

| 2026-10-15 | R2-04/09 | DOING 유지; C01 실패 회수와 C02 한정 보정 | 8jobs/84steps/230pins/197blobs/7checkout, DB16/16와 기존28개 실패 분리. SQL 식별자·일반 job guard·UUID·app fixture 보정 | [C01 결과](reviews/2026-10-15-r2-daily-sql-c01-ci.md), [C02](reviews/2026-10-15-r2-daily-sql-c02.md); 인수 대기 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2-C02 단일 후보 CI 시작 | a252cfb/38694431037 attempt1,241pins/202blobs, exact-head query0·원격 검증 후 단일 dispatch | [회수 기록](reviews/2026-10-15-r2-daily-sql-c02-ci.md); 과거 실패 보존 |

| 2026-10-15 | R2-04/09 | DOING 유지; C02 회귀 해결·C03 수집 규칙 보정 | 8jobs/84steps/241pins/202blobs/7checkout 회수. DB32/73·회귀28 통과, 숫자 포함 결과명 누락으로 후속 DB11 미실행 | [C02 결과](reviews/2026-10-15-r2-daily-sql-c02-ci.md), [C03](reviews/2026-10-15-r2-daily-sql-c03.md); 전체 검증 대기 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2-C03 단일 후보 CI 시작 | dee78c4/38696988981 attempt1,249pins/204blobs, query0·원격 검증 후 단일 dispatch | [회수 기록](reviews/2026-10-15-r2-daily-sql-c03-ci.md); 모든 기존 결과 보존 |

| 2026-10-15 | R2-04/09 | DOING 유지; I2 구현·hosted 검증 완료 | dee78c4/38696988981 attempt1 전체 성공, 249pins/204blobs/7checkout·8logs 회수. DB32/73·DB12 exit0/no-SKIP·회귀28/99/chart6/transport3·bootstrap83·PG60×2/rollback/cleanup 확인 | [I2 인수](reviews/2026-10-15-r2-daily-sql-acceptance.md); 다음 I1b 복원·admission, I3–I7·운영·Owner·3거래일은 별도 미완료 |

| 2026-10-15 | R2-04/09 | DOING 유지; I1b-S3 범위 고정·구현 시작 | kis/kis-calendar 원본 경로, 달력 독립 projection·연속 날짜 증명. I2 인수 유지 | [범위 계약](reviews/2026-10-15-r2-daily-evidence-lineage-contract.md); 로컬 실행·신규 위임 없음, 소스·hosted 인수 전 |

| 2026-10-15 | R2-04/09 | DOING 유지; I1b-S3 소스·22회귀 작성 | 원본 추적·달력 projection·연속 날짜 증명, 기존37 정규화 사례 목록 고정.247 이전 pin 보존 | [소스 판정](reviews/2026-10-15-r2-daily-evidence-lineage-source.md); 포맷·정적 대조 통과, 컴파일·테스트는 hosted 대기 |

| 2026-10-15 | R2-04/09 | DOING 유지; I1b-S3 단일 후보 CI 시작 | eb0e839/38711965495 attempt1,262pins/215blobs, exact-head query0·원격 확인 후 한 번 dispatch | [회수 기록](reviews/2026-10-15-r2-daily-evidence-lineage-ci.md); 이전 결과 보존, 검증 대기 |

| 2026-10-15 | R2-04/09 | DOING 유지; I1b-S3 구현·hosted 검증 완료 | eb0e839/38711965495 attempt1 전체 성공.262pins/215blobs/7checkout·8logs, 새22·정규화37·기존99·DB32/73·legacy28·chart6·transport3·bootstrap83·PG60×2/cleanup 대조 | [S3 인수](reviews/2026-10-15-r2-daily-evidence-lineage-acceptance.md); 다음 가격·기업행사 복원/frontier/admission, 전체 I1b·운영·Owner·3거래일은 미완료 |

| 2026-10-15 | R2 전체 | DOING; Owner 지시로 Ralph 목표 활성화 | R2-01~09와 최종 C_R2 완료까지 연속 진행. 작은 완료마다 사용자 재지시를 기다리지 않으며 기존 권한·실행 제한과 실제 Owner/시간 경과 조건 유지 | 이 기록의 커밋. 시작점52cba24; 다음 I1b-S4 가격·기준정보·기업행사 오프라인 복원 |

| 2026-10-15 | R2-04/09 I1b-S4 | DOING; CI 전체 회수 후 C01 보정 | 48beb25/run38722595047 새32개·회귀·PG 통과, strict Clippy fixture 타입 한 건 실패 보존. 이름 있는 구조체로만 보정하며 새 후보 검증 대기 | [실패 회수](reviews/2026-10-15-r2-daily-provider-reconstruction-ci.md), [C01](reviews/2026-10-15-r2-daily-provider-reconstruction-c01.md). 운영/Owner/3거래일 미시작 |

| 2026-10-15 | R2-04/09 I1b-S4 | bounded DONE; 전체 R2 DOING | 302f29a/CI38725299504 attempt1 전체 성공·개별 회귀/PG/checkout/pin 대조. C01 fixture lint 해결, 이전 실패 보존 | [S4 인수](reviews/2026-10-15-r2-daily-provider-reconstruction-acceptance.md). 다음 S5a 소스 탐색/관측·통합 검증. 운영/Owner/3거래일 미시작 |

| 2026-10-15 | R2-04/09 I1b-S5a | DOING; source written | 전체 snapshot 탐색·D4 session/document·canonical 관측/변경 감지와33개 회귀 작성,274 prior pins 보존. Source/CI 인수 대기 | [소스 검토](reviews/2026-10-15-r2-daily-source-discovery-source.md). Protected prefix/admission·운영/Owner는 후속 |

- 2026-10-15: Wave44 S5a5a83e0d/CI38728478625 단일 실행 시작.33개 회귀 및 전체 필수 결과 회수 대기; protected-prefix 준비는 별도 미등록 파일로 제한.

- 2026-10-15: Wave44 전체 회수 후 S5a-C01 fixture2개 보정과 Wave45 보호된 최초 연장/후보21회귀를 묶음. [실패 근거](reviews/2026-10-15-r2-daily-source-discovery-ci.md), [소스 범위](reviews/2026-10-15-r2-daily-protected-prefix-source.md). R2-04/09 DOING.

| 2026-10-15 | R2-04/09 | DOING 유지; 보호된 최초 가격 연장 단일 후보 CI 시작 | 5f5f4e8/38732527740 attempt1,303pins/243blobs·21새정의/S5a33 고정, exact-head query0·원격 확인 후 한 번 dispatch | [CI](https://github.com/sihioov/Lagrange/actions/runs/38732527740); 전체 회수 전 소스 불변, 직접 회수·후속 독립 파일 준비 |

| 2026-10-15 | R2-04/09 | DOING 유지; 최초 가격 연장·탐색 한정 인수 | 5f5f4e8/38732527740 전체 성공,303pins/243blobs/7checkout,21+33새 범위와 모든 기존 named/PG gates 확인 | [인수](reviews/2026-10-15-r2-daily-protected-prefix-acceptance.md); 실제 v3 전체 경로 통합·후속 저장/승인·운영은 별도 미완료 |

- 2026-10-15 — R2-04/09 Wave46 S5c: descriptor 기반 후보/승인본 저장과 독립 첫 admission 검증·canonical readback을 작성. 새27개 정의는 미실행(26 Linux/1 제외), direct coordinator 정적 검토이며 독립 리뷰·역할분리 end-to-end 인수 아님. [범위/한계](reviews/2026-10-15-r2-daily-artifact-admission-source.md). 기존 성공/실패 보존, exact candidate hosted 검증 후 다음 의존 작업을 계속한다.

- 2026-10-15 — Wave46 후보718d99d/CI38737242162 attempt1 한 번 실행. 기존 all-event exact-head 조회0·원격SHA 일치,313pins/250blobs 고정. 실행 결과 회수 전 인수 없음.

- 2026-10-15 — R2-04/09 Wave46 실패 회수 후 Wave47 S5c-C01 fixture 위치 보정. 운영33,088bytes 동일·기존 이름 유지, 인수는 새 정확한 후보 CI 이후 판단. [보정 범위](reviews/2026-10-15-r2-daily-artifact-admission-c01.md).


### Wave47 C01 single candidate launch — 2026-10-15

Candidate2824dd0c2503ee37ab828bafdf0ddb0750974951, CI38740542364 attempt1,
created13:11:01UTC. Exact-head all-event query0/remotematch preceded one dispatch.
319pins/252blobs/26Linux expected plus1new exclusion are frozen. Full result recovery
pending; no acceptance. Prior failed candidate remains preserved. Only unregistered
successor files and mutable tracking docs change while the run is active.


### 2026-10-15 — Wave48 C02 protected hosted fixture

Wave47 2824dd0/run38740542364 attempt1 is fully recovered FAILURE:8jobs/8logs/
7checkouts,319pins/252blobs. Same16Root::open UnsafePath setup failures,10newpasses;
all prior named groups,Clippy/format/web/Python/policy/PG32/73/83/12/60checksums/cleanup
pass. HOME alone did not meet the fixture precondition; precise ancestor cause unknown.
C02 uses a new run/attempt-specific root0755 ancestor and runner0700 child in isolated
hostedCI, explicit test env, nonrecursive cleanup.33088productionbytes unchanged;
no guards weakened or local privileged/project execution. Scope is onlyartifact
privatefixture and CIworkspace harness.27names retained. Unregistered S5d/e drafts
remain excluded; no source/runtime/Owner/final C_R2 acceptance. See C02 review record.


### Wave48 C02 single candidate launch — 2026-10-15

Candidate69513a8e596ebecdac9736f276b9b2303dba8dc5, CI38744953343 attempt1,
CI/ci.yml/workflow_dispatch/excited-dolphin,created13:42:11UTC. All-event exact-head
queries before/after push returned0 and remote SHA matched; one dispatch only.
325pins/254candidateblobs,27retained names/26Linuxexpected/1newexclusion are frozen.
Full recovery must include explicit protected-fixture setup and nonrecursive cleanup
steps and four mode/UID/GID observations. No source/compiler/runtime acceptance.
Prior two failures remain preserved. Only unregistered successor preparation and
mutable coordinator tracking docs may change while CI runs. Ralph continues.

- 2026-10-15 — R2-04/09 Wave49: full Wave48 failure recovery 뒤 daily parent/frontier/supplier와 격리 CI fixture/Paper clock 보정을 통합. 새37정의 미실행, 기존 증거 보존. [소스 범위·한계](reviews/2026-10-15-r2-daily-continuation-frontier-contract.md).

- 2026-10-15: R2-04/09 Wave49 `edeaa5d`의 CI38750900179 attempt1 단일 발행. 348pins/261blobs 고정, 결과 대기. 이전 실패는 보존하며 I3 계산 경로 준비를 이어간다.

- 2026-10-15 — R2-04/09 Wave50: Wave49 E0603 실패/전체 미실행 근거 보존, fixture 가시성 한정 보정과 I3 typed 계산 경로·20회귀 작성. [소스 범위와 한계](reviews/2026-10-15-r2-daily-factor-target-source.md). 실제 전체 승인 흐름과 운영 인수는 후속.


### Wave50 I3 single candidate launch — 2026-10-15

Candidate `1c82592321609dc2a46e0f84f63a01ad89a70f42`,
[CI38755069474 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38755069474),
created14:50:59UTC, CI/ci.yml/workflow_dispatch/excited-dolphin. All-event exact-head
queries before/after push returned0; remoteSHA matched before exactly one dispatch.
399pins/296committed blobs are frozen. Required evidence includes20new I3 definitions,
29historical computation rows,37S5def and all carried groups. DedicatedPG must prove
13outer exits0/noSKIP and all8owner-beta-publish named rows; their workspace result
alone is not actualDB evidence. Full result recovery pending; no source/compiler/test
acceptance. Only unregistered I4 preparation and mutable tracking docs may change.
I4–I7/integrated independent review/runtime/Owner/3tradingdays/finalC_R2 remain open.

- 2026-10-16 — R2-04/09 Wave50 전체 실패 회수(8jobs/8logs/7checkout/399pins/296blobs). 네 회귀 실패를 보존하고 Wave51 테스트 한정 보정 착수. [근거/범위](reviews/2026-10-16-r2-daily-factor-target-ci.md). 실제DB 발행8개 포함 PG 통과는 전체 R2 인수가 아니다.


### Wave51 single correction candidate launch — 2026-10-16 KST

Candidate `bd33596f515afe18437dad03bd940219c630d248`,
[CI38760423300 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38760423300),
created2026-10-15T15:27:03Z. CI/ci.yml/workflow_dispatch/excited-dolphin.
All-event exact-head queries returned0 before/after push, remote SHA matched;
exactly one dispatch.423pins/302committedblobs,21I3 definitions (one renamed and
one added),38service-status definitions and all carried groups are frozen.
Four test files/regions only; production bytes/SQL unchanged. Source disposition
is direct coordinator only. Full hosted recovery pending; no acceptance yet.
Unregistered I4 source preparation continues independently. Earlier failures,
operational/Owner/three-trading-day/finalC_R2 gates remain.

- 2026-10-16 — R2-04/09 Wave51 `bd33596` 전체 성공을 423pins/302blobs/7checkout·개별 결과와 대조해 I3/S5cdef 범위별 인수. Wave52 I4a cycle/예약, 14개 정의와 실제 PG lane 준비. [근거](reviews/2026-10-16-r2-test-correction-ci-acceptance.md). Ralph 지속.

- 2026-10-16: Wave52 I4a `d0536f6` 단일 CI38764564342 attempt1 시작. 기존 all-event 실행0/remoteSHA 대조 후 dispatch1. 447pins/310candidateblobs·14개 검사를 고정, 전체 결과 회수 전 인수 없음. Ralph 지속.

- 2026-10-16: Wave52 `d0536f6` CI38764564342 전체 실패 회수(8jobs/8logs/7checkout/447pins/310blobs). 세 테스트 logging 참조 보정 및 Wave53 I4b 일일 실패·취소·복구와 실제DB12개 검사 준비. 이전 성공·실패 보존, 전체 R2와 운영 인수는 미완료.

- 2026-10-16: Wave53 a72aaa4 전체 실패 회수(웹 E2E 원인 UNKNOWN), Rust/Clippy/PG16단계와 개별 이름 대조 PASS. Wave54 일일 성공 경계 및 안전한 웹 진단3개 파일 변경. 기존 실패·관측 한계를 보존하며 전체 I4/R2는 미완료.

- 2026-10-16 — R2-04/09 Wave54 전체 성공 회수: 8jobs/8logs/7checkout/488pins/318blobs, 새 Rust8·웹 감독34·E2E121+14 및 모든 기존 PG/Rust gates 대조. [범위별 인수](reviews/2026-10-16-r2-daily-success-ci.md). Wave55 세 파일 체인 로더 착수; 전체 R2는 DOING.

- 2026-10-16 — R2-04/09 Wave55 `3073e9e`의 CI38776392579 attempt1 한 번 발행. 507pins/321blobs·원격 SHA·all-event 실행 부재 대조 후 결과 회수 중. 기존 부분 인수와 최종 R2의 구분 유지.


### Wave55 인수와 Wave56 일일 실행 연결 — 2026-10-16 KST

R2-04/09:3073e9e/CI38776392579 attempt1 전체 SUCCESS 회수.507pins/321blobs,
8jobs/logs/7checkout,새15 및 모든 필수 회귀·PG16단계·Web34+121/14 통과.
기존 실패와 한계를 보존한다. Wave56은 runner/publish와 새 일일 실행 모듈·
실제 역할 테스트·PG harness 다섯 파일만 변경한다.527pins/12새 정의를 고정;
새 CI 전 실행 수용은 없다. 전체 I4/I5-I7/운영/Owner/3거래일/finalC_R2 미완료.


### Wave56 single candidate CI — 2026-10-16 KST

Candidate93437069c7a5bfdf6a1f8d547eb1adb87530f472, CI38780494397 attempt1,
created2026-10-15T17:49:14Z, workflow_dispatch/ci.yml/excited-dolphin. Prepush
all-event exact-head query0 at17:48:47.139843Z, postpush query0 at17:49:13.132163Z,
remote SHA matched; dispatch count1 and one matching run recovered.527pins/325blobs,
new2pure+10actualDB definitions, five historical runner names and all prior groups
frozen. Dedicated PG lane count17 with exact10dailyRunner rows required. No new
source/compiler/test acceptance; only direct review/scoped rustfmt locally.
No rerun/cancel/duplicate. Source stays frozen until full result recovery; later
tracking-doc HEAD does not change candidate. Full I4/I5-I7/operations/Owner/3days/
final C_R2 remain open. Ralph continues without further delegation.

| 2026-10-16 | R2-04/09 | DOING; Wave56 실패 회수·Wave57 테스트 보정 | 8jobs/logs·7checkout·527pins/325blobs, 새2pure+10실제DB·PG17 통과. strict lint의 unused fixture methods 및 collector DB bootstrap1개 실패. 후자는 SQLSTATE 소실로 원인 UNKNOWN. 두 테스트 파일만 보정, 표준 역할/target grants·안전한 진단·불변 replay 검사. | [회수](reviews/2026-10-16-r2-daily-runner-ci.md), [보정](reviews/2026-10-16-r2-daily-runner-lint-correction.md); 후속 후보 미실행 |

| 2026-10-16 | R2-04/09 | DOING; Wave57 인수·Wave58 취소 기능 | c4f35bb 전체 성공·548pins/328blobs/7checkout·2pure+10DB/collector3/PG17 및 이전 gates 대조. 0060 보존·0061 추가와14새DB회귀 착수 | [인수](reviews/2026-10-16-r2-daily-runner-correction-ci.md); 운영·Owner·최종 R2 별도 |

- 2026-10-16 — R2-04/09 Wave58: [superseded 취소 소스](reviews/2026-10-16-r2-daily-supersession-source.md)6파일·14개 DB 회귀 작성,567pins로 고정.0060/기존 회귀 보존. 소스·컴파일 인수는 새 후보 CI 이후 판단.
