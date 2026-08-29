# 고정 30종목 가격·거래량 빠른 베타 실행 계약

Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

## 초기 분류와 실행 그래프

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 artifact·signal core | hard | immutable 증거 계약, 수치 경계, 보안 파일 I/O가 결합된다 | high | fixture로 스키마를 고정할 수 없거나 두 번 이상 검증 실패 시 sol로 상향 |
| WP-2 Raw capture | intermediate | 기존 KIS range·TokenManager 패턴을 별도 scope로 복제한다 | high | 기존 provider 변경이나 새 endpoint가 필요하면 중단 후 hard로 재분류 |
| WP-2B materialize·approval | hard | Raw 응답 파싱, bars/signal artifact, 승인 pointer를 증거 hash로 결합한다 | high | Raw metadata가 artifact 증거 계약과 일치하지 않으면 두 경로를 수정하지 말고 재계획 |
| WP-3 owner-only API | intermediate | 승인 snapshot을 읽는 GET/POST 세 경로와 auth gate다 | medium | DB/queue가 필요하거나 snapshot API가 불안정하면 WP-1 후 재계획 |
| WP-4 Web UX | intermediate | 확정 DTO 위 Top 5·스크리너·상세 화면과 로그인 복구다 | medium | 비동기 Server Component 단위 테스트 한계가 막으면 E2E 범위 확장 |
| WP-5 runtime·release QA | hard | 실제 Raw 수집, artifact 승인, 순차 이미지 빌드와 배포가 결합된다 | high | entitlement/preflight/hash/build/인증 QA 중 하나라도 실패하면 배포 중단 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | hard | bars artifact와 결정적 신호 snapshot | 새 market-data/factor-engine beta 모듈 | 없음 | terra high | provider-free core와 fixture | focused Rust tests, fmt/check |
| WP-2 | 1 | intermediate | KIS Raw-only one-shot 수집 | collectors·sibling ops script·Raw-only Compose | 없음 | luna max | dry-run 기본 capture CLI | collector/static tests |
| WP-2B | 2 | hard | Raw를 bars/signal로 만들고 승인 | provider-free collector CLI·approval registry/wrapper | WP-1, WP-2 | terra high | materialize/check/approve CLI | fixture/tamper/static tests |
| WP-3 | 3 | intermediate | Owner feed/screen/detail API | api-server contract/http/state/OpenAPI | WP-1, WP-2B | luna max | snapshot-only owner API | auth/tamper/DTO/OpenAPI tests |
| WP-4 | 4 | intermediate | Top 5·스크리너·상세 UX | apps/web의 새 beta route/components/contracts/tests | WP-3 | luna max | Owner-only research workspace | type/lint/Vitest/Playwright |
| WP-5 | 5 | hard | 통합·승인·릴리스·직접 QA | deployment/runtime/static checks | WP-1~4 | terra high review + coordinator | immutable installed release | full tests, health, role QA |

모든 WP 작업자는 `/data/worktrees/3puw275b/hungry-zebra`에서 작업하고 서로의 owned
scope를 수정하지 않는다. 보고에는 변경 파일/라인, 명세 이탈, 실행한 검증과 결과,
미해결·후속 항목, 찾거나 확인하지 못한 항목을 반드시 포함한다. 누락 요구를 추측해
메우지 않고 coordinator에게 반환한다.

### Worker briefs

- WP-1: fixed universe와 KIS daily-bars 증거를 검증하는 provider-free immutable bars
  artifact, 20/60/120 가격·변동성·거래량 factor, 결정적 rank/scenario snapshot을 새
  market-data/factor-engine 모듈에 구현한다. candidate/ETF11/collector/API는 금지한다.
- WP-2: dry-run 기본, explicit execute+confirmation, 90 GET ceiling의 전용 Raw capture
  CLI와 Raw-only Compose profile을 구현한다. 기존 ETF wrapper와 materializer는 금지한다.
- WP-2B: WP-2의 immutable batch만 읽어 KIS JSON/OHLCV를 fail-closed 파싱하고 WP-1
  bars artifact와 signal snapshot을 생성·독립 check한 뒤 승인 registry/pointer를 atomic
  교체한다. provider credential·network와 ETF approval wrapper 재사용은 금지한다.
- WP-3: WP-1 공개 verifier를 사용해 승인 snapshot을 매 요청 재검증하는
  `/api/v1/research/owner-beta/equity-price-signals` latest/screen/detail을 구현한다.
  신규 DB·queue·mutation과 Member row 노출은 금지한다.
- WP-4: WP-3 DTO만 사용하는 `/stock-beta` Owner workspace를 구현한다. Top 5, filter,
  instrument detail, provenance/disclaimer, session-expiry login recovery가 필수이며 기존
  candidate/screener/recommendation 계약을 변경하지 않는다.
- WP-5: 앞선 diff를 독립 review하고 fixture/full/static 검증 뒤에만 operator preflight,
  실제 읽기 전용 capture, artifact approval, exact-commit 순차 빌드·설치·브라우저 QA를
  수행한다. 계좌·주문·Live surface와 검증 전 provider 호출은 금지한다.

## 목표와 경계

이 작업은 `2026-08-29-kospi200-production-candidate-activation.md`의 정식 PIT
candidate 계획을 대체하거나 느슨하게 만들지 않는다. 별도의 owner-only 베타로서
`configs/universes/kr-stock-price-beta-v1.json`에 명시한 30종목만 관찰한다.

