# Owner-beta Paper v1 — 입력·금융·운영 계약 초안

작성: 2026-10-29 (Asia/Seoul). 소스 기준: `a30edacff987d55b5be7cca25bed7faeec589629`.
로드맵: **R5-01 / WP-1**. 계약 식별자 제안: `owner-beta-paper-v1-draft-1`.
상태: **PROPOSED / DECISION_PENDING / G1 OPEN**. 이 문서는 운영 설정이나 활성화 증명이 아니다.
후속 채택은 별도 결정 기록으로 원문·해시를 고정한다. 이 문서 작성만으로 미결값을 승인값으로 바꾸지 않는다.

## 1. 범위와 상태 표기

Owner 1명, 고정 ETF11, 승인된 KIS 읽기 전용 입력, 내부 모의 원장만 대상으로 한다.
`PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false`를 입력·API·성과에 유지한다.
배당 포함 총수익, 당시 실제 주문 가능성, 장 시작 체결 완료, 엄격 PIT를 주장하지 않는다.
KIS 계좌·주문·잔고·체결 API, live profile, 새 공급자/endpoint/권리, KIND 자동화는 제외한다.

표의 **기존 결정**은 채택 기록, **구현 사실**은 기준 SHA의 소스, **권장**은 이번 제안이다.
`OPEN`은 구현 불가능 선언이 아니라 필요한 결정 또는 증거가 아직 없다는 뜻이다.
기존 승인을 재요청하지 않는다. R2/R3 FINAL과 D-06/D-07/005930 조사는 독립적으로 유지한다.
실제 R5 소비 입력·무인 증거가 부족하면 운영 전환을 차단하되 독립 소스 개발은 계속할 수 있다.

