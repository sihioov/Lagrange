# ADR-0010: Owner-beta Paper v1 운용 규칙과 증거 경계

- Status: Accepted — 2026-10-29 (Asia/Seoul)
- Decider: Product owner (O1~O3), coordinator (아래 기술 계약·구현 순서)
- Roadmap: R5-01 / G1
- Owner 응답 원문: **“권장 묶음 채택”**
- 기준 소스: `a30edacff987d55b5be7cca25bed7faeec589629`; 이 결정 문서를 추가하는 변경이 계약 채택 대상이다.

## 채택 원문과 범위

[Paper 계약 초안](../contracts/owner-beta-paper-v1.md)의 상태 표기와 과거 OPEN은 원문 그대로 보존한다.
원문 bytes SHA-256은 `18cde9fa7f15e19c9ec272abb780d06ae4be672068ed3f9208c39dc2297ab8d0`다.
계약 식별자는 `owner-beta-paper-v1-draft-1`이며 이 결정과 함께 채택된 v1 운용 계약으로 참조한다.
[WP-1 보완 보고](../reviews/2026-10-29-r5-contract-gap.md)의 SHA-256은
`a4d788a95cec53db1a0198f9923c789e53f02bce85e592f1d63fdf7d27793896`다.
새 사실이나 의미 변경은 원문 수정 대신 후속 버전/결정으로 기록한다.

Owner에게 제시한 전략·자금·예시 비용, EOD 지연 모의 정산·한 계정/전략 자동 적용,
중지/복원 후 보존·새 관찰·명시 재개 묶음을 채택했다. R4와 같은 비용/수량/반올림 대안은 선택하지 않았다.
같은 값과 기존 개인 데이터 권리를 다시 승인받지 않는다.

## O1 — 첫 금융 모형

- `trend_following/1.0.0`: `069500.KRX`, fast50/slow200, 기존 승인 target 계산 재사용.
  초기자금 `10000000.0000` KRW, 정수주/lot1, 최초 진입과 신호 전환 때 재조정한다.
- 기존 Paper sizer·checked fixed-point/반올림을 사용한다. 기존 프로필/R4 계산 의미를 바꾸지 않는다.
- `KRX_ETF_DEFAULT@1`을 **예시 모형**으로 채택한다. 매수/매도 commission 각각0.00015,
  최소 commission1000원/체결, 양쪽 slippage10bp, 최소 거래100000원, 재조정 기준0.005, 매도세0이다.
  실제 증권사 비용이나 세율이라고 표시하지 않는다. slippage를 현금에서 이중 차감하지 않는다.
- 계약 §4의 매도→매수·정수 수량/현금·비용·배당 제외·같은 모형 benchmark 규칙을 적용한다.
  R4와 다른 비용·수량·반올림/기간의 성과는 차이 또는 NOT_COMPARABLE로 설명한다.

## O2 — 시간과 자동 적용

추천 T의 다음 검증된 거래일 E에 대해 EOD 입수/admission 뒤 **E의 raw open**으로 지연 모의 정산한다.
실제 처리·자료 가용 관측 시각과 경제적 세션 날짜를 분리하며 개장 시각에 실행했다고 표시하지 않는다.
T 추천/적용과 E 정산/종가 평가는 각각 해당 KST 날짜의 종료 전 완료하는 경우만 무인 성공이다.
가격/승인 누락 시 대기·차단하고 다른 날짜/공급자 가격으로 대체하지 않는다.

자동 적용은 명시적으로 결합된 Owner 1명의 한 Paper 계정/전략에 한정한다. 이 결정은 운용 방식의
채택이며 아직 없는 account UUID·동의 event·활성화 epoch를 만들지 않는다. 실제 계정 바인딩과 동의는
계약의 인증·보호 설정 경로에 기록하고, 같은 일반 승인을 반복 질문하는 절차로 만들지 않는다.

## O3 — 중지·복원·연속성

원장과 미정산 target/outbox를 보존하고 OFF/복원 후 자동 재개하지 않는다. 새 관찰과 현재 입력·권리·
동의·epoch 확인을 거쳐 명시 재개한다. cohort/의미가 달라지는 release/config·전략·비용·정책 변경은
기존 streak를 새 운용으로 이월하지 않는다. 원래 identity의 재시도/복구로 성공 일수를 늘리지 않는다.
활성화 전 EOD/추천3거래일과 활성화 후 Paper3거래일은 별도 인수다.

## G1 기술 결정과 다음 구현

다음은 Owner 금융값을 추가로 만드는 결정이 아니라 위 계약을 구현하는 coordinator 결정이다.

1. 역사 5-pin/Daily admission/generic DatasetPin을 구분하고 sealed Daily adapter와 별도 beta FK/계보를 사용한다.
   개인 Owner 권리/`paper_view`는 기존 ADR-0005와 구성의 범위에서 현재 유효성을 재검증한다.
2. D5는 계약 §7.2의 **B: root 보호 scheduler가 미래 slot을 직접 arm/dispatch**하는 방식을 사용한다.
   root/kernel/Docker를 신뢰 기반으로 명시하며 timer 활성·InvocationID·임의 bool/JSON만으로 origin을 증명하지 않는다.
   최소 arm 여유60초, due 이후 launch 허용60초, 관측 clock/suspend 차이1초 초과 거절을 v1 기술 한도로 고정한다.
3. 보호 원본 event chain·실제 child/container·Raw original·DB admission/run을 독립 대조한다.
   worker/app/admitter의 DB 행만으로 root origin 권한을 만들지 않는다. 수동/catch-up/UNKNOWN은 성공에서 제외한다.
4. 기존 `daily_input_admitter`/`worker` role과 보완 보고 §3.1의 narrow SQL 네 함수를 사용한다.
   원본 read-only·role별 helper·fixed search_path·FORCE RLS와 직접 쓰기 금지를 적용하며 새 DB role/credential은 만들지 않는다.
5. 동일 source/policy/Owner/날짜의 일일 dispatch 유일성과 정확한 원본 재사용/terminal 기록을 유지한다.
   필드 한도(contract/source64KiB, session256KiB, eligibility1MiB), closed schema, 현재 검사 TTL180초,
   날짜별 pin과 공통 cohort의 분리를 계약대로 적용한다. Python과 Rust는 같은 wire schema를 사용한다.
6. 현재 migration 최대0065를 확인했다. evidence의 신규0066 up/down은 WP-2A2, Paper 원장·control의
   신규0067 up/down은 후속 WP-3에 예약한다. 구 migration/원장 삭제로 rollback을 통과시키지 않는다.
7. WP-2A1의 검증된 collector 필드는 보존한다. 보호 producer/wrapper/DB reader를 **WP-2A2**에서 먼저
   연결한 뒤 기존 WP-2의 입력·연속3세션 checker를 **WP-2B**로 실행한다. 공유 파일은 순차 인계한다.
   구현자가 같은 wire의 상세 표현을 코드·schema·상호 검증으로 제시하면 G2a에서 ABI를 인수하며,
   위 의미·role·예산·시간·권리 경계를 임의 변경하지 않는다.

이 결정으로 **R5-01 계약 정리/G1은 완료**다. 필요한 실제 timerfd/FD/Docker/PG 동작은 해당 구현의
검증 의무로 남으며 결정 문서로 PASS 처리하지 않는다. 설치·권리/입력 실제 readback·pre/post 실제3거래일·
Paper 원장/복원·Owner 화면 인수는 R5-02~05의 별도 조건이다.
운영 installer로 실행 가능한 소스를 준비하되 이 기록은 운영 활성화·새 provider/계좌/주문 API 권한이 아니다.