- 결과는 `OWNER_ONLY`, `vendor_snapshot=true`, `strict_pit=false`,
  `PRICE_VOLUME_RESEARCH_ONLY`다.
- 고정 목록은 KOSPI 200/KOSDAQ 150, 과거 지수 편입, 전 시장 또는 상장 상태를
  주장하지 않는다. 목록 유효일은 2026-08-30이며 과거 가격 범위와 구분한다.
- 허용된 KIS 일봉 GET
  `/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice`,
  TR `FHKST03010100`만 사용한다. 계좌·잔고·주문·WebSocket·Live profile은 금지한다.
- 투자자 수급, 재무, 업종, 목표가, 기대수익률, 확률, 비중 또는 주문 신호를 만들지
  않는다.
- 기존 strict candidate DB/API/feed와 ETF11 recommendation artifact는 변경하지 않는다.

## 사용자 기능

1. 고정 30종목의 가격·거래량 기반 Top 5
2. 20/60/120 거래일 수익률, 변동성, 낙폭, 추세, 20/60 거래량 변화
3. 점수와 최소 거래대금/추세/변동성 조건으로 거르는 단순 스크리너
4. 종목 상세의 원시 팩터, 순위, 데이터 기준일, 조건부 상승/중립/하락 시나리오
5. 모든 화면의 고정 목록·vendor snapshot·비엄격 PIT·연구 전용 표시

## 데이터 및 점수 계약

- 첫 artifact의 요청 범위는 `2025-08-04..2026-08-28`이며 최대 261개의 공통
  관측일을 목표로 한다. 확인된 session은 30종목 모두에 검증된 일봉이 있는 날짜로만
  정의하며 120거래일 수익률에 121개 가격 관측이 필요하므로 최소 121개를 요구한다. 이는 거래소 개장일, 상장 상태 또는 지수 편입을
  증명하지 않는다. 각 종목에 정확히 같은 공통 관측일 집합이 없으면 발행하지 않는다.
- 최대 30종목 × 3 단일-page windows = 90 KIS GET을 예상한다. 호출은 한 프로세스,
  endpoint/TR channel당 초당 1회 이하, 기존 TokenManager 재사용, bounded retry다.
- Raw는 API 응답 바이트와 redacted request metadata를 immutable commit한 뒤에만
  materializer가 읽는다. 중복/누락/잘못된 종목·날짜·continuation·`rt_cd`·OHLCV는
  fail-closed다.
- `FID_ORG_ADJ_PRC=1`을 고정해 원주가(비조정 가격)를 사용하고 artifact에 보존한다.
  기업행사로 수익률·낙폭이 왜곡될 수 있음을 API와 화면에 표시하며, 이 빠른 베타에서
  별도 기업행사 보정이나 수익률 연속성을 주장하지 않는다.
- 점수는 동일 artifact 안 30종목의 결정적 cross-section에서만 정규화한다. 동점은
  canonical instrument ID 오름차순으로 해소한다. 미래 행은 기준일 계산에 들어가지 않는다.
- 상승/중립/하락은 명시적 factor trigger 설명이며 예측 확률이 아니다.

## 구현 패키지

1. 고정 universe parser/hash와 generic KIS range Raw 검증
2. price-volume factor snapshot 및 immutable artifact reader/writer
3. provider-free check와 operator-gated live capture/materialize wrapper
4. DB/queue를 우회하고 승인된 immutable snapshot만 읽는 owner-only API
   feed/screener/detail route와 OpenAPI
5. Web 후보/스크리너/상세 beta surface
6. fixture, tamper, auth, no-secret, Web unit/E2E, static/runtime QA
7. STATUS/runbook 갱신, immutable release, 직접 owner-path QA

## 배포 게이트

- focused/all-target Rust tests, Web type/lint/unit, OpenAPI/static checks가 통과한다.
- live capture 전 plan/preflight가 endpoint, exact 30 symbols, date range, request ceiling,
  secret mounts와 no-order surface를 출력 없이 검증한다.
- artifact verifier와 API가 hash/shape/owner gate를 각각 독립 재검증한다.
- 빠른 베타는 신규 SQL table이나 공용 candidate queue에 쓰지 않는다. snapshot 교체는
  provider-free 검증 뒤 atomic pointer로만 수행한다.
- production image는 순차 빌드하고 exact commit release로만 설치한다.
- 인증되지 않은 요청은 로그인 복구 경로로 가고, Member는 proprietary beta row를 받지
  못하며, Owner는 Top 5·screener·detail을 직접 확인한다.

## Coordinator gates

1. 시작 전: exact universe/hash, KIS allowlist, 비조정 가격, 최소 121 common-observation
   계약과 disjoint file ownership을 재확인한다.
2. Wave별: 작업자 보고를 그대로 채택하지 않고 diff·공개 API·focused test를 직접
   확인한다. 낮은 tier 결과는 기계적 검증이 통과해야만 다음 wave로 넘긴다.
3. 릴리스 전: 전체 Rust/Web/static 검증, secret 미노출, artifact 독립 verifier와
   clean commit을 확인한다.
4. 최종: exact commit 이미지를 순차 빌드해 installer가 완료될 때까지 기다리고,
   health·unauthenticated 로그인 복구·Member 403/비노출·Owner Top 5/screen/detail을
   브라우저와 API 양쪽에서 직접 QA한다.