근거: [R5 계획의 경계](../superpowers/plans/2026-10-29-r5-paper-implementation.md#goal-and-boundaries),
[R4 최종 인수](../reviews/2026-10-28-r4-g3-readonly-preflight.md#2026-10-29-owner-use-acceptance--r4r4-06-done).
파일별 gap·변경 후보·검증 기록은 [WP-1 보고서](../reviews/2026-10-29-r5-contract-gap.md)에 둔다.

## 2. D1~D7 결론과 결정 주체

| ID | 이미 채택된 결정 | 실제 구현 사실 | 권장안 | 대안과 영향 | 남은 결정/상태 |
|---|---|---|---|---|---|
| D1 입력·권리 | 개인 Owner의 기존 KIS 권리는 ADR-0005로 해결됨. 2차 Paper·ETF11·가격수익률 경계도 채택됨 | 역사 5-pin, Daily 4해시+kind/epoch, generic DatasetPin은 다른 타입·저장소. 권리 구성에 `paper_view` 존재 | sealed Daily를 직접 읽는 별도 Paper admission. 원장 재사용, generic 등록/READY 위조 없음. 정확한 기존 권리 reference/hash를 실행마다 재검증 | 실제 generic dataset 등록·publication bridge는 별도 큰 범위이며 데이터 의미/권리 자동 승격 불가 | coordinator가 adapter/DB 경계를 채택. 운영 권리 행의 현재 유효성은 실행 전 확인. **권리 재승인 불필요; 기술 OPEN** |
| D2 금융 | R4 v1/v2 계산 규칙은 R4용으로 채택됨. 기존 Paper preview는 동일 Paper sizer 재사용 설계가 승인됨 | Paper의 `KRX_ETF_DEFAULT@1`은 확인 필요 placeholder. R4 비용·수량 규칙과 다름 | 첫 전략 trend 50/200·1천만원 KRW·정수주·기존 Paper 비용/sizer를 명시적 모형으로 채택, 신호 전환 때만 재조정 (§4) | R4 10bp/0비용과 동일 계산은 새 versioned cost/sizer 경계와 추가 범위 필요; 현재 결과와 숫자 일치를 약속 못함 | **Owner 금융 묶음 O1 OPEN** |
| D3 시간·가격 | 추천일 raw close preview, 후속 raw open 재산정, 없는 가격 대체 금지 | Daily EOD는 16:30 이후. 기존 generic 원장 시각은 합성 00:30 UTC; Daily bar는 날짜형 | E의 EOD 입수·admission 뒤 E raw open으로 지연 정산. 실제 처리·가용 시각은 별도 저장. 같은 KST 날짜 내 완료를 soak 기준으로 제안 | 실제 장 시작 처리에는 별도 승인된 가격 입력/가용성 계약 필요. 현재 범위에서 약속 불가 | **Owner 지연 처리·마감 O2 OPEN**, 기술 시각 필드 OPEN |
| D4 적용·동의 | 수동 preview/apply 분리; generic auto-apply는 binding opt-in 필요 | Owner-beta 추천은 generic 추천 테이블과 분리되어 자동 bridge 없음 | 명시 Owner opt-in 1계정/1전략, 날짜별 immutable 적용 identity. OFF/철회 즉시 신규 적용 금지 (§6) | 수동만 운영하면 기능 검증 가능하나 R5-05 무인 인수 불가 | **Owner 자동 적용 O2 OPEN**, coordinator 충돌/취소 경계 채택 |
| D5 증거 | 활성화 전 3개 실제 연속 거래일, 수동/catch-up 제외, 실패는 연속성 단절 | Daily DB poll/reservation/publication 기록은 있으나 예약 수집부터 끝까지의 무인 증명은 없음 | **B: 미래 slot을 직접 arm/dispatch하는 root scheduler**와 DB·sealed 파일 독립 검증 (§7.2) | A의 timer causal origin은 확인한 로컬 API로 입증 불가. boolean/자기신고 JSON 거절 | **coordinator B 계약·WP-2A/2B 범위 채택 OPEN**; 구현·실제 pre-soak 미확인 |
| D6 연속성 | 날짜별 Daily 입력은 달라질 수 있음. Owner/전략 혼합 금지 | config/activation epoch와 parent chain은 존재; 실제 프로세스 image 결합은 부족 | 날짜별 data pin을 연결하고 공통 cohort를 고정. semantic/release 변경 시 reset, OFF/restore 후 새 epoch (§8) | 버전 호환성을 근거로 streak 이월은 별도 동등성 증명 정책 필요 | **Owner 보수적 reset O3 OPEN**, coordinator cohort ABI 확정 |
| D7 복구 | 원장·pending/outbox 보존, 공식 immutable release 사용 | generic settlement/outbox·Daily drain/floor는 존재. R5 control/drain/복원 계약은 없음 | OFF→ELIGIBLE→ENABLED→DRAINING→OFF, 복원은 RESTORED_OFF. 자동 재개 없음 (§9) | pending 자동 폐기/재정산은 감사·중복 위험. 자동 resume는 별도 복원 인수 계약 필요 | **Owner 복구 동작 O3 OPEN**, coordinator migration/운영 소비자 확정 |

## 3. D1 — 입력 identity와 기존 권리

### 3.1 서로 다른 세 입력 체계

| 입력 종류/버전 | 원본 identity | 무엇을 증명하는가 | Paper에서의 제안 용도 |
|---|---|---|---|
| 역사 `historical_price_only_v3`; 추천 payload `owner_beta_historical_price_only_v1` | candidate content, artifact manifest, Stage5 manifest, action manifest, approval registry의 **5-pin** | 승인 v3 base: 2016-08-29~2026-08-28, 2,452세션·26,972 bars. 새 날짜나 generic READY를 증명하지 않음 | Daily의 검증된 base와 warm-up. 역사 수동 재현은 별도 표기하며 무인 관측에 포함하지 않음 |
| Daily `owner-beta-daily-etf11-{policy,manifest,admission}-v1`; 추천 input kind `owner_beta_daily_etf11_price_only_v1` | content/manifest/admission/policy 해시, base 5-pin, parent admission, source evidence; 추천에 config/activation epoch | 원본 Raw 재검증·독립 admission·base/prefix 계보. 파일 capability와 현재 DB 권한은 별도 | T 신호 입력과 E 실행/평가 입력을 각각 봉인·재검증. T pin을 E pin으로 덮어쓰지 않음 |
| generic `DatasetPin` | UUID `id`, `dataset_id`, `version`, `curated_version`, bare manifest SHA256 | 등록된 dataset_versions와 attested Curated partition의 identity | 기존 generic Paper용으로 유지. Daily hash를 UUID/manifest 필드에 옮겨 채우지 않음 |

정의 근거: [역사 pin](../../crates/market-data/src/historical_price_only_approval.rs#L69),
[Daily wire base/policy](owner-beta-daily-etf11-input-v1.md#2-policy-and-base-objects),
[Daily payload](../../crates/job-queue/src/owner_beta/daily_input.rs#L59),
[generic pin](../../crates/job-queue/src/recommendation/input.rs#L22).
역사 materialization의 `UNREGISTERED/NOT_PUBLISHED`는 현재
[release gate](../../scripts/ops/compose-release.sh#L560)에서도 명시된다.

**권장 adapter:** `ApprovedHistoricalPriceOnlyArtifact`와
`AdmittedDailyPriceInputV1`/`read_sealed_daily_chain`의 실제 reader를 재사용한다.
무권한 wire 파싱으로는 실행 타입을 생성할 수 없다. 읽은 파일의 identity/bytes와 DB의
Owner·권리·admission/ancestor revoke·source conflict를 함께 검사한다.
[opaque Daily input](../../crates/market-data/src/daily_price_only/input.rs#L1)은 파일의 증명이며 DB 권한은 아니다.

DB에는 별도 Owner-beta lineage variant와 `owner_beta_recommendation_runs` 참조를 추가한다.
기존 `recommendation_run_id`는 generic `recommendation_runs` FK이므로 재사용해 위조하지 않는다.
기존 preview의 generic target_portfolio FK도 같은 문제다
([0038:96](../../migrations/0038_paper_rebalance_previews.up.sql#L96),
[0038:165](../../migrations/0038_paper_rebalance_previews.up.sql#L165)).
권장 저장 방식은 기존 preview/pending에 명시 beta variant와 all-or-none lineage 제약을 추가하고,
별도 beta provenance/session 테이블에 날짜별 pins를 보존하는 것이다. beta를 `LEGACY`로 보내지 않는다.
현재 [dataset pin이 없을 때의 fallback](../../crates/api-server/src/paper_session.rs#L158)은 beta에 금지한다.

### 3.2 권리 — 해결된 승인과 아직 확인하지 않은 runtime을 분리

[ADR-0005:13](../decisions/0005-kis-personal-use-entitlement.md#L13)은 승인된 read-only KIS 자료의
개인·단일 Owner 수집/보존/사용을 확인했고, 철회·범위 변경 없이는 재확인을 요구하지 말라고 정했다.
[체크인 권리 구성:23](../../configs/data-rights/kis.entitlement.json#L23)에 `recommendation`,
`backtest`, `benchmark`, `paper_view`가 있으며, 최초 범위는 2016-08-29 이후다.
[2026-08-23 채택:20](../superpowers/plans/2026-08-23-owner-readonly-beta-launch.md#L20)은
Paper를 2차 단계로 포함한다. 따라서 **새 Paper 권리 승인을 요청하지 않는다**.
이는 현재 DB row·protected config·유효일·철회 상태를 이번 작업에서 확인했다는 뜻은 아니다.

현재 [Daily SQL:418](../../migrations/0060_owner_beta_daily_inputs.up.sql#L418)은 정확한
reference/hash와 `krx_eod_bars/recommendation`을 검사한다. generic
[Paper SQL:562](../../migrations/0041_paper_settlement_outbox.up.sql#L562)은 recommendation use를,
[HTTP:535](../../crates/api-server/src/http/paper.rs#L535)는 `PaperView`를 사용한다.
R5 권장은 동일한 기존 entitlement reference/hash 아래 현재 Owner·ACTIVE·데이터·기간과
`recommendation` + `paper_view`를 API/worker 양쪽에서 재확인하는 것이다.
benchmark가 계산되면 기존 `benchmark`도 확인한다. 새 use 문자열 `paper_trade`를 만들지 않는다.
권리 효력은 실제 처리일과 소비 자료 범위를 모두 포함해야 한다. 철회 이후 새 적용·체결·평가는 차단하고
이미 기록된 원장은 보존한다. 조회 허용 범위도 현 권리와 인증 정책을 따른다.

## 4. D2 — 첫 금융 계약 권장값 (Owner O1 미채택)

기존 generic Paper의 금융 코드가 존재한다는 사실은 이 계정의 금융 설정이 채택됐다는 증거가 아니다.
R4의 [v2 채택 기록](../superpowers/plans/2026-10-26-r4-baseline-expansion-adoption.md#L3)은
원본 [v1 계산안](../superpowers/plans/2026-10-23-r4-price-only-simulation-contract-draft.md)과
[v2 확장안](../superpowers/plans/2026-10-26-r4-baseline-expansion-contract-draft.md)을 R4에 고정했다.
R5는 아래 묶음을 명시적으로 선택할 때만 운영 config로 만든다.

| 항목 | 권장값 | 기존 사실/대안과 영향 |
|---|---|---|
| 첫 전략 | `trend_following/1.0.0`; `benchmark_instrument=069500.KRX`, `fast_ma=50`, `slow_ma=200` | 기존 target 계산의 `trend_50 < trend_200`이면 투자, 동률 현금. 새 SMA 구현 금지. 다른 네 전략은 후속 범위 |
| 일정 | 매 세션 T 추천·검증. 최초 1회 및 0%↔100% 신호 전환에만 E 재조정; 그대로면 정상 무거래 기록 | 매일 잔여 현금을 재투자하면 R4 trend와 다른 전략. q=0도 다음 신호 전환까지 자동 재시도하지 않음 |
| 첫 자금/통화 | `10000000.0000` KRW, 새 Paper 계정 하나. 초기 입금 1회만, 이후 입출금 없음 | generic 계정 생성은 양수 자금 인자를 받음. 기존 계정 초기 현금/이력을 덮어써 재사용하지 않음 |
| 수량/정렬 | lot=1을 명시 검증, 정수 주, 매도→매수, 각 방향 종목 ID 오름차순 | 현 generic sizer 기본 lot도 1. 음수 수량·공매도·차입·leverage 없음 |
| 금액/반올림 | 가격·현금 4자리, 비중 ppm 6자리. 입력 가격은 4자리에 정확히 표현 가능해야 함. 계산은 기존 checked fixed-point 및 ties-to-even; 정수 수량/예산은 sizer의 명시 내림 | R4 비용의 0.0001원 단위 올림과 같지 않음. overflow·비유한 값·음수 현금은 전체 session 실패 |
| 매수 가능성 | 기존 Paper sizer 사용: 매도 후 가용 현금을 주문가치 비례 배분, 마지막 canonical buy에 예산 잔여, 최소 수수료 예약 후 가능한 lot로 내림 | R4 v2는 비용 전 목표 수량을 고정하고 비용 포함 부족예산 비례배분·잔여 재배분 없음. 동일 알고리즘이라고 표시 금지 |
| 비용 ID/version | 기존 `KRX_ETF_DEFAULT@1`을 **예시 모형**으로 명시 채택 권장 | 기본 구현은 operator 확인 전 placeholder. 실제 증권사 비용/세율로 소개하지 않음 |
| 비용 상세 | 양쪽 commission `0.00015`, 최소 commission `1000.0000` KRW/체결, 매도 tax `0`, slippage 양쪽 `10`bp, min_trade `100000.0000`, rebalance threshold `0.005` | R4는 양쪽 10bp 통합 비용 또는 0비용, 최소/세금/slippage=0. R4와 숫자 일치 불가 |
| slippage | 매수 `raw_open×1.001`, 매도 `raw_open×0.999`; 현금은 실제 모형 fill notional과 commission+tax만 반영 | informational slippage를 현금에서 다시 차감하면 이중 비용. 원시 시가와 모형 체결가 모두 표시 |
| 현금 | 잔액 무이자, 신호 유지 중 자동 추가매수 없음, 추가 입금/출금·종료 강제청산 없음 | 예산 배분 잔여와 실제 미체결 잔액을 구별하여 보존 |
| 배당/기업행위 | 현금배당 제외. Daily 정책의 비현금 action 차단을 유지하며 raw/adjusted 불일치나 미지원 action이면 실행 중지 | 일반 KIS bonus normalizer가 있다는 이유로 Paper 분할 원장을 가정하지 않음. 배당 재투자·총수익은 새 계약 필요 |
| benchmark/성과 | 같은 시작 E·자금·가격·비용·lot의 069500 최초 매수 후 보유 모형. 최종 강제청산 없음. 가격수익률·현금·비용·일별 NAV만 우선 표시 | benchmark는 별도 비교 계산, 주문 계정 추가가 아님. 계산/계보가 없으면 UNAVAILABLE. 연환산/Sharpe는 새 규약 없이 추가하지 않음 |

구현 근거: [cost placeholder/resolver](../../crates/portfolio-model/src/cost.rs#L46),
[정확한 값](../../crates/portfolio-model/src/cost.rs#L138),
[sizer 규칙](../../crates/portfolio-model/src/sizing.rs#L8),
[매수 예산/수량](../../crates/portfolio-model/src/sizing.rs#L377),
[money 반올림](../../crates/domain/src/fixed.rs#L763),
[계정 초기 입금](../../crates/api-server/src/repos/accounts.rs#L75).

**O1 대안:** R4와 동일한 `ILLUSTRATIVE_10BP_PER_SIDE_V1` 및 R4 수량/반올림을 R5의 별도
versioned 정책으로 구현한다. 재현 비교에는 유리하지만 generic `CUSTOM`은 현재 선택 불가
([cost.rs:119](../../crates/portfolio-model/src/cost.rs#L119)); 새 resolver/프로필과 focused 검증·파일 소유권
확장이 필요하다. 기존 KRX_ETF_DEFAULT나 R4 canonical 결과는 변경하지 않는다.

## 5. D3 — 날짜와 가용 시각, T와 E의 다른 pin

| 필드 제안 | 의미 | 권위/금지되는 추정 |
|---|---|---|
| `signal_date`, `as_of` = T | 성공 추천이 사용한 마지막 세션 종가 날짜 | owner-beta run과 봉인 input. 실행 E의 가격을 신호에 역주입 금지 |
| `effective_session` = E | T 바로 다음 검증된 XKRX TRADING 세션 | 달력 원본+row hash·source conflict 검사. 평일 산술/서버 현재 날짜로 대체 금지 |
| `source_original_retrieved_at` | Raw terminal original batch의 수집 시각/정밀도 batch | 복사본의 현재 retrieved_at으로 원본 가용성을 앞당기거나 새로 만들지 않음 |
| `input_admitted_at` | DB에 admission이 commit되어 소비 가능해진 관측 | 기존 admitted_at은 원본 identity와 결합. provider 발표 시각으로 부르지 않음 |
| `usable_at` | 위 자료·admission·필수 검증 완료를 모두 관측한 보수적인 앱 소비 가능 시각 | 새 runtime witness 필요. `available_at`을 쓴다면 반드시 `APPLICATION_OBSERVED`와 precision을 동반. 실제 시장 발표 시각은 UNKNOWN |
| `processed_at`, `settled_at`, `valued_at` | 실제 처리/DB commit 관측 UTC 시각; UI는 KST | 새 receipt/DB clock. effective date에 09:00/15:30을 붙이지 않음 |
| `processing_date` | 처리 시각의 KST 날짜 | 재처리하면 E와 다를 수 있음. CLI `--date`만으로 과거 실행 증거를 만들 수 없음 |

원본 수집 계보는 [Daily file evidence](owner-beta-daily-etf11-input-v1.md#3-source-file-identity-request-and-copied-precision)에
정의돼 있다. 역사 bar의 acquired_at은 [수집 출처](../../crates/market-data/src/historical_price_only.rs#L99)다.
현재 generic Paper는 [00:30 UTC 합성 stamp](../../crates/job-queue/src/paper_execution.rs#L107)로
orders/fills/cash를 쓰고 이를 [중복 판단](../../crates/job-queue/src/paper_execution.rs#L412)에 사용한다.
R5는 이를 실제 체결 시각으로 표시하지 않는다. 신규 beta session identity로 중복 판단을 만들고
`effective_session`과 실제 처리 시각을 분리한다. 과거 generic 행은 재작성하지 않는다.

### 5.1 날짜별 계보 mapping

| 단계 | 고정하여 보존할 입력 | 다음 단계의 검증 |
|---|---|---|
| T EOD 수집·admission | base H(5-pin), A_T(content/manifest/admission/policy), 원본 Raw·calendar/action evidence | T까지 연속 prefix, ETF11 완전성, source conflict/권리 확인 |
| T 추천·preview | A_T, run/job/attempt/reservation, strategy/config/epoch, factor hash F_T, target hash W_T, raw close(T), account state version/hash | READY는 원장 권한이 아님. 실행 E 달력 identity도 고정 |
| T apply 또는 auto-apply | 위 identity+Owner 동의+account+E+금융 정책, idempotency identity | 아직 A_E가 없는 것이 정상. target만 저장, 주문·fill 없음 |
| E EOD 지연 정산 | **별도 A_E**, raw open(E), T→E 달력 연결, T prefix를 보존한 동일 base/policy 연장 | A_E가 A_T의 검증된 descendant이고 T 이하 bytes/신호 동일. 새 data hash를 허용, 동명 manifest 동등 취급 금지 |
| E 종가 평가 | A_E 또는 동일 E를 증명하는 명시 valuation pin, raw close(E), 확정 원장 fingerprint | 모든 보유 종목 mark 및 현금합 일치. 평가 재실행 동일값 no-op, 다른 값 conflict |
| 다음 날짜 | A_next, 새 F/W·run identity | 공통 cohort만 동일; 날짜별 hash는 각 원본과 독립 대조 |

**권장 EOD 지연 정산 (O2):** [현재 수집 cutoff](../runbooks/owner-beta-daily-input.md#r2-source-collection-and-legacy-publication)를
유지한다. T 추천/자동 적용은 T의 16:30 이후부터 KST 날짜 종료 전까지, E 정산/종가 평가는
E의 EOD 자료 admission 이후부터 E의 KST 날짜 종료 전까지 완료하는 경우만 해당 세션 무인 성공이다.
마감 표현은 `next_civil_midnight_exclusive`로 고정하여 초/나노초 경계 모호성을 피한다.
시가 체결은 **나중에 계산한 모형**이며 E 장 시작 시 시스템이 체결을 끝냈다는 뜻이 아니다.

- E 자료가 아직 없으면 `WAITING_PRICE`/`WAITING_ADMISSION`; 원장 변화 없음. 현재 generic의 가격 오류→SKIPPED를 그대로 적용하지 않는다.
- 휴장일은 완료 세션 수에 넣지 않는다. 정확한 CLOSED 증거를 사용하며 다음 TRADING 날짜를 이미 고정한 E로 유지한다.
- T 추천이 다음 civil date에 생성되거나 E가 apply 시점에 도래하면 해당 무인/수동 next-session apply를 거절한다. 날짜를 뒤로 옮기지 않는다.
- 마감까지 필요한 자료가 없거나 모호하면 `BLOCKED`·streak reset. 전일 시가/종가, quote/WS, 다른 공급자로 대체하지 않는다.
- 복구 시 이미 적용된 target은 E 가격이 뒤늦게 확인돼도 자동 정산하지 않고 `RECOVERY_REQUIRED`에 둔다. 명시 복구 절차에서 같은 E로만 재개하며 `RECOVERY`로 기록, soak 제외.
- 누락 E를 해결하기 전 같은 계정의 뒤 세션을 실행하지 않는다. 현재 포지션으로 과거 valuation을 역산하지 않는다.
- admission/가격 bytes가 conflict/철회되면 과거 원장을 고쳐 맞추지 않고 원인과 영향 세션을 보존·차단한다.

## 6. D4 — 수동과 무인 적용

기존 [승인된 preview 설계:130](../superpowers/specs/2026-08-13-recommendation-paper-rebalance-preview-design.md#L130)의
수동 동의와 scheduled opt-in 구분을 유지한다. R5 권장 규칙은 다음과 같다.

1. `OFF`에서는 이력·상태를 권한 내 읽을 수 있지만 새 preview/apply/settlement는 허용하지 않는다.
   `ELIGIBLE`도 ON이 아니다. 활성화 후 수동 apply와 auto-apply를 별도 origin으로 기록한다.
2. Owner가 account/strategy/config/financial contract를 확인해 명시적으로 opt-in한다.
   기존 `auto_apply_recommendations=true`만으로 beta 운영 동의·eligibility를 대체하지 않는다.
   동의 이벤트는 인증 actor·시각·대상 config hash·동의 버전을 저장하되 session token은 보존하지 않는다.
3. manual preview는 성공한 exact beta run의 T close로만 계산한다. account version/hash·binding·target·권리·E가
   변하면 `STALE`; apply에서 원장을 쓰지 않는다. 기존 generic 설계는 computation 날짜 이후 세션을 택하지만,
   R5는 최신 T와 바로 다음 E 조건을 추가해 오래된 추천을 나중 세션으로 자동 이월하지 않는다.
4. auto-apply는 authoritative Daily 성공+원본 예약 witness+opt-in을 확인한 뒤 target을 저장한다.
   키는 `(owner, account, effective_session)`의 기존 유일성을 보존하고 run/target/input/config/consent를 비교한다.
   동일 요청 재전송은 같은 target, 다른 내용은 `TARGET_CONFLICT`; 수동/자동 충돌도 overwrite하지 않는다.
5. 신호 유지·현금·비용상 lot 불가 등 정상 무거래는 `NO_TRADE`와 typed reason을 저장한다.
   이 기록이 있어야 정상 무인 세션으로 셀 수 있다. 현재 `SKIPPED` 문자열만 보고 성공으로 세지 않는다.
6. 동의 OFF/취소·운영 OFF는 epoch를 바꾸고 새 적용/실행을 fence한다. commit 전 target은
   `HELD` 또는 명시 `CANCELED`로 보존; 이미 commit된 fill을 취소했다고 하지 않는다.
   동시 실행은 account/control lock의 선형화 순서로 판정하고 commit 증거를 먼저 대조한다.
7. 같은 epoch/target의 자동 retry는 제한된 transient 오류만 허용한다. 결과 불명 commit은 원본 id로 readback하고
   새 target/입금/주문을 생성하지 않는다. terminal 실패·취소는 기존 row를 PENDING으로 되돌리지 않는다.
   복구 승인은 같은 원본을 참조하는 append-only recovery event이며 이미 실행된 원장에는 재실행하지 않는다.

실제 구현 차이: [현재 SessionOutcome](../../crates/api-server/src/paper_session.rs#L45)은 정상 무거래·차단·실패를
`SKIPPED`로 모으고, [outbox](../../crates/api-server/src/repos/pending_targets.rs#L309)는 terminal target과 원자 연결된다.
R5 typed detail을 추가하되 generic 결과 의미와 outbox 재시작 보장을 유지한다.

## 7. D5 — 증거의 권위와 checker 공개 인터페이스 초안

### 7.1 기존 생산자와 부족한 증거

| 실제 생산자/저장소 | 존재하는 field와 terminal 의미 | 새로 필요한 것 / 현재 없는 것으로 취급 |
|---|---|---|
| sealed Daily manifest/admission | source original/copy chain·hash, base/parent, producer `code_commit`, `verifier_code_commit`, 날짜·행수 | 실제 실행 binary/image를 관측한 binding, scheduler origin. 선언 commit은 runtime 증명이 아님 |
| `owner_beta_daily_poll_receipts` | receipt UUID, stage ADMISSION/SCHEDULE, Owner/policy/activation epoch, started/completed, result_code, source frontier/admission/event/completion binding | timer 발생 원본, KIS 수집 invocation·실제 종료, 정확한 recommendation run 연결. VERIFIED는 입력 검증, CHECKED는 schedule poll 완료 |
| `owner_beta_daily_reservations` / beta run / jobs / attempts / publication claims | 날짜별 run/job, strategy/config/epochs/admission, 성공 run의 factor/target hash, attempt/claim/lease, 각 terminal 상태 | 사람이 시작한 source 수집을 배제하는 end-to-end origin; 실제 image/config digest 결합 |
| producer/admitter `Checkpoint` | stage·Owner·policy·generation·last_started·pending candidate 또는 receipt id | 세션 성공 이력 아님. `pending=None`을 성공으로 해석 금지 |
| generic pending / ledger / daily_equity / settlement outbox | account/owner, E, generic pins, EXECUTED/SKIPPED, 원장과 알림 의무, 일별 평가 | beta pins·NO_TRADE/WAITING/실패 구분·실제 처리시각·run→ledger evidence 연결 |

근거: [DB receipt:160](../../migrations/0060_owner_beta_daily_inputs.up.sql#L160),
[reservation:188](../../migrations/0060_owner_beta_daily_inputs.up.sql#L188),
[publication claim:203](../../migrations/0060_owner_beta_daily_inputs.up.sql#L203),
[Daily scheduler:85](../../crates/job-queue/src/owner_beta/runner/daily_schedule.rs#L85),
[checkpoint:19](../../crates/job-queue/src/owner_beta/runner/daily_local_state.rs#L19),
[target hashes:20](../../crates/job-queue/src/owner_beta/daily_target.rs#L20).

### 7.2 B 권장 — 미래 slot을 직접 소유하는 scheduler/witness (PROPOSED)

**A는 채택하지 않을 것을 권장한다.** 확인한 systemd D-Bus `JobNew/JobRemoved`에는 timer/manual
호출 원인이 없고 `TriggeredBy`는 정적 관계다. 마지막 trigger 시각과 service 실행을 시간 근접으로 결합하면
manual/Persistent catch-up과 race를 배제하지 못한다. **B 하나를 구현안으로 제시한다.** 로컬 문서 근거와
한계는 [D5 보완 보고](../reviews/2026-10-29-r5-contract-gap.md#31-d5-보완--b-단일-권장안의-근거와-검증)에 있다.

**신뢰 주체/저장:** 공식 R5 installer의 root 장기 실행 service가 소형 Python scheduler를 실행한다.
systemd는 프로세스 수명 관리만 하고 실행 기원을 대신 증명하지 않는다. scheduler는 glibc `timerfd_create`
/`timerfd_settime`을 stdlib `ctypes`로 호출해 아래 due event를 직접 소비한다. 새 의존 라이브러리나 공급자 API는 없다.
원본은 `<LAGRANGE_DATA_DIR>/owner-beta-paper-evidence/events/`의 root:10001 0440 파일, root:10001 0750
디렉터리에 순서대로 fsync/no-replace 게시한다. `.control/`은 root:root 0700, lock/state는 0600이다.
worker/app/admitter는 원본을 쓸 수 없다. 경로·부모·ACL·symlink/hardlink·크기·event-chain 검증을 적용한다.
root/설치된 코드·커널·Docker daemon은 신뢰 기반이며, 침해된 root의 거짓 서명을 hash로 탐지한다고 주장하지 않는다.

**identity:** `slot_id=H(schema,Owner,source_policy,cohort,KRX_session,scheduled_slot_utc)`;
별도로 `(Owner,source_policy,KRX_session)`의 일일 dispatch 유일성을 cohort 변경 후에도 유지한다.
`arm_id`는 boot_id+service generation+random nonce, `reservation_id`/`invocation_id`는 한 번 생성한 UUID다.
각 event에는 순번·previous hash·UTC/BOOTTIME/MONOTONIC 관측·실제 config/release/image hashes가 붙는다.
service `InvocationID`는 보조 세대 식별값일 뿐 SCHEDULED 판정 입력이 아니다. token은 독립 권한이 아니며
slot/cohort/arm/invocation에 묶인 nonce와 **동일 부모가 생성·보유한 inherited private FD**가 함께 필요하다.

| 상태/원본 event | 전이 조건과 실행 | 실패·재시작·무기록 의미 |
|---|---|---|
| OFF → STARTED | 공식 enable config·단일 scheduler lock·root 원본/DB cohort 일치, 현재 boot/process identity 기록 | 수동 service 시작도 즉시 수집하지 않음. 과거 slot은 MISSED_CATCH_UP; 실행 없는 날짜를 성공으로 mint하지 않음 |
| STARTED → ARMED | 검증된 XKRX TRADING T의 **16:30 KST**만 선택. calendar/config/release를 고정한 ARM_INTENT fsync→timer 설정→ARMED fsync 후에도 due까지 최소60초 남았는지 확인 | CLOSED는 NO_SESSION, 달력 누락/충돌은 BLOCKED_CALENDAR. timer를 이미 지난 시각으로 arm하거나 durable 완료가 늦었으면 MISSED_UNARMED; 즉시 expiry를 성공으로 mint하지 않음 |
| ARMED → RESERVED | `CLOCK_REALTIME` 절대 one-shot timerfd + `TFD_TIMER_ABSTIME\|TFD_TIMER_CANCEL_ON_SET`; 같은 살아 있는 arm 프로세스가 expiry 1회를 읽고, due≤now≤due+60초·같은 T·clock/cohort를 재검증 | ECANCELED·clock discontinuity·suspend 또는 늦은 wake는 CLOCK_INVALID/MISSED_LATE. polling의 now≥due만으로 발사 금지 |
| RESERVED → LAUNCH_INTENT → RUNNING | 디스크 일일 reservation과 DB expected slot 등록을 먼저 commit/readback, invocation·고정 argv 기록. launch 직전에도 due+60초/clock/cohort 재검증. 부모가 직접 `Popen(shell=False, pass_fds=...)`로 기존 wrapper를 한 번 spawn하고 PID/starttime·자식 channel을 결합 | spawn 전후 crash/불명 결과는 UNKNOWN. restart가 intent를 재전송하거나 같은 slot을 다시 scheduled로 실행하지 않음 |
| RUNNING → SOURCE_FINISHED | wrapper/모든 소유 child 종료·reap, 정확한 Docker create/start/exit/image/config 관측, bounded typed 결과·Raw original 재검증을 모두 확보 | 비정상 exit·프로토콜 누락·image 불일치·관측 단절은 FAILED/UNKNOWN. receipt에 성공 문구만 있어도 불충분 |
| SOURCE_FINISHED → LINKED | 아래 순서대로 admission·추천 run/attempt·원본을 결합하고 DB readback, root LINKED event를 마지막 게시 | admission/run 실패·마감 초과는 BLOCKED/FAILED. original source 성공만으로 무인 세션 성공 아님 |
| 재시작/중지 | 이전 ARMED/RESERVED/RUNNING을 원본 그대로 보존, terminal 미확정은 INTERRUPTED/UNKNOWN. 기존 자식·named container를 확인/정리하고 새 launch 금지 | 과거/이미 claim된 slot은 CATCH_UP/RECOVERY만 가능하며 새 명시 복구 gate 필요. 이전 ARMED slot도 새 프로세스에서 이어받아 성공으로 mint하지 않음 |

60초 arm 여유/launch 허용폭은 새 **보수적 제안값**이며 API request budget을 늘리지 않는다. due 전·실행 중
REALTIME/BOOTTIME/MONOTONIC을 관측하고 시계 step·suspend를 fail-closed로 처리한다. 실행 중에도 미래
guard timer를 cancel-on-set으로 유지한다. 인접 관측의 realtime−boottime 차이 변화가1초 초과 또는
boottime−monotonic 증가가1초 초과면 CLOCK_INVALID; 정상 slew 자체를 미래 가격 가용성으로 해석하지 않는다.
unsupported clock/API, boot 변경, calendar/config 교체는 arm 폐기다. 이미 진행 중이면 기존 정상 종료/회수
계약을 사용하고 재발사하지 않는다. 정상 provider 내부 bounded retry는 같은 invocation에 포함하며,
wrapper 자체 재실행·수동 source·Persistent catch-up은 scheduled 성공에서 제외한다.

**수동/중복 경계:** 수동 wrapper에는 scheduler의 살아 있는 child channel이 없으므로 MANUAL이다.
수동으로 scheduler를 미래 slot 전에 시작한 것은 service 준비일 뿐이며, 이후 위 arm/due 경로로 실행한
미래 slot만 scheduled가 될 수 있다. `--run-now`, supplied origin bool, 문자열 token만의 attach는 제공하지 않는다.
공식 installer는 기존 KIS timer와 B scheduler의 중복 dispatch를 허용하지 않고 old timer disable/drain과
이미 실행/대기 중인 수집을 대조한 뒤 전환한다. 일일/Raw/calendar/action 기존 lock·예산·cutoff는 유지한다.
claim 없음은 성공 없음이며, checkpoint나 event 누락도 실패와 구별되지 않는 **불완전 증거**다.
checker는 성공 목록만 읽지 않고 설치된 최초 slot~현재의 기대 일정과 모든 arm/claim/terminal을 대조한다.

**정확한 join 순서:**

1. root의 RESERVED 원본 hash를 DB에 등록/readback한 뒤 launch한다. DB 등록은 동일 slot/예약만 replay 가능하며
   false success 인자를 받지 않는다. root 파일 없이 DB row만 있어도 scheduled 권위가 되지 않는다.
2. 기존 wrapper의 provider-free precheck→현재일 capture→action supply 순서를 유지한다. witness mode만
   연구 worker 컨테이너 이름에 invocation/phase를 묶고 `--rm`을 제거해 종료 객체를 보존한다. launch 전
   같은 이름의 객체가 없어야 하며 부모가 실제 ID·Created/StartedAt/FinishedAt·exit/OOM·image/config를
   Docker inspect로 확인하고 원본을 durable 게시한 뒤 정리한다. 객체 조기 삭제/교체·ID 누락은 거절한다.
   원래 wrapper의 네트워크 요청/재시도 수는 늘리지 않으며 기존 timeout 내에서만 메타데이터를 처리한다.
3. existing worker의 `raw_committed(batch_id,target_date,reused_existing_source)`를 해당 자식의 전용 FD로
   손실 없이 전달하고 부모가 정확한 Raw batch/manifest line/file hashes와 원본 수집 시각을 다시 읽는다.
   날짜만 같은 새 파일을 directory diff로 추정하지 않는다. 기존 progress TSV는 batch ID를 버리므로 확장 필요다.
   preexisting source 또는 `reused_existing_source=true`는 신규 scheduled source 성공으로 인정하지 않는다.
   다른 writer가 precheck 뒤 먼저 commit해도 기존 CalendarDayLock 아래 재사용 판정이 이를 거절한다.
4. current T bars/reference와 그 수집에 필요한 current-day calendar original은 한 slot에만 귀속한다.
   과거 입력 prefix/calendar/action의 합법적 재사용은 dependency 참조로 남겨 새 capture라고 부르지 않는다.
   현재일 달력을 과거 calendar로 대체하지 않는다. **추가 producer 필드**로 CalendarDayLock 안에서 수집 전
   확인한 `calendar_claim_preexisting`과 실제 `calendar_batch_id`를 전달한다. 현 `reused_existing_source`는
   이 calendar 재사용 여부를 말해주지 않는다. current-day 선행 claim=true/누락이면 이번 무인 성공은 거절하며
   날짜·수집 시각의 근접성으로 보완하지 않는다. claim을 지우거나 달력을 재호출하지 않는다.
   action supply의 기존 reserved range/claim/정확한 chains와 실제
   결과를 결합한다. 불명/이미 소비한 claim은 다시 호출하지 않는다. 중복 original 귀속·빠진 실패 event는 거절한다.
5. 독립 admitter가 root source 원본과 실제 sealed admission의 source keys/base/parent/policy를 대조해 DB에
   slot→original digest→admission을 append-only 연결한다. 추천 성공 후 worker는 기존 immutable DAILY
   reservation의 날짜/Owner/config/epochs/admission과 run/job/attempt/F/W hash를 결합한다. 추천이 먼저 끝났으면
   뒤늦은 **identity linkage**만 가능하며 run 생성 시각·outcome은 바꾸지 않는다. 두 단계 실패/UNKNOWN은 유지한다.
6. Rust checker는 root event bytes/ownership/전이/시간/cohort와 sealed originals를 직접 재검증하고,
   DB narrow reader의 저장 identity·현재 rights/revokes·실제 terminal들과 대조한다. DB reader는 FS나 timer의
   진위를 증명하지 않는다. root는 DB SUCCEEDED를 스스로 만들지 않는다. 모든 join 완료 후에만 §7.4 성공이다.

원본 event 필수값은 위 identity·clock 외에 parent/child PID/starttime, 고정 argv hash, 실제 container ID/
image ID·release manifest hash·비밀을 제외한 semantic config hash·create/start/exit/OOM 상태, wrapper exit,
raw batch/manifest/file digest·reused flag·action outcome/claim/range, DB 예약·admission/run link IDs다.
Rust는 FS owner/mode·bytes/hash/순서·arm→dispatch·원본 체인을 재검증하고 DB reader는 실제 저장 identity/
terminal/현재 권한을 별도 확인한다. root event의 clock/child 관측을 독립 재관측하는 것은 아니다.
provider 본문·credential은 receipt에 넣지 않는다. 새 실제 activation/role/FD wiring·runtime PASS는 아직 없다.
선행 구현 WP-2A의 정확한 파일·DB 함수/역할과 실제 Linux integration 조건은 보완 보고에 고정한다.

### 7.3 타입·결과·소비자 (새 이름은 모두 제안, 구현 존재 주장 아님)

| 공개 경계 제안 | 입력 → 결과 | 필수 소비자 |
|---|---|---|
| `decode_paper_contract_v1(bytes)` | 제한 bytes → `UntrustedPaperContractV1` | installer/CLI/API config loader. deserialize는 권한 없음 |
| `verify_paper_input(ctx, signal_pin, execution_pin?)` | protected readers+DB transaction+현재시각 → private-constructor `VerifiedPaperInputV1` | preview·apply·auto-apply·settle·valuation |
| `read_unattended_session(ctx, date, cohort)` | 원본 source witness+DB IDs+sealed data → `VerifiedUnattendedSessionV1` 또는 typed rejection | eligibility checker. fixture reader와 production reader 분리 |
| `check_paper_eligibility(ctx, expected_cohort, now)` | 최신 3개 연속 세션을 직접 discovery·검증 → `Eligible` 또는 `Ineligible {reasons}` | 공식 ops gate 및 API/worker. 임의 성공 receipt 3개를 caller가 골라 제출 못함 |
| `authorize_paper_action(tx, verified, expected_epoch, action)` | 현재 Owner·권리·cohort·원장/state 잠금 → 짧은 transaction-scoped permit | app narrow 함수/worker write. admin pool은 대체 실행 권한 아님 |
| `PaperSessionReceiptV1` | 원장 commit identity+typed result+valuation/outbox linkage | UI·post-soak·backup/restore verifier. worker가 외부 JSON으로 성공 삽입 못함 |

`VerifiedPaperInputV1`은 kind/version, H, A_T와 optional A_E, calendar proof, strategy/config/F_T/W_T,
run/job/attempt, entitlement ref/hash, raw 가격·날짜·usable_at, capability flags를 포함하고
수명 동안 검증된 reader handle을 유지한다. serialize해서 승인 capability로 복원하지 않는다.
API body는 Owner UUID·파일 경로·pins·success bool을 받지 않고 인증 actor와 서버 선택 run/account를 사용한다.

`EligibilityReceiptV1` 제안: `schema`, `verifier_revision`, `evidence_root_digest`, `cohort_sha256`,
`pre_sessions[3]`(날짜별 원본 IDs/pins/outcome), `evaluated_at`, `expires_at`,
`rights_binding_sha256`, `expected_control_epoch`, `decision`, `reason_codes`.
보호된 출력 또는 직접 verifier 결과만 신뢰한다. 복사본/API 출력은 설명용이다.
**유효기간 권장: 평가 뒤 180초 이내이면서 다음 검증된 EOD 기준일 전환 전까지**, 더 이른 조건을 적용한다.
현재 Daily의 [180초 검증](../../migrations/0060_owner_beta_daily_inputs.up.sql#L735)을 바꾸지 않는다.
3일 전 receipt 자체가 180초 안이어야 한다는 요구가 아니다. 과거 원본은 보존하고 현재 checker를 다시 실행한다.
이 TTL은 **활성화 허가**의 유효기간이다. enable transaction이 원본 pre 목록과 fresh 판정을 activation event에
고정한 뒤에는 매 체결마다 “현재 날짜까지 새 pre 3일”을 요구하지 않는다. ON의 action permit은 그 activation
identity와 현 Owner/권리/동의/epoch·source/admission·가격을 재검증한다. 그러므로 E 정산을 위해 먼저 E의
Paper 성공 증거를 요구하는 순환이 없다. 새 enable/restore는 다시 최신3일·fresh activation 판정이 필요하다.

wire 자원 한도 권장값은 contract 64KiB, source invocation receipt 64KiB, session receipt 256KiB,
eligibility receipt 1MiB, JSON 깊이 16, 세션 목록 정확히 3개다. 큰 원본 file 목록은 기존 봉인 evidence의
hash 참조로 연결하고 새 JSON에 무제한 복제하지 않는다. UTF-8 closed schema·중복/unknown key 거절,
정렬된 canonical key/배열·정수형 카운트·ISO 날짜/UTC 시각 규칙을 version에 고정하고 overflow는 거절한다.
이 한도는 새 수집/보존 quota나 provider 호출 승인이 아니다. G1에서 실제 producer와 함께 확정한다.

새 typed reject 제안:
`INPUT_KIND_UNSUPPORTED`, `INPUT_NOT_ADMITTED`, `INPUT_PIN_MISMATCH`, `PREFIX_CHANGED`,
`RIGHTS_UNAVAILABLE`, `OWNER_SCOPE_MISMATCH`, `STRATEGY_BINDING_MISMATCH`, `PRICE_NOT_AVAILABLE`,
`CALENDAR_UNPROVEN`, `SOURCE_REVOKED`, `EVIDENCE_MISSING`, `EVIDENCE_TAMPERED`, `ORIGIN_UNPROVEN`,
`SESSION_NOT_CONSECUTIVE`, `COHORT_CHANGED`, `ELIGIBILITY_EXPIRED`, `PAPER_DISABLED`,
`CONSENT_REQUIRED`, `PREVIEW_STALE`, `TARGET_CONFLICT`, `RECOVERY_REQUIRED`, `READER_INCOMPATIBLE`.
HTTP 공개 이름은 `OWNER_BETA_PAPER_` prefix를 제안하며 비Owner 객체는 기존 `RESOURCE_NOT_FOUND`로 숨긴다.
재시도 가능한 가격 대기/일시 DB 오류와 영구 무결성 실패를 분리하고 free-form SQL/provider 오류를 노출하지 않는다.

### 7.4 terminal 성공과 철회

pre 성공은 `SOURCE_VERIFIED` + `ADMISSION_VERIFIED` + `RECOMMENDATION_SUCCEEDED`의 검증된 결합이다.
legacy Curated/PUBLISHED를 Daily 경로에서 만들어내지 않는다. R2의 승인된 sealed input·추천 publication을
각 원래 authority로 확인한다. Raw commit이나 schedule CHECKED만으로 결합 성공을 만들 수 없다.
post 성공은 위 입력 성공 + 자동 적용/정상 신호유지 + `SETTLED_TRADE` 또는 `SETTLED_NO_TRADE` +
`VALUED` + durable settlement/outbox 의무의 정합성이다. 알림 transport가 미전달이면 backlog로 별도 표시하고
전달했다고 쓰지 않는다. 원장 commit 전/후 crash와 terminal 기록 누락은 UNKNOWN이며 성공 카운트 불가다.
같은 원본 예약 안에서 기존 허용된 bounded transient retry가 당일 마감 전에 자동으로 정상 종결되면
한 세션 성공으로 집계할 수 있다. 모든 attempt를 보존하며 terminal failure·마감 실패를 나중 성공으로 지우지 않는다.

권리/Owner/consent 철회, admission·ancestor revoke, source event, config/release/epoch 변경,
calendar conflict가 생기면 캐시 eligibility를 무효화한다. activation 직전과 모든 write 직전에 재검증한다.
revoked 과거 증거를 삭제하지 않는다. 재검증 실패 시 신규 쓰기를 차단하고 현재 상태를 `BLOCKED`로 투영한다.

## 8. D6 — 공통 cohort와 날짜별 pin

`cohort` 권장 필드: Owner, account, strategy id/version/config hash, 금융계약/hash,
cost id/version/내용 hash, policy/base pins, entitlement ref/hash, Daily config_epoch/activation_epoch,
실제 release manifest/image set와 verifier version, 자동 적용 동의 버전, semantic config hash.
처리 job/run UUID, 날짜별 A_T/A_E·F/W hash, source frontier는 **세션별 값**이며 cohort 동일성에 넣지 않는다.

- 첫 운영 계정·금융/동의 config를 Paper OFF 상태에서 먼저 고정하고 같은 릴리스로 pre-soak를 쌓는다.
  활성화 한 번의 Paper control 전이는 별도 activation event로 이전 OFF epoch와 새 ON epoch를 연결한다.
  pre 세션에 아직 존재하지 않는 ON epoch를 요구하지 않는다. Daily epoch/config와 semantic cohort는 유지한다.
- config 전체 바이트 hash와 semantic cohort hash를 구별한다. OFF→ON selector 변화는 기록된 전이에서만 허용한다.
  새 서비스 image·전략·비용·입력정책·권리 reference·Daily activation 변경은 이 예외가 아니다.
- post 세션 3개는 같은 실제 ON epoch로 묶는다. pre와 post는 별도 목록이고 중복 세션 승격/재사용으로
  post 횟수를 채우지 않는다. E의 정산에는 직전 T 추천이 필요하므로 calendar의 T→E 연결을 각각 확인한다.
- 원본 run/target 같은 identity의 재시도는 한 세션으로만 센다. 날짜별 해시가 달라져도 정상 parent/prefix면 허용한다.
- terminal 실패/누락/수동 개입/catch-up/마감 시간초과/UNKNOWN인 실제 trading session이 사이에 끼면 streak=0.
  CLOSED는 건너뛰되 근거가 없는 날짜를 휴일로 가정하지 않는다. 성공 3개를 임의 골라 모으지 않는다.
- **reset 권장(O3):** 제품 release·semantic config·Owner·strategy·cost·policy·rights 변경, OFF/restore/rollback은
  새 관찰 cohort/epoch를 요구한다. 단순 같은 image 프로세스 재시작은 원본 identity·무중복 recovery를 증명하면
  cohort를 유지할 수 있지만 수동 개입한 해당 거래일은 성공에서 제외한다.
- eligibility는 현재 예상 세션으로 끝나는 최신 연속 3개를 요구한다. 주말/공휴일도 DB·권리·원본을 현재 재검증한다.
  옛 streak를 수개월 뒤 재사용하지 않는다. post 실패는 기록을 지우지 않고 신규 적용을 정지·복구 대상으로 보낸다.

## 9. D7 — 운영 상태, 보존, 호환성

아래 상태는 **새 제안**이다. 현재 env enum이나 accounts.status에 이미 존재하는 값으로 서술하지 않는다.
운영 control과 target settlement, process lifecycle은 다른 상태 축이다.

| 현재 → 다음 | 조건·동작 | pending/outbox·원장 |
|---|---|---|
| OFF → ELIGIBLE | 동일 후보·계약·권리·pre 3세션·현재 checker 통과. 상태 조회 결과만 바뀜 | 신규 거래 없음. 이력 불변 |
| ELIGIBLE → ENABLED | 공식 installer/ops, fresh evidence, 명시 opt-in, actual image/config/DB fence 일치; control enable 마지막 | activation event/new epoch. 준비된 target만 해당 epoch 규칙으로 수락 |
| ENABLED → DRAINING | OFF/철회/장애/backup 요청 시 먼저 DB 새 admission·apply·claim fence, epoch 변경 후 graceful stop | 이미 commit된 원장 확인·terminal/outbox 복구만 수행. 신규 경제적 체결 금지 |
| DRAINING → OFF | 모든 writer/lease/child가 종료·회수되고 commit unknown을 원본 readback으로 해결; fresh drain receipt | 미정산은 HELD/RECOVERY_REQUIRED로 보존. outbox ready/inflight/exhausted/archive를 대조; 미전달을 delivered로 바꾸지 않음 |
| OFF → BACKUP_COMPLETE | 같은 drain generation·DB consistent snapshot·sealed source/admission/정책/config/승인·reader floor·동의·전체 원장/outbox 암호화 보존 및 검증 | 잔여 target 포함. R4/Daily 기존 이력도 보존. DB와 파일의 서로 다른 시점을 성공 backup으로 조합 금지 |
| BACKUP_COMPLETE → RESTORED_OFF | 격리 실제 PG 복원, grants/RLS·migration·입력 hash·원장/target/outbox 대응 검증 | 자동 실행 없음. 외부 알림도 재발송하지 않는 격리. 실제 전달 후 restore된 outbox는 delivery identity 대조 필요 |
| RESTORED_OFF → ELIGIBLE → ENABLED | 현재 권리·입력·epoch·release 재검증, 미정산 목록에 명시 복구 결정, 새 pre-soak와 별도 enable | 과거 입금/fill 재생성 금지. replay는 같은 identity no-op; 바뀐 bytes/금액 conflict |
| ENABLED/OFF → rollback target | OFF/drain 후 호환 reader floor·schema 검사, 공식 immutable release 전환 | Paper 모르는 old reader/원장 손실 migration down 거절. 실패 release·backup·원본 실패 기록 보존 |

backup에는 accounts·bindings·consent/control/history, pending·preview·beta lineage/session evidence,
orders/fills/cash/positions/daily_equity, settlement outbox와 archive/notification identity, jobs/attempts/reservations,
실제로 소비한 base/Daily/Raw 근거·권리·비용·config/release manifest·schema version을 포함한다.
복원 후 latest pointer만 맞춰 성공으로 판정하지 않는다. receipt만 살리고 원본을 버리면 재검증 불가다.
입력 retention은 기존 권리 안에서 정하고 R5 원장이 참조하는 근거의 자동 GC를 허용하지 않는다.

기존 [Daily reader floor/drain](../runbooks/owner-beta-daily-input.md#compatible-rollback)와 R4 보존 절차를
확장해야 하며 대체하거나 설치된 release를 수정하지 않는다. 현재 max migration은 0065;
필요한 신규 schema와 번호 **후보**는 [gap 보고서](../reviews/2026-10-29-r5-contract-gap.md#4-신규-migration과-정확한-파일-후보)에 둔다.
현재 Daily release/operations 자체에도 Paper disabled 전제가 있다. R5 enabled와 Daily owner_only의 공존은
단순 Paper gate 제거로 구현되지 않는다. fresh Paper authority를 검증하는 동일 공식 전환 경로에서만
기존 전제를 확장하고, Daily OFF/drain 복구가 Paper 상태 때문에 막히지 않게 공동 writer inventory를 대조한다.
정확한 충돌 근거와 추가 파일 후보는 보고서 E19에 기록한다.

## 10. R4 비교와 수용 기준

`comparison_status` 제안은 `COMPARABLE`, `EXPLAINED_DIFFERENCE`, `NOT_COMPARABLE`다.
일치 판정은 input 종류/정확한 행·기간·warm-up, signal/target/F hashes, 전략/version/cadence,
초기 상태·현금·lot/rounding·가격 의미·비용/세금/slippage·action 정책·valuation 구간을 모두 대조한다.
raw content hash가 다른 형식 사이에서는 검증된 행 mapping 없이 단순 문자열 일치를 요구하거나 동일하다고 추정하지 않는다.

- R4 역사 마지막 2026-08-28 이후 Daily Paper는 기간과 데이터가 다르다. 기본 판정은 `NOT_COMPARABLE`.
- 같은 기간이라도 Paper 기존 sizer/비용과 R4 10bp·v2 수량 규칙이 다르면 `EXPLAINED_DIFFERENCE`와
  `COST_POLICY`, `SIZING_POLICY`, `ROUNDING`, `SIGNAL_SCHEDULE`, `PRICE_AVAILABILITY` 차이를 표시한다.
  정확한 쌍이나 계보가 없으면 `NOT_COMPARABLE`; 단순 화면 병치는 수익률 차이 검산이 아니다.
- 반올림 허용오차나 기간 선택을 바꿔 parity를 맞추지 않는다. 기존 generic parity를 R4 성과 비교로 재명명하지 않는다.
- 모형 benchmark의 matching input/initial state가 없으면 `BENCHMARK_UNAVAILABLE`. R4 결과·해시는 불변이다.

| 수용 단계 | 반드시 증명할 사례 | 증거 수준 / 본 WP |
|---|---|---|
| deterministic unit·음성 회귀 | 3세션+휴일, 정상 날짜별 hash 변화; 누락/실패/중복/수동/catch-up/미래/다른 Owner·전략·epoch/변조/철회 거절; T→E prefix, 가격 대기·정상 no-trade, 비용/lot/현금 불변식 | synthetic/source. **NOT RUN** |
| 실제 PG/RLS | app/worker 실제 role·FORCE RLS, beta FK/유일성/역할별 생성권한, stale preview, revoke race, 동시 적용·실행, commit 전후 crash, zero-trade idempotency, migration 재적용/보존형 down 거절 | hosted 실제 PG. **NOT RUN** |
| API/worker | enabled bool만으로 통과 불가, actual revision/image·입력 reader, generic 회귀, E 지연 자료로 정산, 실제 시각·typed 상태·outbox recovery | 동일 후보 integration. **NOT RUN** |
| Web | Owner 제한, OFF/부족 증거/대기/복구, 수동 opt-in, preview와 실제 수량 차이, 가격수익률·비교 불가, 실제 API DTO 일치 | Vitest/OpenAPI+공식 Web E2E. **NOT RUN** |
| 운영복원 | actual disabled→enable→fresh drain→encrypted backup→isolated restore→RESTORED_OFF→명시 재개; 원장·pending/outbox·R4/Daily 이력 동등성; old reader 거절 | fixture와 실제 host 결과 분리. **NOT RUN** |
| pre-soak | 동일 cohort의 연속 실제 3 TRADING 세션의 예약 source→admission→추천 publication, 현재 재검증 | 수동/fixture/healthy 제외. **NOT RUN/미확인** |
| post-soak | enable 이후 별도 연속 실제 3 TRADING 세션, auto-apply 또는 정상 신호유지, 지연 정산·종가 평가·원장 정합·Owner 화면 인수 | pre receipt 전용 금지. **NOT RUN/미확인** |

문서 정적 검증은 이 계약의 채택·제품 검증·배포·실제 무인 성공을 뜻하지 않는다.
