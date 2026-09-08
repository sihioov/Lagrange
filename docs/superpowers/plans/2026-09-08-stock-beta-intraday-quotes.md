Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Stock Beta 장중 현재가 반영 실행 계획

작성일: 2026-09-08 (Asia/Seoul)
상태: WP-2B·WP-5·WP-3A·WP-3B1·WP-3B2a 통합 완료; WP-3B2b 수집 루프 연결 진행; 운영 활성화 미실행
기준 커밋: `d1baf9da9b13fcb61649b1c26de56aed87a83418` (main 통합·원격 푸시 확인)

## Goal and boundaries

### 목표와 제품 범위

장중 선택한 종목의 현재가와 전일 대비를 Stock Beta Koyfin형 화면에 주기적으로
반영한다. 사용자의 “실시간” 요구는 앞서 제안한 **REST 현재가 polling 1차안**으로
계획한다. 매 체결을 받는 WebSocket 스트리밍이나 확정 종가 실시간 변경을 뜻하지 않는다.
UI에는 `장중 현재가 · 주기적 조회`로 표시하며 무지연 실시간이라고 표기하지 않는다.

1차 범위는 `/stock-beta`와 `/stock-beta/[instrument]`에서 **선택한 한 종목**의
현재가 위젯이다. 실제 owner의 승인된 V2 membership에 속한 종목만 허용한다.
ETF는 해당 화면의 승인 membership·동일 KRX 조회 계약을 만족하는 경우에만 포함한다.
기존 별도 ETF11 제품 전체, 관심종목 전체 일괄 polling, 종목 자동 등록은 포함하지 않는다.
선택 종목 우선이라는 범위와 아래 숫자는 제안 기본값이다. 전 종목 갱신이나 tick 수신이
필수라면 WP-1 전에 이 계획의 예산·스케줄·소스 승인 범위를 수정해야 한다.

### 측정 가능한 완료 조건

- 정상 장중, 하나의 활성 종목, 합성 provider 응답 1초 이내 조건에서 서버의 5초 목표
  수집 주기와 Web의 5초 cache 조회 주기가 동작한다. 합성 가격 변경은 최악 11초 이내
  화면에 나타난다. 실제 provider의 거래 지연이나 네트워크 지연에 대한 SLA는 아니다.
- 전일 대비 금액·퍼센트의 부호가 정확하고, 종목/시장/세션/기준가격이 다른 값을 섞지 않는다.
  현재가와 `마지막 확정 EOD 종가`, EOD 기준일, 신호 snapshot 기준일은 각각 구별된다.
- cache 조회·컴포넌트 재렌더·다중 탭이 provider 직접 호출을 유발하지 않는다.
  동일 종목 demand는 병합되고, 모든 실제 재시도도 서버의 공유 예산에 포함된다.
- 현재가 도착이 기존 일봉 OHLCV, SMA, 수익률, signal score/rank, snapshot/generation,
  Raw EOD evidence 또는 publication 상태를 변경하지 않는 회귀 증거가 있다.
- hidden/offline/unmount/logout/membership disable 시 Web polling과 demand 갱신이 멈춘다.
  만료된 demand, 장외, 휴장, calendar 불명, 토큰/예산/정책 오류 시 수집이 안전하게 중단된다.
- API 권한·DB 분리·schema 검증, 실제 Chromium 조작 QA, 부하·장애·다중 프로세스
  토큰/호출량 회귀 검사가 통과한다. UI 위젯의 추가·제거·재배치를 catalog에서 검증한다.

### 대상 workspace와 지침

- 계획 저장 위치: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes.md`.
- coordinator checkout: `/data/worktrees/3puw275b/enhanced-pig`.
- 실행 시 위 기준을 포함하는 최신 승인 커밋에서 전용 integration branch와 Paseo worker
  worktree를 생성한다. 기존 운영 checkout, 배포용 고정 snapshot, 다른 작업 branch를 수정하지 않는다.
- 아래 worker의 cwd는 executor가 생성한 **각 package의 repo-root 절대경로**다. 실행 전
  실제 경로·branch·base SHA를 각 brief에 삽입한다. 미해결 경로로 worker를 시작하지 않는다.
- 읽은 지침: 사용자 제공 repository/global AGENTS 지침, 저장소 `AGENTS.md`,
  `apps/web/AGENTS.md`, `apps/web/CLAUDE.md` (`@AGENTS.md`). 현재 checkout에는 root
  `CLAUDE.md`가 없으며 main checkout의 해당 파일도 `@AGENTS.md` 위임이다.
  각 worker는 실행 시 추가/변경된 계층 지침을 다시 확인한다.
- Web 코드를 쓰기 전 설치된 Next의 `node_modules/next/dist/docs/`에서 해당 API 가이드를 읽는다.
- provider/model: Codex. 아래 luna/terra/sol은 각각 `gpt-5.6-luna`, `gpt-5.6-terra`,
  `gpt-5.6-sol`을 뜻한다. 정해진 구현은 luna max, 독립 리뷰는 terra high, 열린 설계는 sol high.
  비용 이유의 luna 선택은 max를 사용한다. 모델/effort 가용성은 실행 시 확인하고 미지원이면
  `resolve at execution from current Paseo availability`; 모델명을 추측하거나 조용히 대체하지 않는다.
- 반복 실패 2회, loop, 맥락 누락 또는 검증 반복 실패 시 luna → terra → sol 순서로
  한 단계씩 재배정한다. 모델과 effort를 동시에 올리지 않는다. worker 자체 하위 위임 금지.
- 모든 WP는 `$paseo-delegate`로만 실행한다. 현재 계획 작성에는 worker를 띄우지 않는다.
  실행 시 이 스킬이 없으면 첫 worker 전에 중지하고 설치/가용성 전제를 보고한다.
- Mem0 scope resolver와 두 검색을 사용했으나 관련 결과가 없었다. 아래 근거는 소스와
  대화의 확정 결정이며 메모리에서 새로운 승인이나 운영 상태를 추론하지 않았다.

### 확인된 기술적 출발점

- `crates/kis-client/src/market_data.rs`: `KisMarketDataClient`의 정확한 read allowlist에
  `GET /uapi/domestic-stock/v1/quotations/inquire-price`, `FHKST01010100`이 존재한다.
  이 transport를 재사용하며 generic proxy나 새 ETF 전용 endpoint로 확장하지 않는다.
- `crates/kis-client/src/auth.rs:69`, `rate_limit.rs:74`: 토큰과 rate state는 각각
  프로세스 내부 mutex에 있다. 같은 App Key를 쓰는 별도 프로세스 사이의 공유를 보장하지 않는다.
  `Arc`를 하나 더 만드는 것으로 전역 호출 제한을 해결했다고 판정하지 않는다.
- `crates/job-queue/src/bin/owner-equity-v2-runner.rs`와
  `data-pipelines/collectors/src/worker.rs`가 credentialed read client를 구성한다.
  장기 worker, 일일/수동 one-shot, V2 runner의 실제 동시 실행 경로 확인이 선행되어야 한다.
- `crates/api-server/src/http/owner_equity_v2.rs`와 `deploy/compose/compose.yml`은
  API에 provider credential/Raw root를 주지 않는다. 이 경계를 유지한다.
- `apps/web/components/stock-beta/dashboard/widget-registry.ts`, detail registry,
  `shared/widget-types.ts`의 catalog 구조를 사용한다. EOD `chart-load-coordinator.ts`와
  `signal-refresh-coordinator.ts`에 장중 수집 로직을 덧붙이지 않는다.
- [ADR-0005](../../decisions/0005-kis-personal-use-entitlement.md)의 private single-owner
  권리 결정은 유지한다. 같은 범위의 권리를 재확인하도록 요구하지 않는다.
- 기존 운영 수집 복구와 새 릴리스 배포는 미완료 기록이 있다.
  [운영 준비 기록](2026-09-07-stock-beta-production-release-readiness.md),
  [수집 incident](../../runbooks/kis-daily-stale-release-20260907.md)를 읽고 실제 상태를
  별도 확인한다. main 푸시가 이 문제들을 해결한 것으로 가정하지 않는다.

### 명시적 제외와 승인 경계

- WebSocket, 분봉/tick history 저장, 장중 candle 생성·기존 일봉 덮어쓰기, 장중 신호 재산출 제외.
- 계좌·잔고·주문·실행내역·주문 WebSocket·Compose `live` profile은 전부 제외.
  기존 `crates/kis-client/src/websocket.rs`가 있다는 이유로 시세 연결에 재사용하지 않는다.
- KRX 이외 NXT/통합시장, 다른 가격 공급자/fallback, OpenDART/KIND 호출 제외.
- feature 개발은 fixture/provider-free가 기본이다. 계획은 장중 상시 호출, 새로운 response
  contract, 새 credential 경로 또는 운영 활성화를 자동 승인하지 않는다.
- WP-1은 기존 parser 대비 추가 필드를 명시한다. `AGENTS.md`가 요구하는 response contract
  변경 승인이 필요한 경우 정확한 필드/표본 계약/예산을 제시하고 승인 전 해당 분기를 중지한다.
- 테스트 토큰 sentinel 외 실제 key/token/body/provider prose는 로그·문서·Git·Web·quote DB에
  남기지 않는다. 토큰 persistence가 필요하면 보호된 runtime 전용 저장소만 검토한다.
- 실제 배포·sudo grant 변경·수집기 재시작·DB migration 실행·Funnel 변경·main merge/push는
  이 계획 실행의 자동 단계가 아니다. 별도 release 승인 gate로 남긴다.

## 설계 계약 및 제안 기본값

### 데이터 흐름과 격리

Web selection demand → owner API의 제한된 demand 저장 → credentialed worker의
스케줄러/공유 KIS 호출 경계 → 검증된 단기 quote cache → owner API GET → 현재가 위젯.

API GET은 cache-only이며 provider 호출/토큰 발급/새 demand 생성이 없다.
demand 생성·연장은 별도 CSRF 보호 mutation으로 표현한다. 각 소비자의 bounded lease는
별도로 관리하고 같은 종목의 수집 수요만 병합한다. 한 탭의 해제로 다른 탭의 유효 lease를
삭제하지 않는다. 무제한 lease/queue/job 생성은 금지하며 최대 lease 수도 WP-1에서 고정한다.
DB 접근은 기존 actor transaction/RLS 및 최소 권한을 따른다. API는 quote를 쓰지 않고,
worker는 허가된 active membership만 읽고 quote를 쓴다. Web에는 DB/worker 인증이 없다.

기본 방향은 기존 V2 credentialed runner에 독립적인 quote loop를 추가하고,
기존 EOD 작업과 같은 read boundary를 사용하는 것이다. 큐의 15분 작업이 quote loop를
무조건 막거나, quote가 EOD 재시도를 고갈시키지 않도록 실제 실행 모델을 WP-1에서 확정한다.
새 서비스/Redis/외부 broker는 기본안이 아니다. 반드시 필요하면 coordinator가 소유 범위와
운영 비용을 다시 승인한 뒤 graph를 수정한다.

### 조회 예산과 여러 탭

| 항목 | 제안 기본값 및 의미 |
| --- | --- |
| Web cache polling | visible/online에서 5초, 이전 요청 완료 후 다음 예약; 중첩 요청 금지 |
| demand lease | 30초; 별도 mutation을 15초마다 갱신, 화면 종료 시 best-effort 해제 |
| intraday provider slot | App Key 공유 범위에서 최소 5초 간격, 동시에 한 요청만 실행 |
| 중복 제거 | owner/instrument/venue/session으로 병합; 같은 종목 탭 수로 호출 증가 금지 |
| 활성 종목 한도 | 단일 owner의 서로 다른 탭 합계 최대 5개; 초과는 명시적 capacity 응답 |
| 여러 종목 fairness | round-robin; 5개면 종목별 약 25초 목표, 단일 종목 5초 목표; 보장 SLA 아님 |
| 기존 KIS 상한 | 해당 credential의 endpoint/TR 채널별 총 1 request/sec 이하, 기본 순차 |
| 장중 일일 상한 | credential별 5,000 GET attempts 제안; 실패/재시도 포함, 도달 시 다음 날까지 중단 |
| request deadline/retry | WP-1에서 기존 transport와 양립하는 deadline 확정; 최대 2회 재시도, 모든 시도에 slot 적용 |
| throttling | Retry-After 우선, bounded backoff; 한 화면 새로고침으로 cooldown 우회 불가 |

하루 요청 수는 “승인된 세션 길이 ÷ slot 간격”과 retries로 계산하여 WP-1에 기록한다.
특수 개장일·세션 연장에도 daily hard cap을 초과하지 않는다. 이 숫자는 polling 예산 제안이지
증권사의 공식 quota 숫자가 아니다. 기존 EOD/token 정책이 우선이며 부하 시 현재가가 느려졌음을
표시한다. 모든 탭이 닫히면 lease 만료 후 새 provider 요청은 0이어야 한다.

공유 경계는 token 재사용·발급 직렬화·최소 발급 간격·재시작 및 credential 회전, 채널별
quota와 cooldown을 **같은 credential을 쓰는 모든 활성 프로세스**에 적용해야 한다.
token secret을 담은 일반 DB cache나 단순 무잠금 파일은 금지한다. 안전한 공유가 불가능하면
새로 각자 발급하지 않고 장중 기능을 disabled로 유지한다. 기존 EOD를 임의 중지하지 않는다.

### API와 quote 의미

WP-1이 고정할 proposed application routes (KIS endpoint 변경을 뜻하지 않음):

- `POST /api/v2/owner-equity/quote-demands`: instrument/membership generation과 bounded
  lease 식별자만 받는다. arbitrary provider URL/TR/account/owner ID override는 받지 않는다.
- `DELETE /api/v2/owner-equity/quote-demands/{demand_id}`: 해당 owner demand 해제.
- `GET /api/v2/owner-equity/instruments/{instrument}/quote`: 인증·권한 확인 후 cache-only 응답.

기존 V2 prefix와 route conventions를 WP-1에서 대조해 literal path를 확정한다.
path가 달라지면 WP-1 산출물에서 표를 교체한 뒤 WP-3/4/5를 시작하며 각자가 추측하지 않는다.

최소 DTO: schema version, instrument ID, venue=KRX, currency=KRW, membership generation,
검증된 session identity, price, previous-close 기준 값/일자(입증 가능한 경우), change/percent,
수집 시각 `received_at`, provider timestamp(문서에 있고 검증될 때만), sequence/quote version,
next-poll hint, transport freshness, market/session status, typed failure reason.

- KIS 현재가 field 의미와 부호 code, 거래정지/무거래/0·빈 값 처리, ETF 동일 계약,
  identity/date 검증은 공식 문서+fixture로 고정한다. 금액은 decimal-safe 계약을 사용한다.
- `received_at`은 서버가 응답을 받은 시각이지 마지막 체결 시각이 아니다. 제공되지 않는
  trade timestamp를 만들지 않는다. 최근 조회에 성공해도 “방금 체결”이라고 표시하지 않는다.
- 이전 거래일 종가를 옛 EOD snapshot의 마지막 값으로 대신하지 않는다. provider 기준일이
  입증되지 않으면 해당 메타데이터를 unavailable로 두고 EOD 기준일과 합치지 않는다.
- freshness(RECENT/STALE/UNAVAILABLE)와 시장 상태(OPEN/CLOSED/HALTED/UNKNOWN)를
  분리한다. OPEN은 KST 시계만으로 판정하지 않는다. 수집 calendar가 없거나 오래됐으면
  UNKNOWN으로 fail-closed. `chk-holiday`는 승인된 daily 결과를 재사용하며 polling하지 않는다.
- 최근 cache의 transport age 30초 초과 시 STALE. 서버 last-success 시간을 유지하고
  실패마다 received_at을 갱신하지 않는다. 재시작 후 복원 cache도 현재로 둔갑시키지 않는다.
- quote payload가 identity/schema/integrity 검증에 실패하면 새 가격은 표시하지 않는다.
  last-good 유지 허용 시 동일 identity+세션에만, stale/실패 이유·시각을 함께 표시한다.
- 장 마감은 현재가를 확정 종가로 승격하는 이벤트가 아니다. 다음 일자의 확정 종가는
  기존 EOD pipeline의 검증·publication을 통해서만 반영한다.
- Cache는 short-lived latest record이며 EOD Raw/Curated와 분리한다. 기본 history 미저장,
  quote row 최대 24시간 retention 및 membership disable 시 읽기 차단. 정확한 GC와 restart
  규칙은 WP-1에서 확정하고 운영 데이터 삭제를 이번 계획으로 실행하지 않는다.
- API response는 `Cache-Control: no-store`; 초대 Member/비로그인/다른 owner는 가격과
  demand 존재 여부를 볼 수 없다. logout/401/403 때 화면의 잔존 quote를 제거한다.

### UI/UX와 확장성

`CurrentQuoteWidget`은 dashboard/detail에서 같은 view model과 컴포넌트를 사용한다.
서버 I/O와 timer는 `quote-load-coordinator`/전용 hook, UI는 상태를 렌더하는 컴포넌트로 분리한다.
기존 workspace는 연결만 담당하며 거대 파일로 수집 상태 기계를 합치지 않는다.
widget 제거·숨김으로 소비자가 없어지면 polling/demand가 정리되어야 한다.

- 현재가, 전일 대비, `주기적 조회`, 마지막 수신 시각 및 지연/장외/정지 상태 표시.
- EOD 차트 옆 독립 quote 카드가 1차안이다. 일봉 y축/SMA/tooltip 배열이나 마지막 candle을
  quote로 변경하지 않는다. 차트 overlay/분봉은 후속으로 남긴다.
- 선택 A→B, membership generation 변경, disable/re-add, snapshot 변경 시 quote identity를
  재검증한다. AbortController와 단조 증가 request token으로 늦은 A 응답을 거부한다.
- 기존 메뉴/한영/접근성 유지. 색상만으로 상승·하락을 구분하지 않고 숫자 부호/텍스트를 함께 표시.
  매 tick마다 screen-reader announcement를 강제하지 않으며 상태 전환만 적절히 알린다.
- 375×800, 640×360, 768×1024, 1280×720, 1440×900에서 가로 overflow 0,
  quote/종목명/시각/상태/조작 버튼 접근 가능. 44px touch target, keyboard, forced colors,
  reduced motion과 200% zoom-equivalent를 검사하고 실제 zoom과 혼동하지 않는다.

## Initial classification

분류를 먼저 정한 뒤 아래 graph에서 worker를 배정했다. 아직 해소되지 않은 필수 계약은
복잡도라는 이유로 worker에게 떠넘기지 않고 WP-1 및 coordinator gate에서 고정한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | hard | source contract·시간 의미·공유 credential 운영 경계의 설계 판단 | high | 문서 충돌, 기존 read 프로세스 누락, 승인되지 않은 응답/시장/권한 필요 시 영향 분기 중지 |
| WP-2A | hard | 프로세스 간 token/rate/cooldown primitive의 crash·restart 실패 비용 | high | durable reservation/rotation race 증명 실패 시 설계 재검토 |
| WP-2B | hard | 여러 기존 read caller와 default-off 호환 경계 연결 | medium | baseline 생성자 호환과 fail-closed opt-in 충돌 시 coordinator가 정책 고정 후 launch |
| WP-3 | intermediate | 동결 계약의 demand/cache/producer 구현과 DB fencing | medium | 별도 서비스 필요, 기존 job lease 침범, DB 권한 모델 변경 확대 시 hard로 재분류 |
| WP-4 | intermediate | 기존 owner API·RLS·OpenAPI 패턴으로 제한된 endpoint 추가 | high | actor isolation 증명 불가, API에 secret/egress 필요 주장 시 구현 중지 |
| WP-5 | intermediate | catalog 위젯과 독립 polling 상태 기계, 결정적 UI 검증 | high | catalog 수정이 공통 엔진 재설계로 확장되거나 race 수정 2회 실패 시 상향 |
| WP-6 | intermediate | 확정 구조의 opt-in runtime wiring/검사/다이어그램 동기화 | medium | 새 DB login/image 서비스·root grant 필요 시 변경 범위 재승인 |
| WP-7 | hard | cross-layer 안전성과 승인 경계에 대한 독립 수락 판단 | high | critical/high/medium 발견 시 담당 반환; 자체 구현·범위 확대 금지 |
| WP-8 | intermediate | 동결 acceptance의 fixture/DB/browser·부하 회귀 실행 | high | 재현 불가 race, 결과 누락/반복 flaky이면 중지 후 harness 또는 코드 결함 분리 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | hard | 정확한 소스·API·스케줄·공유경계 계약 동결 | 아래 신규 contract spec만 | 없음 | Codex sol high | 계약/승인 delta/정확한 파일 map | 공식 근거, caller 목록, budget 계산, coordinator 승인 |
| WP-2A | 2 | hard | 비연결 shared read primitive 구현 | 신규 read_coordination 및 module/dependency/전용 tests | WP-1 제한 수락 | Codex sol high | 보호 state·durable budget/token API | fake transport/clock·OS process barriers |
| WP-2B | 3 | hard | 검증 primitive를 기존 read caller에 연결 | 기존 WP-2 나머지 scope, WP-2A와 순차 | WP-2A 및 compatibility gate | Codex sol high | explicit mode·caller wiring | constructor/기존 read 회귀·disabled 호환 |
| WP-3 | 4 | intermediate | demand/cache DB와 quote producer | 신규 migration, market-data/collector/job-queue quote 모듈 | WP-2B·response gate | Codex luna max | producer·fencing·role grant | disposable DB, scheduler fake-time, read boundary regression |
| WP-5 | 4 | intermediate | 현재가 위젯·Web client·polling | apps/web의 아래 지정 source/unit tests | WP-1·WP-2B gate, 동결 fixture contract | Codex luna max | dashboard/detail quote UX | unit·typecheck·lint, mock-only race tests |
| WP-4 | 5 | intermediate | owner cache API·OpenAPI | API Rust/contract/generated OpenAPI | WP-3 | Codex luna max | 인증·demand mutation·cache GET | DB-backed HTTP, OpenAPI check, no-provider assertions |
| WP-6 | 6 | intermediate | opt-in 배포 계약·runbook·diagram | Compose/ops/docs 및 필요 CI 접속점 | WP-3·WP-4·WP-5 | Codex luna max | default-off 설정과 검사/로컬 PNG | static/self-test, manifest compatibility, local renderer |
| WP-7 | 7 | hard | 독립 전체 변경 리뷰 | source read-only, review report | WP-6 | Codex terra high | severity별 ACCEPT/REJECT | source/권한/계약/데이터 독립성 증거 |
| WP-8 | 8 | intermediate | 실제 기능·회귀 QA와 증거 | 지정 E2E/fixtures/QA report | WP-7 ACCEPT | Codex luna max | 통합 QA matrix | production browser 2회·DB·부하·전체 회귀 |

Wave 4의 WP-3과 WP-5만 병렬이다. 다른 wave는 순서대로 통합한다. WP-2B의 공용 caller
수정이 끝나기 전에 WP-3은 시작하지 않는다. WP-4와 WP-5는 Rust/OpenAPI 대 Web으로
파일이 분리되지만 API는 DB 후 실행한다. QA는 모든 code integration 후 한 worker만 실행한다.

## Worker briefs

### 모든 worker에 복사할 공통 brief

- cwd: executor가 package에 배정한 Paseo worktree repo-root 절대경로와 확인된 base SHA.
  필요한 이전 결과는 경로만 가리키지 말고 동결 계약·결정·제약 요약을 prompt에 포함한다.
- 본 문서의 해당 scope, 목표, 예산, 금지 경계와 단계별 승인을 지킨다. 다른 source를
  읽는 것은 가능하나 owned scope 밖은 수정하지 않는다. 미지정 파일이 필요하면 coordinator에
  정확한 파일/이유를 보고하고 scope amendment 전 수정하지 않는다. 추측·재위임 금지.
- 필수 보고: (1) 변경 파일/라인 범위 (2) brief와 다른 처리 및 이유 (3) 실제 명령/exit/
  count/elapsed (4) unresolved/후속 항목 (5) 찾지 못했거나 검증 못한 항목.
  빈 항목도 `none`을 명시한다. “실행 시작”/생략된 terminal summary를 PASS로 세지 않는다.
- dependency 설치, DB/Docker/Next/browser 시작은 자신의 단계에서 명시한 범위와 실행환경
  권한 확인 뒤만 수행한다. 실제 KIS·계좌·주문·production 서비스 접근은 전 WP에서 금지한다.
- owned package commit과 clean 상태를 보고하되 main merge/push나 운영 릴리스 변경은 하지 않는다.

### WP-1 — 계약 동결과 승인 delta

- cwd: WP-1 Paseo worktree. hard/high confidence, 열린 cross-system 설계이므로 sol high.
- 입력: baseline source, 본 문서, AGENTS, ADR-0005, 운영 incident, 공식 KIS 자료.
  공식 자료만 research하고 broker endpoint 자체를 호출하지 않는다.
- owned write: `docs/superpowers/specs/2026-09-08-stock-beta-intraday-quotes-contract.md` 신규.
  다른 source/AGENTS/기존 ADR 수정 금지. 필요한 승인 변경은 spec에 proposal로 기록한다.
- 산출물: 실제 read caller 전체의 file:line 목록, 동일 credential 판별/공유 저장소 권한,
  token restart/rotation 방안, 모든 limiter 우회 경로의 차단 방식, producer 실행 위치,
  API literal DTO/path/CSRF/lease/capacity/failure code, schema 숫자·시각·부호 계약,
  market calendar/특수일/정지·무거래 정책, 정확한 migration/role/helper/source 파일 소유 map.
- 공식 [현재가 예제](https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/inquire_price/inquire_price.py),
  [필드 매핑](https://github.com/koreainvestment/open-trading-api/blob/main/examples_llm/domestic_stock/inquire_price/chk_inquire_price.py),
  KIS portal 및 제공 XLSX를 현재 상태로 재확인하고 조회일·가능하면 revision pin을 기록한다.
  현재가는 REST이고 공식 예제도 streaming에는 WebSocket을 안내한다. sample 전체 실행 금지.
- 최소 fixture matrix와 예산 산식을 제출한다. response contract 변화/운영 high-frequency
  승인 필요 여부를 명시한다. 중요한 문서 충돌이나 source freshness 증명 불가를 숨기지 않는다.
- 검증: 소스 근거 및 공식 근거 직접 대조, 모든 proposed owned file·dependency에 중복 없음,
  baseline 대비 승인 delta 목록. coordinator가 승인 전 downstream launch 금지.

### WP-2 — read credential arbitration

아래 원래 owned scope는 WP-2A → WP-2B 두 순차 package의 합집합이다. primitive와
runtime 연결을 분리하여 source-response 승인 및 default-off 호환 결정을 기다리느라
외부 호출 없는 구현까지 멈추지 않는다. 두 package에도 공통 필수 보고 형식을 적용한다.

#### WP-2A — 비연결 primitive (현재 launch 허용)

- cwd: executor의 별도 WP-2A Paseo worktree. hard/high confidence, Codex sol high.
- owned: 신규 `crates/kis-client/src/read_coordination.rs`, 필요 그 하위 모듈,
  `crates/kis-client/src/lib.rs` export, `crates/kis-client/Cargo.toml`, `Cargo.lock`,
  신규 `crates/kis-client/tests/read_coordination*.rs`와 전용 fixture helper만.
- 기존 auth/market_data/limiter/transport/caller의 실행 동작과 constructor는 변경하지 않는다.
  실제 secret/path/env/network를 읽지 않고 명시적으로 제공한 fake credential·temp root·clock·
  issuer/read callback으로 shared protocol을 검증한다. 아직 production 보호가 연결됐다고 보고하지 않는다.
- WP-1 sections 5–6의 file trust/atomic state/lock/credential generation/HMAC/token reuse/
  rate/cooldown/quote budget을 primitive로 구현한다. 파일 descriptor를 보유한 검증으로
  symlink/hardlink 및 check/open 경합을 방지한다. fake root의 uid는 현재 사용자로 명시한다.
- crash 예약은 마지막 시작시각만 저장해서는 안 된다. 시작 전 durable in-flight deadline과
  token issue attempt를 기록하고, 새 프로세스가 죽은 요청의 유효 deadline 이전에 재진입하지
  못하게 한다. 정상 완료만 fence에 맞춰 예약을 해제한다. ambiguous/cancel 시 보수적으로 보존한다.
  broker 내부 처리 종료를 완벽히 증명한다는 주장은 하지 않는다.
- token 획득/재사용 자체는 GET quota를 소비하지 않는다. 최종 GET 전 시각으로 read 간격과
  예약을 다시 검사한다. 실패를 typed error로 반환하고 secret을 Debug/error에 포함하지 않는다.
- 검증: offline/locked Cargo, 기존 kis-client 회귀와 두 OS 프로세스 fixture. 한 번에 한
  compiler invocation, `CARGO_BUILD_JOBS=2`; 실제 HTTP/DB/Docker/provider는 0.
- 결과는 primitive+테스트/commit/scope diff. 파일 추가나 설계 미해결은 추측하지 않고 반환한다.

#### WP-2B — 기존 caller 연결 (WP-2A 검증 이후)

- cwd: WP-2A를 통합한 별도 worktree. hard/medium confidence, Codex sol high.
- owned: 아래 원래 WP-2의 나머지 auth/market_data/retry/error 및 정확한 read caller wiring.
  primitive 보완은 순차 소유이며 필요한 경우 coordinator가 diff scope를 명시한다.
- launch 전 coordinator는 기존 설정의 feature-off 호환성과 신규 coordinated-required
  mode의 fail-closed 동작을 고정한다. “모든 LiveTransport 생성자를 즉시 금지”와
  “기존 default-off 릴리스가 그대로 동작”을 동시에 충족했다고 가정하지 않는다.
- 응답 parser 필드 확대나 실제 polling 활성화 없이 read 경계만 연결·검증한다.
  WP-2A 단독 성공은 이 package 또는 shared-boundary gate 완료가 아니다.

- cwd: WP-2 worktree. hard/high confidence; sol high. 입력은 동결 WP-1과 baseline.
- owned: `crates/kis-client/src/{auth,rate_limit,market_data,token_issuer}.rs`, 필요 신규
  read-coordination 모듈과 `lib.rs` 등록 및 focused tests; WP-1에서 지정한 credentialed read
  생성 지점 (`data-pipelines/collectors/src/worker.rs`, V2 runner bin, 필요한 one-shot의
  **생성 연결 부분만**). Cargo dependency 변경은 사전 file map에 있을 때만.
- 기존 TokenManager/allowlist를 유지하며 process-shared 정책으로 보강한다. read 전용 opt-in
  보호가 live-order client의 실행 의미를 바꾸거나 계좌 경로에 접근해서는 안 된다.
- 정상/만료/401/429/timeout/crash/restart/clock skew/credential rotation에서 token 발급,
  cooldown, channel budget을 검증한다. 위장 credential alias로 동일 key budget이 분리되는
  경로를 막는다. 실제 key/hash값을 진단에 노출하지 않는다.
- marker만 공유하고 secret lifecycle은 각 프로세스에서 재발급하는 가짜 공유를 금지한다.
  공용 보호 저장소가 없거나 안전하지 않으면 typed disabled/error, unguarded fallback 0.
- 검증: `CARGO_BUILD_JOBS=2 cargo test --locked -p kis-client`와 동결 scoped tests.
  두 실제 OS 프로세스+fake transport의 barrier로 동시 만료/죽은 leader/재시작을 검사한다.
  provider 호출은 0. live 통합 test가 있으면 실행 제외 목록을 명시하고 대체 fixture 근거를 제출한다.

### WP-3 — quote producer와 저장 모델

- cwd: WP-3 worktree. intermediate/medium confidence; luna max. WP-1 계약과 WP-2 연결 완료 필요.
- owned: 다음 빈 번호의 `migrations/*_owner_intraday_quotes.{up,down}.sql` (번호는 launch 때 고정),
  `crates/market-data/src/intraday_quotes.rs`, `data-pipelines/collectors/src/intraday_quotes.rs`,
  `crates/job-queue/src/owner_equity_v2/intraday.rs` 신규 및 필요한 module 등록,
  기존 V2 runner의 quote loop wiring, 해당 quote/scheduler/DB 테스트.
  bootstrap role 파일이 필요하면 WP-1 map에서 지정; 임의 새 login/서비스 생성 금지.
- 스케줄러는 quote provider slot/일일 예산/세션/demand cap을 적용한다. 동일 identity merge,
  공정성, no-demand stop, EOD contention, deadline/backoff를 fake-time으로 증명한다.
- quote 상태는 독립 테이블 latest cache. 소유자·generation·session·monotonic version/fencing,
  transaction 경계와 worker 재시작을 처리한다. disable 중 in-flight 응답은 publication 불가.
- 기존 Raw/EOD/신호 테이블을 수정하는 경로는 0. 시장·가격 parser 추가가 기존 reference
  normalizer response 의미를 넓히지 않는다. raw body는 cache나 operational log에 저장하지 않는다.
- 검증: market-data/collectors/job-queue focused unit 및 disposable QA PostgreSQL role/RLS,
  grants/up-down safety/lease-expiry/concurrent-write 테스트. DB 없으면 필수 acceptance 실패로
  보고하며 조용히 skip 후 성공시키지 않는다. 운영 DB는 사용하지 않는다.

### WP-4 — owner API와 계약

- cwd: WP-4 worktree. intermediate/high confidence; luna max. WP-3 DB와 WP-1 route spec 입력.
- owned: `crates/api-server/src/http/owner_intraday_quotes.rs`,
  `crates/api-server/src/repos/owner_intraday_quotes.rs` 및 필요한 mod/state/runtime/contract 접속점,
  `apps/api-server/scripts/openapi-spec.mjs`, `openapi.json`, `generated/openapi.ts`,
  신규 `crates/api-server/tests/http_owner_intraday_quotes.rs`와 scoped OpenAPI tests.
- API는 DTO whitelist/owner-only/current membership 검증, bounded demand mutation/CSRF,
  cache-only GET을 구현한다. 외부 URL/query 임의 전달 금지, job 폭증/lease 과장/cap 우회 방지.
- GET 무한 반복이나 비인가/잘못된 요청이 provider 호출·토큰 발급·demand write를 유발하지
  않음을 테스트한다. DB 역할과 expiry/sequence/integrity 계약이 Web과 동일해야 한다.
- 검증: real disposable DB HTTP acceptance, actor/Member/expired session/CSRF 및
  unknown/disabled/generation mismatch, `cargo test --locked -p api-server --test openapi_contract`,
  `npm run openapi:check --workspace @lagrange/api-server`. DB 생략 PASS 금지.

### WP-5 — quote 위젯과 polling 상태 기계

- cwd: WP-5 worktree. intermediate/high confidence; luna max. WP-1의 동결 DTO fixture로 개발하며
  WP-3과 mutable scope가 겹치지 않는다. 실제 endpoint 연동은 WP-4 통합 gate에서 확인한다.
- owned: `apps/web/lib/products/intraday-quotes-{contracts,client}.ts` 신규,
  `components/stock-beta/quote/` 신규(coordinator/hook/view/widget/CSS), dashboard/detail registry와
  view-model types의 최소 접속점, workspace/detail composition, stock-beta locale dictionary,
  필요 `apps/web/lib/api/contracts.ts` mutation path 등록, 신규 `tests/stock-beta-intraday-*.test.*`.
  기존 EOD chart renderer·geometry·signal 계산과 E2E 파일은 수정하지 않는다.
- 재사용 가능한 widget을 catalog에 등록하고 add/remove/reorder/hide에 따른 lifecycle을 검사한다.
  숫자 가격만 전체 페이지 rerender하거나 signal refresh를 호출하는 연결 금지.
- fake clock으로 polling5초/demand15초/TTL30초, 숨김·offline·unmount·logout cleanup,
  선택 race·generation·영문/한글·부분 실패·마지막 성공시각 및 상태 표시를 검증한다.
- 검증: scoped/full Vitest, `npm run typecheck --workspace @lagrange/web`, Web lint.
  필요 시 `--configLoader runner`를 사용하고 기존 shared dependencies는 재설치/삭제하지 않는다.
  이 wave에서 Next/browser는 띄우지 않는다. UI runtime 검증은 WP-8이 단독 수행한다.

### WP-6 — default-off 운영 계약과 구조 문서

- cwd: WP-6 worktree. intermediate/medium confidence; luna max. WP-2~5 통합 source 입력.
- owned: `deploy/compose/compose.yml`과 WP-1에서 명시한 env example/schema/ops validator,
  V2 runtime static/self-test, credential-coordination 안전 검사, 필요한 CI test 접속점,
  신규 `docs/runbooks/stock-beta-intraday-quotes.md`,
  `docs/diagrams/{component_architecture,runtime_deployment}.{puml,png}`.
- 기능 기본 OFF; 잘못된/누락된 quota/calendar/권한 설정은 시작 실패 또는 typed disabled.
  기존 릴리스 설정에서 안전하게 비활성 호환되어야 한다. API/Web에 credential/Raw 쓰기 권한
  추가 금지. 새 token-coordination mount는 credentialed read worker에만 최소 권한으로 연결한다.
- 기존 image set을 재사용하는 기본안을 지킨다. 새 image가 필요한 판단은 coordinator로 반환한다.
  현행 pinned manifest/static checks를 우회하거나 검사 전체를 삭제하지 않는다.
- runbook에는 활성화 승인/예산/health/backout/disable/EOD 복구 전제/기능과 배포 QA 차이를 적는다.
  privileged 인증·bootstrap·72h grant를 새로 만들거나 frozen 운영 파일을 변경하지 않는다.
- 구조 edge마다 실제 file:line을 쓰고 두 PNG를 로컬에서 재렌더한다:
  `docker run --rm -v "$PWD/docs/diagrams:/data" plantuml/plantuml -tpng /data/component_architecture.puml /data/runtime_deployment.puml`.
  Docker 권한이 없으면 미검증을 보고하며 원격 renderer로 보내지 않는다.
- 검증: 수정 validator와 provider-free static/self-tests, default-off compose 계약,
  기존 7개 release preflight 회귀. 실제 image build/install/rollout은 하지 않는다.

### WP-7 — 독립 설계·구현 리뷰

- cwd: reviewed integration을 고정한 별도 Paseo worktree. hard/high confidence; terra high.
- owned write: `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes-review.md`만.
  source는 read-only; WP-1~6 구현자가 아닌 독립 worker를 선택한다.
- source/API/DB/runtime/UI 전체 diff에서 권한, token/rate 우회, calendar/시각 정직성,
  stale-data 표시, 원래 EOD와의 독립성, registry 확장성, 통합 race를 검토한다.
- Critical/High/Medium별 file:line·재현·영향·담당을 적고 미해결 blocker가 있으면 REJECT.
  직접 수정하지 않고 담당 package scope로 반환한다. WP-8 이후 변경분/QA 증거 재검토도
  동일 package의 후속으로 수행한다. QA 전 static ACCEPT를 최종 release ACCEPT로 부르지 않는다.

### WP-8 — 기능 QA와 통합 acceptance

- cwd: WP-7 통과 integration의 QA worktree. intermediate/high confidence; luna max.
- owned: 신규 `apps/web/tests/e2e/stock-beta-intraday.spec.ts`, 관련 synthetic fixture 신규 파일,
  `tests/e2e/support/synthetic-api.mjs`의 최소 routing/시나리오 연결,
  신규 `docs/superpowers/plans/2026-09-08-stock-beta-intraday-quotes-qa.md`.
  기존 E2E 하네스 접속점 수정이 필요하면 exact map 승인 후 진행; production source 수정 금지.
- broker 대신 결정적 fake transport를 사용한다. HTTP polling provider mock와 browser용
  synthetic app API를 구분하고, 실제 broker/계좌/주문/외부 네트워크 요청은 0으로 검사한다.
- 필수 runtime: 종목 선택/전환/등록 후 READY/disable/재등록, 값·부호·시각 변화,
  A→B 늦은 응답, multi-tab dedup/cap/fairness, visibility/offline/reconnect/logout,
  429/timeout/503/invalid data/unknown calendar/휴장/마감/정지/0가격/날짜 변경,
  오래된 EOD 차트 옆 최신 quote와 정상 EOD 불변성을 실제 Chromium에서 조작한다.
- timeout 뒤 fake response 도착, stale cache의 재시작, 미래 timestamp/역행 clock,
  worker lease 만료/disable in-flight 및 예산 소진을 unit+DB+integration 층에서 검사한다.
- 동일 안전 API를 1/10/100회 polling해도 cache-only GET이 provider call을 늘리지 않는지
  fake-time count로 증명한다. 30분 상당 scheduler 시간을 압축해 request/메모리/demand/cache
  bound를 검사한다. idle 체류와 UI hidden 상태에서 API/request counter를 직접 확인한다.
- 실사용 flow의 가격 숫자/시각/상태를 screenshot과 semantic assertions로 남긴다.
  수동/자동 클릭, 합성 데이터/실제 provider, zoom-equivalent/실제 zoom을 구분해 기록한다.
- final typecheck/lint/OpenAPI/full Vitest, Rust focused tests/clippy, disposable DB acceptance를
  수행한다. fresh production build 두 개로 focused Playwright 두 회 통과를 요구한다.
  Next `output: standalone`이면 standalone server 사용, 각 API URL을 build 시 정확히 고정한다.
  page와 실제 JS asset HTTP 200 확인, 한 worker, 각 회 완전한 terminal exit/count 확보.
- 전체 Web E2E도 수행한다. 기존 synthetic account/live UI fixture 외 실제 forbidden traffic은 0.
  중간 test 시작만 보인 실행은 PASS가 아니며 누락 결과를 timeout 근거로 면제하지 않는다.
- 빌드/테스트 하나씩 진행하고 14 GiB host 자원/OOM 정책을 지킨다. QA background session과
  정해진 새 loopback ports만 사용, 정확한 PID 정리, 사용자 프로세스/포트/기존 산출물 삭제 금지.
- 실제 KIS 장중 smoke와 외부 Tailscale 접속은 이 fixture QA의 범위 밖이다. 필요 시 별도 승인
  release gate에서 본인 인증 owner session과 8443 경로로 확인하며 공개 demo에 실데이터를 넣지 않는다.

## Coordinator gates

1. **Pre-launch:** `$paseo-delegate` 가용성, 모든 지침·baseline·clean state·현재 날짜 확인.
   기존 worker/build와 충돌하지 않는 worktree/자원 확보. 선택 종목 polling 범위를 유지하며
   live 호출은 승인하지 않은 상태로 WP-1 문서 작업만 시작한다.
2. **Contract gate:** WP-1의 정확한 route/DTO/session proof/동시 read caller map/공유 token
   모델·예산·파일 소유권을 coordinator가 직접 수락한다. response contract 변경 또는
   계획 범위 밖 서비스/시장/권한이 필요하면 그 분기를 중지하고 사용자 결정을 요청한다.
3. **Shared-boundary gate:** WP-2의 실제 다중 프로세스 fixture 증거 확인. credential/limiter
   우회가 남거나 토큰 재시작 정책이 미확정이면 producer 구현·운영 활성화를 허용하지 않는다.
4. **Per-wave integration:** WP-3/5 별도 commit 통합, exact owned scope와 기계적 검증 확인.
   WP-4에서 실제 DB/API와 Web schema 대조. 불일치는 계약 변경 승인 후 관련 package를
   순차 수정한다. reviewer가 구현자를 대신해 고치거나 worker가 임의로 graph를 재작성하지 않는다.
5. **Static acceptance:** WP-6 default-off/diagram/ops 검증 후 WP-7 독립 리뷰. unresolved
   Critical/High/Medium은 담당에게 반환하고 재검증 후에만 QA로 진행한다.
6. **Final end-to-end:** WP-8에서 두 fresh production focused pass와 전체 Web regression,
   DB/cross-process/rate tests의 exit/count/기간을 확보한다. QA 수정 뒤 WP-7 후속 리뷰가
   ACCEPT해야 개발 완료다. 새로운 source 수정이 있었다면 영향받는 runtime 검사를 다시 수행한다.
7. **Production activation (별도, 이번 실행 제외):** 최신화 incident/기존 research health,
   설치된 V2 릴리스/DB·calendar freshness·백업 및 rollback·entitlement pins·shared read worker
   rollout을 확인한다. 기존 72h image-only grant를 재사용해 migration/restart/live polling하지 않는다.
   사용자 승인된 종목·시간·요청 예산으로 제한된 장중 smoke를 먼저 수행하고, 실제 값과 수신 시각,
   provider 요청 수/오류, EOD 무영향을 확인한 뒤에만 상시 활성화를 검토한다.
   권한이 없으면 정확한 blocker를 보고하며 새로운 sudo 체계를 자동 제작하지 않는다.

## 계획 변경 규칙

아직 미확정인 소스·시장 세션 계약과 credential 공유 방식은 WP-1 산출물 및 gate에서
고정할 대상이다. 현재 문서가 그 사실을 검증 완료했다고 뜻하지 않는다.
이후 실행에서 scope/승인/dependency가 어긋나면 해당 branch를 멈추고 본 graph와 brief를
수정한 뒤 `$paseo-delegate`로 재개한다. native subagent로 우회하지 않는다.

## 실행 기록 — 2026-09-08

- 계획 커밋: `b5f32abf833f86de5dc46fc0ba7fd88923fcbaf8`.
- 전용 integration branch: `feature/stock-beta-intraday-quotes-20260908`.
- WP-1은 별도 Paseo worktree의 `work/stock-beta-intraday-contract-20260908`에서
  Codex `gpt-5.6-sol`, high로 시작했다. scope는 신규 contract spec 한 파일뿐이다.
- 현재 모델 가용성과 지침을 확인했다. provider 시작·결과 수락은 단순 agent ID 생성과
  구분하며, WP-1 계약 검토/승인 gate 전 WP-2 이후 작업을 시작하지 않는다.
- main 변경·push, 실제 시장 데이터 호출 및 운영 변경은 수행하지 않았다.

### WP-1 검토와 다음 단계

- WP-1 `3a3f528`을 검토하고 integration에 문서만 통합했다. 문서 완료와 모든 내용의
  production 승인은 구별한다. 정확한 기존 API prefix, 20 consumer lease/5 identities,
  11초 happy-path 한정, EOD 분리 원칙은 coordinator가 개발 계약으로 수락한다.
- 새 quote 응답 필드 해석의 승인 gate와 실제 운영 활성화는 보류한다. 당일 calendar/
  session-window 증거 공급도 production 전제이며 해결 완료로 보고하지 않는다.
- shared primitive의 provider-free 개발은 승인된 구현 범위다. 계약의 crash reservation
  보강과 default-off 호환 모순을 분리해 WP-2A/2B로 순차 분해했다.
- worker 완료 후 미처리 방지를 위해 bounded Paseo heartbeat를 사용한다. 완료/오류/실제
  사용자 결정 필요 시 결과를 검토하고 후속 작업 또는 정확한 blocker를 처리한다.

### WP-2A coordinator 검토 — 보완 후 재수락

- worker `c52971ef-6447-4fd5-98dd-e4ec6bcc1922`의 idle 완료와 커밋
  `392a1b0112fa5aca8c457dbb0dd12dc8e1cd65de`를 확인했다. 변경은 지정한 6개 파일뿐이며
  worker tree와 diff check는 clean이다. worker는 offline 테스트 182개와 fmt/clippy 통과를
  보고했다. 아직 integration에 해당 구현을 통합하거나 WP-2B를 시작하지 않는다.
- 직접 코드 검토에서 다음 보완을 요구했다: 초기화된 root에서 state 파일 소실을 최초
  실행으로 취급하여 token/quota ledger를 재설정하지 않을 것; 전달된 request timeout을
  실제 pending callback의 중단에 적용하고 timeout 시 durable reservation을 보존할 것;
  async 실행 경로의 state read/write/fsync를 reactor 밖에서 수행할 것.
- 동일 idle worker에 위 세 항목만 후속 위임한다. scope는 WP-2A 모듈과 전용 테스트이며
  기존 caller/응답 parser/운영 설정은 여전히 수정하지 않는다. 회귀 검증과 보완 커밋을
  확인한 후 WP-2A 수락 및 WP-2B 호환 결정을 진행한다.

### WP-2A 수락 및 WP-2B 연결 결정

- 보완 `add315a`의 전체 diff와 새 테스트를 직접 검토했다. 초기화 witness, callback timeout,
  잠금을 소유하는 blocking I/O 작업이 세 지적을 처리한다. worker의 offline 187개 테스트,
  fmt/clippy 통과 보고와 clean scope를 확인했다. integration은 `ac694a2` + `65853e9`이며
  disconnected primitive로만 수락한다. 운영 환경 및 전체 shared-boundary 수락은 아니다.
- WP-2B 호환 정책: `KIS_READ_COORDINATION_MODE`는 누락 시 `legacy`, 명시값은
  `legacy|shared_required`만 허용한다. legacy는 기존 EOD 동작을 유지하지만 intraday를
  제공하지 않는다. `OWNER_INTRADAY_QUOTES_MODE=owner_only`는 shared_required가 필수이고,
  누락/`off`만 비활성으로 취급한다. 비어 있거나 알 수 없는 명시값은 오류다.
- shared_required는 기존 credential 경로를 그대로 사용하며 production root를
  `/run/lagrange/kis-read-coordination`, UID 10001로 고정한다. 양의 canonical
  `KIS_READ_CREDENTIAL_GENERATION`이 필수이고, 안전하지 않은 공유 상태/설정은 callback
  전에 실패한다. legacy fallback이나 임의 root/env override는 허용하지 않는다.
- 위 모드를 네 production reader 생성 지점에 명시적으로 연결하고 provider-free 생성/호출
  테스트로 검사한다. 섹션 5.2의 모든 LiveTransport 즉시 금지 문구는 이 호환 결정으로
  대체한다. intraday 운영 활성화 전에는 네 caller 모두 shared_required로 전환했음을 WP-6과
  별도 release gate에서 입증해야 한다. legacy 혼재 상태가 전역 보호된다고 주장하지 않는다.
- EOD와 intraday는 동일 inquire-price 채널이라도 explicit read class로 구별한다.
  EOD는 1초 공유 간격, intraday만 추가 5초/5,000회 한도를 소비한다. intraday class는
  해당 exact path/TR에서만 허용하고 coordinated client에서만 사용할 수 있다.
- WP-2B는 기존 네 caller 접속점, kis-client auth/market_data/retry/error/token_issuer,
  read_coordination 및 lib export와 해당 focused test를 순차 소유한다. parser/DB/Web/Compose
  변경과 실제 provider 호출은 제외한다. config parser가 필요하면 kis-client 신규
  `read_coordination_config.rs`에 한정한다. Cargo 의존성 추가는 이번 연결 범위에서 제외한다.
- WP-2B worker: `1a321ba4-6ac2-46c2-a7c1-01ccbc0cf049`, Codex sol/high,
  workspace `wks_9348f3ca1a30ddb5`, cwd
  `/data/worktrees/3puw275b/stock-beta-intraday-read-wiring`, base `2c4b9a0`.
  기존 WP-2A heartbeat를 종료하고 WP-2B용 5분 간격, 최대 2시간 heartbeat로 교체했다.

### WP-2B 검토 및 WP-5 독립 실행 결정

- WP-2B worker의 idle 완료와 `6a7b87a33d2812ffc13f5bae5c9c878fe2e8c4d3`를 확인했다.
  변경은 지정된 9개 파일이며 worker는 offline kis-client 199개, collectors 67+13개,
  runner 7개 테스트와 focused clippy/fmt 통과를 보고했다. coordinator가 mode parser와
  shared GET 실행 경로를 직접 확인했고, 토큰·예약·caller 경계를 별도 Codex terra/high
  reviewer `f7941398-44f7-4293-a6d5-c72bac530a97`에 read-only 위임했다.
  아직 WP-2B를 통합하거나 shared-boundary gate를 수락하지 않았다.
- Graph amendment: WP-5는 WP-1의 application DTO만 사용하는 독립 provider-free Web
  구현이므로 WP-2B 리뷰와 병렬로 시작할 수 있다. WP-3의 shared-boundary/response 승인
  gate는 그대로다. WP-5는 Rust/provider parser를 읽어 새 계약을 추측하거나 수정하지 않는다.
- WP-5 default-off 연결은 서버에서 `OWNER_INTRADAY_QUOTES_MODE`를 읽어 정확히
  `owner_only`일 때만 enabled prop을 넘긴다. 누락/`off`는 disabled이며 다른 값도 활성화하지
  않는다. client env 접근이나 credential 전달은 금지한다. 현재 Compose/env는 수정하지 않는다.
  disabled일 때 기존 EOD 화면과 조회 수는 유지하고 현재가 위젯의 요청·타이머는 0이다.
- 최소 scope amendment: 두 Stock Beta `app/(authenticated)/stock-beta/**/page.tsx`의
  enabled prop 연결과 detail의 기존 membership API 조회를 허용한다. detail에서는 flag가
  켜졌을 때만 READY membership의 instrument/generation을 signal과 정확히 대조해 넘긴다.
  조회 실패는 quote만 unavailable 처리하며 기존 EOD detail은 유지하되 인증 실패는 기존
  로그인 복구를 따른다. dashboard도 READY membership·선택 signal의 일치를 요구한다.
- catalog 등록에 따른 최소 renderer 접속은 dashboard `stock-beta-dashboard.tsx`, detail
  `stock-beta-detail-layout.tsx`까지 허용한다. 신규 optional `current-quote` entry와 visibility
  전달만 다루며 catalog 엔진을 재설계하지 않는다. breakpoint별 CSS 숨김도 실제 소비자
  lifecycle에 반영한다. 기존 `stock-beta-dashboard.test.tsx`, `owner-beta-equity-signals-surface.test.tsx`,
  `stock-beta-widget-architecture.test.tsx`는 신규 optional entry의 목록/순서 기대값만 조정할 수
  있다. 다른 assertion 완화/기존 테스트 삭제 금지, 새 동작 회귀는 신규 intraday tests에 둔다.
- logout 시작 시 즉시 정리를 위해 `lib/api/browser-client.ts`의 logout 접속점과 신규
  `lib/api/browser-lifecycle.ts`의 payload 없는 local logout notification만 허용한다.
  quote-specific 로직을 공통 auth 모듈에 넣거나 기존 CSRF/응답 복구를 변경하지 않는다.
- WP-5 application schema는 section 8의 exact DTO/route를 사용한다. 금액은 문자열을
  유지하고 `numeric(20,8)` 저장 가능 범위인 정수부 최대 12자리·소수부 최대 8자리도 검사한다.
  quote version은 bigint 문자열 비교, generation/sequence는 JS safe integer를 요구한다.
  이 수신 DTO 방어는 KIS의 새로운 응답 필드 parser 승인이나 운영 활성화를 뜻하지 않는다.
- 새 response field 파싱은 여전히 owner의 명시적 승인 전 보류한다. 실제 provider 호출,
  운영 변경, main merge/push는 두 worker 모두 금지한다.

### WP-2B 보완 및 WP-5 launch 기록

- 독립 reviewer가 idle/REJECT를 반환했다. coordinator도 401이 기존 read retry predicate에서
  제외된 코드와, 두 child를 단순 해제하는 기존 테스트를 직접 확인했다. 401 뒤 한 번의
  shared 재발급 기회 및 실제 contention barrier 증거를 WP-2B 담당에게 요구했다.
- 추가로 durable anchor 검사 뒤 callback 첫 poll이 지연될 때 다음 호출 간격이 짧아질 수
  있다는 정적 지적은 deterministic reproduction부터 요구했다. 아직 실제 provider에서
  발생한 장애라고 보고하지 않는다. 임의 lead 확대가 아닌 보수적인 spacing·deadline·
  cancellation 처리와 명확한 callback 보장 범위를 검증하게 했다.
- 기존 idle WP-2B worker `1a321ba4-6ac2-46c2-a7c1-01ccbc0cf049`에 위 세 항목만
  후속 위임했다. 수정은 kis-client 공유 read 모듈과 focused tests 중심이며 외부 API 응답
  parser/운영 설정은 제외한다. `6a7b87a`는 아직 통합하지 않았다.
- WP-5 worker `6302f7a6-76ba-48b6-9e61-785e92b8fd20`, Codex luna/max,
  workspace `wks_f63a48bceda4e05e`, cwd `/data/worktrees/3puw275b/stock-beta-intraday-widget`,
  branch `work/stock-beta-intraday-widget-20260908`, base `d41a733`의 running을 확인했다.
  위 default-off/scope amendment와 exact application DTO로만 구현한다. Browser/Next runtime
  검증과 실제 API 연동은 후속 gate이며 이번 worker의 unit 검증과 구별한다.

### WP-2B 보완 완료와 재검토

- `85ac02b28b0c095cf6ca18a00a6aa396e0e8649f`의 idle 완료 및 지정 3개 파일/clean tree를
  확인했다. worker는 수정 전 EOD/intraday 간격 회귀의 실패를 재현했으며 수정 후 전체
  kis-client 204개, fmt/clippy 통과를 보고했다. 아직 통합하거나 최종 수락하지 않았다.
- coordinator가 두 production 모듈의 보완 diff와 새 OS child barrier를 직접 확인했다.
  최종 dispatch guard, 완료 시각 기준의 보수적 간격, 401 재발급 1회 기회, 첫 issuer를
  유지한 채 다른 프로세스의 LockBusy를 관측하는 테스트가 추가됐다. 완료 기준 간격은
  처리량이 낮아질 수 있으며 정확히 5초마다 실제 요청이 시작된다는 보장은 하지 않는다.
- 기존 idle reviewer `f7941398-44f7-4293-a6d5-c72bac530a97`에 위 delta와 앞선 세 지적의
  해소 여부를 read-only 재검토하도록 후속 위임했다. WP-5는 별도 worktree에서 계속 진행한다.

### WP-2B shared-boundary 개발 gate 수락

- reviewer `f7941398-44f7-4293-a6d5-c72bac530a97`의 idle/ACCEPT를 확인했다.
  세 지적 모두 해소됐으며 reviewer가 직접 offline integration 36개와 market-data 13개를
  순차 실행해 통과했다. 전체 204개/fmt/clippy는 구현 담당의 검증 보고와 구분한다.
- coordinator의 코드·barrier 확인, 독립 검토와 focused 재실행, 지정 9개 파일/clean 상태를
  근거로 provider-free shared-boundary 개발 gate를 수락한다. 실제 provider나 운영 전역
  보호의 검증 완료를 뜻하지 않으며 모든 reader의 shared_required 배포 전제는 유지한다.
- `6a7b87a` → `0684ab9`, `85ac02b` → `87e672c`로 기능 integration branch에 통합했다.
  통합된 9개 source/test 파일이 검토 HEAD와 동일함을 diff로 확인했다. main merge/push나
  provider 호출·운영 변경은 수행하지 않았다.
- WP-5는 실행 중이다. WP-3의 새 KIS 응답 필드 해석은 별도로 요청한 owner 승인에 아직
  답변이 없어 시작하지 않는다. 승인 대기 때문에 독립 Web 구현을 중단하지 않는다.

### WP-5 완료 결과 회수 및 독립 Web 리뷰

- implementer의 idle과 `743bca7ca2e33ed1b76a364599ff51e5d059f0dd`, clean tree를 확인했다.
  최종 변경은 Web 33개 파일이다. 작업 도중 보였던 기존 `detail/detail.module.css` 변경은
  최종 커밋에 없으며 기존 EOD 테스트의 변경은 catalog 목록/optional 기대값뿐이다.
- worker는 focused Vitest 10 files/75 tests, full Vitest 44 files/296 tests,
  typecheck/lint/diff check 통과를 보고했다. dependency symlink는 제거됐다.
  이 보고를 browser QA 또는 실제 API/provider 연동 완료로 취급하지 않는다.
- coordinator가 page/flag/membership, registry/renderer, workspace 및 logout 접속 diff와
  hook/widget 연결 코드를 직접 확인했다. 통합은 독립 검토 후 결정한다.
- bounded read-only reviewer `6bd4786b-5ed1-4ab5-bcc4-999210431972`, Codex terra/high,
  동일 frozen WP-5 worktree에서 running을 확인했다. strict DTO/client/coordinator/hook,
  cleanup/race/freshness/catalog lifecycle 및 테스트 근거를 검토하며 소스 수정은 금지한다.
  이는 WP-5 개발 gate의 사전 리뷰이지 후속 WP-7 전체 계층 리뷰의 대체가 아니다.
- WP-3 response-field 승인 대기는 유지한다. main merge/push, provider 호출, Next/browser,
  DB/운영 활성화는 수행하지 않았다. heartbeat는 구현 담당 대신 리뷰 담당을 추적한다.

### WP-5 독립 리뷰 지적 및 bounded 보완

- reviewer의 idle/REJECT를 회수했다. 독립 focused 10 files/75 tests, typecheck/lint는
  통과했지만 다음 네 결함이 남았다: demand lease 만료 후 polling 중단/복구 부재(High),
  receipt/success의 KST 날짜와 session 날짜 대조 누락(High), 30초/24시간 경계의 조기
  stale/만료 전환(Medium), 매 poll의 거짓 unavailable 알림과 HALTED 표시 누락(Medium).
- coordinator가 acceptDemand/poll/statusText/parser와 동결 lease/세션/freshness 계약을
  대조했다. 구현 커밋은 미통합으로 유지하며 idle implementer에게 네 항목만 후속 위임한다.
  수정 범위는 quote coordinator/view, application contracts, 신규 intraday tests와 필요한
  최소 locale 문구다. 기존 EOD assertion, API/DB/provider 계약 변경은 허용하지 않는다.
- lease 만료는 cache retention과 별개로 관리한다. 만료 시 cache GET을 중단하고 ambiguous
  mutation은 같은 key/body/sequence로 serialized 재처리한다. pending mutation 중 중첩이나
  consumer 교체로 quota를 우회하지 않는다. 정해지지 않은 서버 동작은 추측하지 않고 보고한다.
- 회귀 재현 후 수정·full Vitest/typecheck/lint를 요구했다. 완료 후 동일 reviewer의
  read-only 재검토를 거쳐 수락한다. WP-3 응답 parser 승인 대기는 그대로 유지한다.

### WP-5 보완 결과 회수 및 재검토

- `4d39adb465e9020d9b03bfe9679d4ec51046bbb1`의 idle/clean 및 지정 7개 파일 변경을
  확인했다. worker는 수정 전 회귀 재현, focused 3 files/27 tests, full 44 files/306 tests,
  typecheck/lint/diff check 통과를 보고했다. 아직 두 WP-5 커밋 모두 통합하지 않는다.
- coordinator가 production delta를 직접 확인했다. 별도 lease timer와 KST 날짜 검사,
  strict freshness 경계, fetching/semantic status 분리가 추가됐다. 다만 lease 만료 시
  stop() 이후 복구 없이 idle에 머무는지, 요청 실패 후 retained 값을 ready로 표시하면서
  실패 이유를 숨기는지를 재검토 항목으로 명시했다. worker 요약만으로 해소 판정하지 않는다.
- 기존 idle reviewer에게 네 원지적 및 위 delta의 회귀를 read-only 후속 위임했다.
  구현 담당은 대기하며 수정/리뷰 동시 소유는 없다. provider 응답 승인 대기와 운영 제외는 유지한다.

### WP-5 재검토 결과 및 복구 계약 명확화

- reviewer가 다시 idle/REJECT를 반환했다. 만료 후 영구 idle(High), 실패한 GET의 retained
  값을 정상으로 표시하는 회귀(High), GET 시작 시각을 미래 시각 판정에 사용하는 문제(Medium)가
  남았다. KST 날짜·strict 경계·background status/HALTED 원지적은 수정된 것으로 확인했다.
  독립 focused 3 files/27 tests와 typecheck/lint는 통과했지만 완료 판정으로 대체하지 않는다.
- coordinator가 client의 고정 nowMs 전달과 앞서 확인한 두 경로를 대조했다. application
  contract 8.2에 expired ACTIVE와 명시적 RELEASED를 구별하는 복구 규칙을 명시했다.
  다음 sequence 재갱신은 권한/READY/cap을 다시 확인하며 replay는 기존 만료를 연장하지 않는다.
  missing/RELEASED는 자동 새 consumer로 우회하지 않는다. WP-3/4의 구현·DB 검증 의무다.
- browser는 만료 시 GET만 중단/세대 무효화하고 mutation 직렬화 문맥을 유지한다. 복구는
  15초 이상 간격·연속 3회 제한이며 pending 요청과 중첩하지 않는다. 수신 시각 clock과
  실패 표시도 함께 보완한다. 이는 provider 계약 승인이나 실제 API 활성화가 아니다.
- 반복 미해결에 따라 기존 luna implementer는 idle로 유지하고 새 Codex terra/max 담당에게
  세 항목만 재배정한다(모델만 한 단계 상향, effort max 유지). 소유 범위는 quote coordinator/
  view, application client, 신규 intraday tests, 필요한 최소 locale이며 다른 변경은 금지한다.
- 새 담당자 `f148b96b-96fc-4f98-ba90-11c3baf8872a`의 terra/max running을 확인했다.
  동일 WP-5 workspace의 `4d39adb`에서 순차 소유하며, coordinator의 `40df089` 문서에
  확정한 복구 계약을 읽도록 지시했다. 재검토 담당자는 구현에 참여하지 않는다.

### WP-5 복구 수정 회수 및 독립 재검토

- terra 담당자의 idle/clean 및 `9ccd6f3fb802c6bcde03250f1bdfd437c609864c`를 확인했다.
  지정된 8개 Web 파일만 변경됐으며 기존 두 커밋과 함께 아직 미통합이다.
- coordinator가 production/test delta를 직접 읽었다. 같은 consumer의 만료 복구,
  GET epoch fencing, 실패 표시 유지 및 응답 수신 후 clock 검증이 추가됐다.
  worker는 focused 4 files/35 tests, full 44 files/313 tests, typecheck/lint 통과를 보고했다.
- 기존 idle reviewer에게 세 원지적과 새 경계 조건을 read-only 재검토하도록 전달했다.
  특히 만료 이전 Retry-After의 만료 이후 보존, 타이머보다 늦은 renewal 응답이 먼저
  처리될 때 이전 GET의 무효화 여부는 별도로 확인한다. 통과 보고만으로 수락하지 않는다.
- 구현 담당은 idle로 유지한다. backend 복구 계약 검증 및 KIS 응답 parser 승인 대기는
  그대로이며 browser/운영 검증이나 main 통합·push는 수행하지 않았다.

### WP-5 복구 경계 재검토 결과

- reviewer가 `9ccd6f3`에 REJECT를 반환했다. 만료 이전 429 Retry-After 대기 시간이
  만료 시 사라지는 문제와, 만료 타이머보다 갱신 응답이 먼저 처리될 때 기존 GET이
  새 lease 아래 반영되는 문제를 High로 확인했다. coordinator도 해당 경로를 대조했다.
- 실패 표시 및 응답 수신 후 clock 판정은 수정됐다. 독립 lint는 통과했으나 focused
  테스트/typecheck는 기존 app node_modules의 의존성 해석 실패로 실행 증거가 불충분하다.
  기존 캐시 디렉터리는 보존하며, 구현 담당자의 root 임시 symlink 검증 방식으로 재시도한다.
- idle terra 담당자에게 coordinator와 신규 coordinator test 두 파일만 후속 위임했다.
  절대 retry-not-before 유지 및 타이머와 무관한 이전 GET epoch 무효화를 회귀 테스트로
  먼저 재현한 뒤 수정한다. 기존 커밋은 모두 미통합이며 재검토 후에만 수락한다.

### WP-5 경계 수정 완료 및 검증 재개

- `f0442733f246e5d93eb7d8de57e9b37403bab2d8`의 idle/clean 및 두 파일 범위를 확인했다.
  coordinator가 전체 delta를 읽었으며 절대 재시도 기한과 이전 lease의 GET 무효화가
  추가됐다. worker는 수정 전 27개 중 3개 실패 재현, 수정 후 27개 통과 및 전체 Web
  44 files/316 tests, typecheck/lint 통과를 보고했다.
- idle reviewer에게 두 High와 회귀를 재검토하도록 전달했다. 독립 실행에서는 기존 app
  캐시를 보존하고 repository root의 임시 dependency symlink 방식으로 검증을 재개한다.
  아직 WP-5 커밋은 모두 미통합이며 backend/browser 검증이나 provider 승인을 뜻하지 않는다.

### WP-5 개발 gate 수락 및 통합

- 독립 reviewer가 `f0442733f246e5d93eb7d8de57e9b37403bab2d8`에 ACCEPT를 반환했다.
  focused 4 files/38 tests, 전체 Web 44 files/316 tests, typecheck/lint가 독립 실행에서
  통과했다. 남은 Web 개발 gate 지적은 없으며 기존 warning 4개와 config info만 남았다.
- 기능 브랜치에 순차 통합했다: `743bca7` → `a030cf9`, `4d39adb` → `110f49a`,
  `9ccd6f3` → `34b5839`, `f044273` → `06ea903`. 통합한 apps/web 전체가 독립 검증된
  worker HEAD와 동일함을 git diff로 확인했고 diff check와 작업 트리 상태도 통과했다.
- WP-3은 신규 KIS 현재가 응답 7개 필드의 파싱에 대한 명시적 owner 승인이 아직 필요하다.
  승인 없이 parser를 시작하지 않는다. WP-4/6/7/8은 dependency가 충족되지 않았으며
  별도 독립 Web 작업은 완료됐다. 사용자 결정 대기를 알리고 반복 heartbeat를 종료한다.
- main merge/push, 배포, 실제 KIS 호출, API/DB 및 브라우저 런타임 검증은 수행하지 않았다.
  본 수락은 fixture 기반 Web 개발 gate이며 전체 기능 QA 또는 출시 승인이 아니다.

### Owner 응답 parser 승인 및 WP-3 분할

- 2026-09-08 사용자가 직전의 7개 응답 필드 파싱·테스트 구현 승인 질문에 “승인해”로
  답했다. `stck_prpr`, `prdy_vrss`, `prdy_ctrt`, `prdy_vrss_sign`, `stck_sdpr`,
  `iscd_stat_cls_code`, `temp_stop_yn`의 동결 계약을 fixture 기반으로 구현할 수 있다.
  실제 KIS 호출, 운영 활성화, main merge/push/배포 승인은 포함하지 않는다.
- WP-3을 순차 분할한다. WP-3A는 독립 parser와 typed 값/오류 및 fixture 테스트만 맡는다.
  owned: `crates/market-data/src/intraday_quotes.rs`, lib.rs의 module 등록 한 곳,
  `crates/market-data/tests/intraday_quotes.rs`. 기존 EOD/reference parser는 변경하지 않는다.
  intermediate, Codex luna/max, 결과는 coordinator 및 독립 검토 후 수락한다.
- WP-3B는 WP-3A 수락 후 기존 WP-3의 migration/cache/demand/producer 범위를 맡는다.
  0054 번호 충돌 및 실제 disposable DB 검증 환경은 해당 launch 전에 확인한다.
  WP-3A에는 DB/runner/transport/Compose/Web 변경이나 네트워크 호출을 주지 않는다.
- WP-3A 담당자 `b4a77188-5d3a-4685-b5bf-1f2535a1a1d1` (Codex luna/max)를
  `/data/worktrees/3puw275b/stock-beta-intraday-parser`, workspace `wks_532168c8d9670b4d`,
  base `9f4fb4f07f1dff73772922403b90d457d858ff96`에서 시작했다. 공식 KIS 필드 매핑과
  inquire-price 공개 문서를 재확인했으며 샘플 실행이나 실제 provider 요청은 하지 않았다.

### WP-3A 구현 완료 및 독립 검토

- 담당자는 `3ff2bb4659d59259cc04307d143dd1461fd881d0`을 완료하고 idle/clean 상태다.
  coordinator가 지정된 세 파일의 전체 source/test를 읽고 범위를 확인했다. 아직 미통합이다.
- worker는 offline/locked focused parser 테스트 15개, scoped clippy, fmt 및 diff check
  통과를 보고했다. 독립 reviewer `0ca82aff-cdd6-4d78-ae6f-8a841b6e1fe5`
  (Codex terra/high)를 같은 workspace에서 시작했고 running 상태를 확인했다.
- 중복 critical 필드 거부, decimal/sign/halt 경계, 오류 redaction과 fixture의 실제
  검증 대상을 독립 확인한다. 일부 wrong-shape fixture가 malformed JSON으로도 실패하는
  점은 검토 대상으로 전달했다. ACCEPT 이전에는 통합하거나 WP-3B를 시작하지 않는다.
- 실제 provider, DB, runner, browser, 운영 변경은 수행하지 않았다.

### WP-3A 독립 수락 및 테스트 보강

- 독립 reviewer는 `3ff2bb4`의 parser 구현에 ACCEPT를 반환했다. focused 15개 테스트,
  scoped clippy/fmt/diff check가 독립 실행에서 통과했다. 추가 임시 probe는 유효 JSON의
  잘못된 envelope, 모든 critical 필드의 null/숫자 타입/escaped 중복을 거부함을 확인했다.
- 남은 Low는 영구 테스트 범위다. coordinator가 두 fixture의 잘못된 JSON 구성을
  직접 확인했고, idle luna 담당자에게 기존 테스트 한 파일만 보강하도록 후속 위임했다.
  reviewer는 idle이며 담당자는 running/permission 대기 없음으로 확인됐다.
- 보강 후 검증된 두 커밋을 통합한다. WP-3B 준비 확인에서 0054 번호는 비어 있고,
  고정된 disposable QA Compose 정의가 존재한다. 로컬 PostgreSQL 실행 파일은 없지만
  sandbox 밖 read-only Docker version 조회는 성공했다. 운영 DB를 대안으로 사용하지 않으며
  아직 QA DB를 시작하거나 migration을 적용하지 않았다.

### WP-3A 통합 및 WP-3B 실행 분할

- Low 테스트 보강 `1a7a52701d9bdeba1d27f267861134a75f486ce7`의 단일 파일 diff를
  coordinator가 읽었다. 유효 JSON 확인과 8필드 × 4형태 32개 거부 사례가 추가됐고,
  focused 16개 테스트, scoped clippy/fmt/diff check 통과를 확인했다.
- 수락된 source `3ff2bb4` → `37286bb`, test `1a7a527` → `fc795e5`를 기능 브랜치에
  통합했다. 세 파일 전체가 worker HEAD와 동일하고 작업 트리는 clean이었다.
- WP-3B를 같은 동결 계약 안에서 B1 저장소, B2 producer 연결로 순차 분할한다.
  테스트 가능한 DB 경계를 먼저 수락해 loop/transport와 권한·동시성 문제를 분리한다.

| Package | Complexity | Basis | Confidence | Escalation signal |
| --- | --- | --- | --- | --- |
| WP-3B1 | intermediate | 동결 0054 schema 및 명시적 transaction/RLS 구현, 실제 역할별 DB 검증 가능 | medium | 새 권한/trigger 필요 또는 반복 동시성 검증 실패 |
| WP-3B2 | intermediate | 수락된 저장소와 shared read/parser 연결, 동결 session/budget/fairness 규칙 | medium | 기존 EOD lease 침범 또는 계약에 없는 runtime 의존성 필요 |

| Package | Wave | Objective | Owned scope | Depends on | Worker | Verification |
| --- | --- | --- | --- | --- | --- | --- |
| WP-3B1 | 4a | demand/cache/producer lease 저장소와 권한·fencing 검증 | 0054 up/down; 신규 job-queue owner_equity_v2/intraday.rs; 상위 module 등록만; 신규 intraday_quotes DB/unit tests | WP-3A | Codex luna/max | disposable DB role/RLS, capacity/idempotency/expiry/takeover/publication/GC/up-down; fmt/clippy |
| WP-3B2 | 4b | provider-free 검증된 producer loop 연결 | 신규 collectors intraday_quotes.rs 및 등록; 기존 runner quote-loop wiring; B1 모듈의 필요한 통합; 신규 quote scheduler tests | B1 ACCEPT | Codex luna/max | fake clock/transport fairness/session/budget/deadline 및 DB fenced publish |

- B1은 §8.2/9.2의 expired ACTIVE 복구와 exact replay 원래 만료 유지, RELEASED 재활성화
  금지, 20 consumer/5 identity cap을 실제 동시 transaction으로 검증한다. owner별 producer
  fencing과 cache success/failure 분리, 0053 composite lineage, 기존 EOD 불변도 검증한다.
  parser/collector/runner/API/Web/Compose/Cargo/기존 migration·test는 변경하지 않는다.
- B2 전까지 runtime 호출은 연결하지 않는다. 저장소 API는 typed inputs/SQL로 작성하며
  검증되지 않은 session/budget을 승인된 것으로 가장하는 production placeholder는 금지한다.
  다른 파일 또는 계약 변경이 필요하면 해당 경계를 coordinator에게 보고한다.
- 각 패키지는 독립 review 후 소유 범위와 기계적 검증을 대조하여 통합한다. 보고는 파일/라인,
  deviation, 정확한 검증 결과, 미해결/미검증 항목을 명시한다. 전체 API/browser 수락은 후속 gate다.
- 격리 QA DB: `lagrange-intraday-qa-20260908` project, 고정 PostgreSQL 18.4 image,
  tmpfs, `127.0.0.1:55438`만 바인딩. 신규 이름/포트의 공백을 확인한 뒤 `--pull never`로
  시작했고 healthy 및 SQL version 응답을 확인했다. 운영 DB·서비스는 사용/변경하지 않았다.
  coordinator가 해당 QA 컨테이너만 관리/정리하며 worker는 loopback의 임시 test DB만 사용한다.
- B1 담당자 `6416026d-b838-4fa1-a697-f2243b618577` (Codex luna/max)를 base `a0da246`,
  `/data/worktrees/3puw275b/stock-beta-intraday-storage`, workspace `wks_cb50f59b1dc645f6`에
  시작했다. source/parser 담당자와 reviewer는 idle로 유지하며 mutable 범위는 겹치지 않는다.

### WP-3B1 완료 보고 및 독립 검토

- 구현 담당자는 `b02959d4be8cc5aee8dacceaf57ecb85c29cf627` (parent `a0da246`)로
  완료했으며 idle/clean이다. 소유한 6파일만 변경했고 아직 기능 브랜치에 통합하지 않았다.
  worker는 격리 DB 9 tests/5.76s, all-target clippy 및 fmt/diff 통과를 보고했다.
- coordinator가 migration 전체와 demand/cap/publication 경로 및 테스트 일부를 직접 읽었다.
  frozen policy-row lock은 기존 app SELECT-only 권한 때문에 실행 불가능하다는 worker
  보고가 있고, 실제 구현은 owner advisory transaction lock + policy 존재 확인이다.
  권한 추가는 없지만 계약과의 차이는 아직 수락하지 않았다.
- 독립 reviewer `4ff1fb96-28a4-4a6d-ac03-11dca13f875a` (Codex terra/high)를 같은
  storage workspace에서 시작했다. 전체 6파일과 DB 검증, 정책 잠금 대안의 보장 범위를
  read-only 검토한다. coordinator가 확인한 5-identity cap의 기존 identity 신규 consumer
  거부 가능성과 lock 대기 중 expiry/receipt 순서 경계도 재현·판정 대상으로 전달했다.
- B2는 B1 review ACCEPT와 필요한 coordinator 계약 판단 이후에만 시작한다.
  임시 QA DB는 검토에 계속 사용하며 운영 DB/provider/배포에는 접근하지 않는다.

### WP-3B1 독립 REJECT 및 범위 한정 수정

- reviewer는 기존 DB 9/9 및 job-queue all-target clippy/fmt를 통과시켰지만, 격리 DB
  회귀 probe로 세 결함을 재현했다: 5개 identity에서 동일 identity의 새 consumer 거부,
  producer 잠금 대기 중 만료 후에도 transaction-start now()로 publish 허용,
  동일 fence의 오래된 receipt가 최신 cache를 덮음. 아직 통합하지 않는다.
- coordinator가 해당 SQL과 호출 경로를 읽어 수정 대상으로 수락했다. 기존 Tokio
  시작 barrier에 더해 실제 SQL/advisory lock 대기를 관찰하는 테스트를 추가한다.
- coordinator 계약 판단: policy UPDATE 권한을 추가하지 않고 owner별 advisory mutex를
  intraday capacity 전용으로 채택한다. policy는 존재 anchor일 뿐이며 0053 관리 작업을
  잠근다고 주장하지 않는다. 고정 cap과 actor scope, READY/current-generation 검증,
  publication membership lock은 유지한다. spec 6.1/8.2/9.1에 post-lock DB clock 및
  receipt 역행 거부도 명시했다. 실제 provider/운영 권한에는 변화가 없다.
- 기존 luna/max 구현 담당자에게 첫 bounded remediation을 맡긴다. 소유 범위는 새
  intraday repository, intraday DB tests, 필요 시 새 test support뿐이다. migration,
  기존 0053/EOD/parser/API/Web/Compose/Cargo는 수정하지 않는다. 세 재현 테스트를
  먼저 실패시킨 뒤 수정하고 독립 재검토를 받는다. B2는 ACCEPT 이후에만 시작한다.
- 전체 workspace clippy는 unowned kis-historical-price-v3-artifact.rs:448의
  large_enum_variant로 실패했다. B1 변경 파일은 아니지만 baseline 재현은 아직 하지
  않았으므로 기존 결함이라고 확정하지 않는다. 이 수정 범위에 편입하지 않는다.

### WP-3B1 수정 완료 및 재검토

- 구현 담당자는 `8c25106ecfb9492baeeefcbcebe7c7d4b7dbccb5` (parent `b02959d`)로
  한정한 세 파일만 커밋하고 idle/clean 상태다. 원본과 수정 모두 아직 미통합이다.
  수정 전 11 pass/9 fail로 세 결함 재현, 수정 후 DB 21/21(22.94s), job-queue
  all-target clippy/fmt/diff 통과를 보고했다. 독립 검증 전이므로 수락하지 않는다.
- coordinator가 production 변경의 주요 경로와 SQL 대기 관찰 helper 및 테스트 일부를
  확인했다. 동일 identity cap 허용, cache 잠금 후 clock 샘플, stale-receipt typed
  거부가 추가됐다. 기존 reviewer `4ff1fb96-28a4-4a6d-ac03-11dca13f875a`에게
  세 결함 및 amended 계약을 기준으로 read-only 재검토를 맡겼다.
- 추가 확인 대상으로 demand 갱신이 fresh clock 이후 UPDATE에서 publication의
  FOR SHARE 잠금을 기다릴 가능성을 전달했다. advisory lock 대기 테스트만으로
  이 경계가 검증되는지는 아직 판정하지 않는다. reviewer가 재현/반증한다.

### WP-3B1 남은 후행 잠금 경계 수정

- 독립 재검토는 21/21 DB tests(23.02s), scoped clippy/fmt를 통과시켰고 최초 세
  High 및 SQL 대기 증거 보완은 해결로 판정했다. 하지만 추가 probe 두 건은 실패했다.
  demand 갱신이 row FOR SHARE 뒤 UPDATE에서 기다리며 30초 lease를 짧게 만들고,
  최초 producer INSERT가 다른 미커밋 INSERT의 rollback을 기다리며 20초 lease를
  대기 전 시각에서 만들었다. coordinator가 두 코드 경로를 확인하여 수정 대상으로
  수락한다. 원본/첫 수정 모두 아직 미통합이다.
- 시간/잠금 경계가 두 차례 검토에서 남았으므로 모델만 luna→terra로 한 단계 올리고
  effort는 max로 유지한다. 새 독립 구현 담당자는 같은 clean workspace의 `8c25106`
  에서 두 경로만 수정한다. 기존 luna 및 reviewer는 idle이며 파일 소유권은 겹치지 않는다.
- scope: 새 intraday repository와 새 DB tests, 필요한 test support만. 기존 demand
  row를 갱신/해제 전에 잠근 후 clock을 읽고, 최초 producer insert 성공 후 자기
  미커밋 row의 시간을 다시 설정한다. 새 회귀는 실제 demand-row/unique-index 대기를
  관찰하고 먼저 실패를 재현한다. grant/migration/0053/B2/API/Web/운영 변경은 없다.
- reviewer는 수정 후 다시 독립 수락 판단을 한다. B2는 B1 ACCEPT 이후로 유지한다.
- 새 담당자 `1d21676d-0284-43db-a1e3-2936dcaafc19` (Codex terra/max)를 storage
  workspace `wks_cb50f59b1dc645f6`에서 시작했으며 running/권한 요청 없음 확인했다.

### WP-3B1 후행 잠금 수정 완료 / 수락 재검토

- terra 담당자는 `c22bf5728e755b0e60d475f16f3ec3ee63aa5776` (parent `8c25106`)로
  repository와 DB test 두 파일만 커밋하고 idle/clean이다. coordinator가 전체 두 파일
  diff를 읽었다. 기존 demand renew/release SELECT에 FOR UPDATE가 추가됐고 최초
  producer insert 성공은 post-lock clock으로 자기 row를 갱신하되 fence 1/Acquired를
  유지한다. 재현 테스트는 두 demand-share-lock 경로와 initial unique-index rollback이다.
- 담당자는 수정 전 21 pass/3 fail, 수정 후 24 pass(30.57s), scoped clippy/fmt/diff
  통과를 보고했다. 격리 DB 접속 sandbox 제한은 정상 승인 경로의 재실행으로 해결했고
  운영 DB나 container lifecycle 변경은 없었다. 독립 reviewer에게 수락 재검토를 맡긴다.
  `b02959d`, `8c25106`, `c22bf57` 모두 ACCEPT 전까지 미통합이다.

### WP-3B1 수락 및 WP-3B2 연결 경계 분할

- 독립 reviewer `4ff1fb96-28a4-4a6d-ac03-11dca13f875a`가 최종 `c22bf57`을 ACCEPT했다.
  격리 DB의 실제 app/worker 역할로 24/24 tests(30.5s), job-queue all-target clippy,
  fmt/diff check를 독립 통과했다. 원래 세 결함과 후속 두 clock 경계 모두 해결됐다.
- source `b02959d` → `63d8301`, `8c25106` → `ad2c3ce`, `c22bf57` → `46a70ca`를
  기능 브랜치에 통합했다. 소유한 여섯 파일이 수락된 worker HEAD와 정확히 같고 clean임을
  확인했다. B1 수락은 runtime/session-window/shared-budget 연결 수락이 아니다.
- B2 pre-launch 검사에서 `get_intraday`는 내부 3회 retry와 `MarketDataReply`만 제공하며,
  durable reservation의 KST date/count/fence를 반환하지 않는 것을 확인했다. 이 상태로
  B1 `IntradayAttemptReservation`을 채우거나 매 retry의 수요/producer/session 검증을
  가장하지 않는다. B2를 아래 두 순차 패키지로 좁힌다. 외부 응답 계약/호출 권한은 불변이다.

| Package | Wave | Complexity | Objective / owned scope | Depends on | Worker | Verification |
| --- | --- | --- | --- | --- | --- | --- |
| WP-3B2a | 4b1 | intermediate | kis-client read_coordination.rs/market_data.rs의 새 단일 guarded intraday attempt와 실제 예약/receipt metadata, lib re-export만, 신규 focused tests | B1 ACCEPT | Codex luna/max | fake transport/clock, shared contention/budget/deadline, 기존 read regression, fmt/scoped clippy |
| WP-3B2b | 4b2 | intermediate | 기존 B2 collector/session/scheduler/runner/DB 연결 범위; 새 단일 시도 API로 최대3회 retry를 명시적 소유 | B2a ACCEPT | Codex luna/max | fake session/fairness/retry + disposable DB fenced publish |

- B2a confidence medium: 기존 shared ledger/gate는 수락됐고 필요한 변경은 그 증거의
  typed 반환과 caller final eligibility 경계다. EOD 변경/새 dependency/저장 schema 변경이나
  계약 모순이 필요하면 보고 후 coordinator가 재분류한다. B2b confidence medium과 기존
  EOD lease 침범/runtime 의존성 escalation 조건은 유지한다.
- B2a 계약: 기존 get/get_intraday 동작/테스트는 유지하고 새 API는 정확히 한 번 이하의
  GET만 시도한다(내부 재시도 없음). exact path/TR, nonblocking shared lock, 전용3초
  transport, durable 5000/day 및5초 ledger, shared401 invalidation/cooldown을 그대로 쓴다.
  Caller의 async eligibility 검증을 final callback 안에서 실행한 뒤 dispatch guard와 send를
  인접하게 수행한다. false/error/cancellation이면 GET 0회; 이미 예약한 debt는 환불하지 않는다.
  검증의 시간도 기존 timeout/dispatch deadline 안에 포함한다. OS 선점 보장은 주장하지 않는다.
- 결과 metadata는 persist된 실제 reservation에서만 얻는 KST date/day count/fence이며
  credential fingerprint/token/body를 포함하지 않는다. 성공 receipt는 완전한 bytes 수신 뒤,
  응답 검증/파싱 전 clock을 캡처한다. 오류/Debug에는 broker body/message가 노출되지 않는다.
  B2b가 이 metadata로 저장소 입력을 구성하며 임의 성공 예약이나 두 번째 quota는 금지한다.
- B2a 테스트: 정상 metadata/receipt, legacy/wrong channel reject, busy skip, caller deny/error,
  callback delay/cancel, 401/429/timeout 단일 GET, no eager retry, budget exhaustion/date rollback,
  기존 EOD 및 intraday retry regressions. 실제 provider/DB/runner/API/Web/Compose/Cargo 수정 금지.
  하나의 compiler, CARGO_BUILD_JOBS=2, --locked --offline 사용. scoped tests/clippy/fmt 수행.
- 작업 디렉터리는 기능 브랜치에서 분기한 새 intraday-attempt workspace다. worker는 이 plan과
  spec 전체, root AGENTS를 읽고 파일/라인, deviation, 실행 명령/결과, 미해결/미검증(없으면
  없음)을 보고하며 소유 파일만 commit한다. 독립 review ACCEPT 후 scope/tree를 대조해 통합한다.
  B2b는 B2a 수락 이후 시작한다. QA DB는 후속 DB 작업을 위해 유지한다.

### WP-3B2a 완료 및 독립 검토

- 구현 담당자 `6ece60e0-e6f8-4825-a743-4f6a841d93ac` (Codex luna/max)가
  `51cd791685147e70ab95da0a3bf4500f501c2c82` (parent `ac436c5`)로 완료했다.
  `/data/worktrees/3puw275b/stock-beta-intraday-attempt`, workspace
  `wks_31cd2daa34b62251`에서 소유한 4파일만 변경했고 idle/clean이다. 아직 미통합이다.
- coordinator가 production diff 전체와 focused tests 일부를 읽었다. 실제 persist된
  예약 date/count/fence 반환, caller eligibility 이후 final dispatch, bytes 수신 직후
  검증 전 receipt clock을 확인했다. 기존 get/get_intraday 동작을 유지하는 별도 API다.
- worker는 신규 9 tests, coordination 36, market-data 13, package all-targets 213,
  scoped clippy/fmt/diff 통과를 보고했다. 독립 검증 결과로 간주하지 않는다.
- reviewer `b55535f1-8251-4323-9392-4c679f0b5219` (Codex terra/high)를 동일 workspace의
  read-only 검토로 시작했다. 새 API의 budget exhaustion 테스트가 기존 coordinator
  API만 실행한다는 coverage caveat와 실제 transport timeout 분류도 확인·반박 대상이다.
  임의 결함으로 확정하지 않는다. ACCEPT 및 검증 후에만 통합하고 B2b를 시작한다.

### WP-3B2a 수락 검토 후 마지막 범위 한정 보완

- reviewer는 51cd791에 Critical/High/Medium 구현 결함 없음으로 scoped ACCEPT했다.
  신규9/coordination36/market-data13/transport-agreement5 tests 및 scoped clippy/fmt/diff를
  독립 통과했다. runtime 수락은 아니며 신규 API 직접 budget exhaustion 증거와 concrete
  transport timeout fixture는 미검증으로 남겼다. reviewer는 idle/clean이다.
- coordinator가 live_transport.rs:96의 GET timeout → Broker status504와 새 API의
  transport Err 500..=599 → ProviderUnavailable를 대조했다. 따라서 로컬 timeout이
  Timeout 대신 일반 서버 장애로 분류되는 경로가 있다. fake live-shaped 오류로 먼저
  재현한 뒤 transport Err 504만 Timeout으로 교정한다. HTTP 응답 504 정책은 바꾸지 않는다.
- 유휴 구현 담당자에게 tests/intraday_attempt.rs와 위 market_data.rs 최소 수정만 맡겼다.
  기존 테스트를 유지하면서 NEW public API의 한도 거부/다음날 reset/zero extra GET와
  concrete timeout/no retry/redaction을 검증한다. source 전체 재설계·다른 파일 변경은 없다.
  51cd791은 보완 검증 전 미통합으로 유지하고 B2b는 이후 시작한다.

### WP-3B2a 통합 및 B2b 실행 경계

- coordinator가 보완 `5b393bc134fc78a11d871793efb95706c0b7822c`의 두 파일 전체 diff를
  읽었다. source는 transport-local 504 분류 세 줄뿐이며 새 public API의 5000번째 예약,
  5001번째 거부/eligibility 0회/다음날 reset/rollback과 timeout/raw-504 구별을 추가했다.
  이전 coordinator budget 테스트도 유지됐다. worker 재현은 수정 전 9 pass/1 fail,
  수정 후 11 pass였다. package all-targets 최종 보고는 215 pass이나 중간 로그에 225가
  있어 그 합계를 독립 수락 증거로 쓰지 않는다.
- 수락 원본 `51cd791` → `416e51f`, 보완 `5b393bc` → `06b5092`를 기능 브랜치에 통합했다.
  네 파일 모두 worker HEAD와 동일함을 확인했고 coordinator가 통합 트리에서
  `CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true cargo test --locked --offline -p kis-client
  --test intraday_attempt -- --test-threads=1`을 실행해 11/11 pass(5.10s)를 확인했다.
- B2b는 기존 intermediate/medium 분류, Codex luna/max를 유지한다. 동결된 session,
  fairness, producer, retry 정책의 연결이며 가짜 clock/transport와 격리 DB로 검증 가능하다.
  EOD lifecycle 변경, 새 외부 계약/권한/schema/dependency 또는 반복 검증 실패는 재분류·상향
  신호다. 아래에서 명시하지 않은 요구가 빠졌거나 모순이면 만들지 말고 보고한다.
- 소유: 신규 `data-pipelines/collectors/src/intraday_quotes.rs`와 lib module 등록;
  신규 `crates/job-queue/src/owner_equity_v2/intraday_producer.rs`와 상위 module 등록;
  B1 `intraday.rs`의 필요한 typed integration/query; runner quote-loop wiring;
  `runtime.rs`는 기존 adapter가 이미 소유한 Arc reader를 clone하는 accessor만 허용한다.
  EOD 실행/lease/Raw 로직 변경은 금지한다. 신규 collectors/tests/intraday_quotes.rs,
  job-queue/tests/intraday_producer.rs 및 기존 신규 intraday DB support 확장만 허용한다.
- pre-launch 확인: B2a 성공 metadata만으로는 B1 record_failure의 실제 reservation 입력을
  만들 수 없다. 이 연결에 한해 kis-client read_coordination.rs/market_data.rs 및 lib exports,
  신규 intraday_attempt.rs 테스트를 순차 소유 범위에 포함한다. 별도 outcome API로 실패에도
  실제 persist된 reservation(date/count/fence)만 전달하고 아직 예약되지 않은 실패는 None으로
  구분한다. 기존 get/get_intraday/get_intraday_attempt 동작은 wrapper로 보존한다. 완료된
  bytes가 없는 오류에 receipt를 만들지 않는다. 성공·실패 모두 임의 reservation/별도 quota 금지.
- 검증된 reservation을 DB context의 non-nil UUID로 연결할 때는 고정 UUIDv5 namespace와
  명시적 owner/producer-holder/실제 reservation fence/date tuple로 상관 ID를 구성한다.
  ID 자체가 권한 증거라는 주장은 금지하며 실제 private metadata가 없는 provider 실패에
  임의 숫자를 넣어 record_failure를 호출하지 않는다. non-attempt skip은 cache success를
  갱신하지 않으며 API의 상태 도출 경계와 분리한다.
- collector는 closed-schema/hash-pinned session-window bytes 검증과 기존 calendar lineage
  조회, 승인 parser/새 guarded attempt를 연결한다. job-queue dependency를 collectors에
  추가하지 않는다. 런타임 파일은 spec의 고정 경로만 사용하며 fixture는 주입된 bytes만 쓴다.
  실제 날짜 entry/운영 config는 생성하지 않는다. calendar/version/batch/current KST/36h,
  half-open window를 호출 직전과 DB publication 직전에 재확인한다. publication은 기존
  blocking locks 이후 fresh DB clock으로 window 종료도 검증하며 기존 fencing을 약화하지 않는다.
- daemon+owner_only에만 독립 task, 기본 off/--once 0 quote task. 동일 Arc reader를 사용하고
  EOD 15분 작업/lease와 독립된 20s producer lease/5s heartbeat/cancellation을 유지한다.
  worker-role active-demand owner enumeration은 기존 세 테이블 범위에서만 수행한다.
  중복 consumer merge, last-attempt/instrument 순서, 5s target/halt60s, busy skip,
  최대3회 GET/매회 guarded eligibility/공유 budget/Retry-After/401 한 번을 준수한다.
- 테스트: fake clock/transport로 no-demand/defaultoff/once/unknown/closed 0 calls,
  regular/special/open-close/rollover/lineage/hash invalid, 1/5 fairness/duplicate/halt,
  retry accounting/deadline/429/401/EOD contention; 실제 QA DB로 demand expiry,
  producer takeover/disable/generation/session-close 중 inflight discard, 실패 last-good 보존,
  기본 B1 24개 회귀. 운영 main/binary를 실제 credentials로 실행하지 않는다.
- 격리 QA 컨테이너 `lagrange-intraday-qa-20260908-qa-db-1`가 running/healthy이며
  127.0.0.1:55438 및 tmpfs임을 다시 read-only 확인했다. synthetic URL은
  `postgres://postgres:lagrange@127.0.0.1:55438/postgres`; worker는 자신이 만든 임시 DB만
  생성/삭제한다. Docker lifecycle은 coordinator만 관리한다. 모든 이전 worker는 idle이다.
- 소유 파일만 commit하고 파일/라인, deviations, 명령/결과, 미해결/미검증(없으면 없음)을
  보고한다. 하나의 compiler/CARGO_BUILD_JOBS=2/locked/offline; 별도 independent review와
  coordinator scope/tree 검증 후 통합한다. migration/Cargo/Compose/API/Web/ops/실제 provider,
  운영DB/root/deploy/main merge/push는 범위 밖이다. 추가 파일 필요 시 먼저 보고한다.
- B2b 작업자 `7f7ba4bf-1cbb-445e-bf6c-5a3eafd09896` (Codex luna/max, auto-review)를
  `/data/worktrees/3puw275b/stock-beta-intraday-producer`, workspace `wks_7ee1494fc4f22bcc`,
  branch `work/stock-beta-intraday-producer-20260908`, base `372cb42`에서 시작했다.
  초기 inspect에서 running 및 pending permission 없음으로 확인했다. 이전 B2a 담당자와
  reviewer는 idle이며 새 작업자의 완료 후 실제 소유 diff/tests를 검토한다.

### WP-3B2b 완료 및 독립 수락 검토

- 구현 작업자는 `7dd9483aadfdbbf29b00394247f14a9a0ab184ef` (parent `372cb42`)로 완료했고
  idle/clean이다. coordinator가 실제 14파일 +2904/-95 범위와 clean tree를 확인했다.
  아직 기능 브랜치에 통합하지 않았다.
- worker 보고: KIS lib133/신규12, collectors3, job-queue lib132, producer DB4,
  보존 B1 DB24, runner7, fmt/diff 및 scoped clippy 통과. collectors의 기존 경고 예외는
  독립 검토에서 원인/기존 여부를 확인해야 하며 무조건적인 lint 통과로 간주하지 않는다.
- coordinator는 runner/runtime accessor diff 전체, session parser 앞부분 및 producer
  실행/재시도 경로를 읽었다. 런타임 quote task 실패는 EOD와 분리됐으나 시작 시
  missing/malformed window가 process FAILURE를 반환하는 경로는 검토 대상으로 남겼다.
  4개 DB 테스트의 제목만으로 실제 fairness/retry/취소 경합 전체가 입증되지는 않는다.
- 독립 read-only reviewer `f9652834-45af-425c-8be0-fdfbe2753984` (Codex terra/high)를
  동일 producer workspace에서 시작했고 running을 확인했다. 전체 14파일/증거와 계약을
  검토하며 source는 수정하지 않는다. ACCEPT와 coordinator 확인 전에는 통합하지 않는다.
- worker가 window evidence staleness의 구체적 임계값 누락을 보고했다. calendar 36h와
  동일하다고 추정하지 않는다. 검토 후 별도 계약 판단이 필요하며 운영 활성화 승인은 없다.

### WP-3B2b 첫 검토 반려 및 범위 한정 수정

- 독립 reviewer는 REJECT: High `quote_attempt_eligible`의 producer FOR SHARE 이전 DB clock
  샘플이 잠금 대기 후에도 사용되어 장 마감 후 GET을 허용할 수 있다. Medium runner의
  missing/malformed window 로딩 오류가 EOD 루프 진입 전 process failure를 반환한다.
  coordinator가 두 실제 경로를 읽고 확인했다. reviewer가 재현 절차를 제시했으나 실행한
  failing race test 증거는 없으므로 구현자가 먼저 재현한다.
- 독립 실행 KIS12/collectors3/producerDB4 및 targeted clippy/fmt는 통과했다. 기존
  collectors artifact large_enum_variant는 parent에서 확인됐고 변경 파일의 lint는 통과했다.
  producer 성공 전체 경로, 3회/401/429 재시도, 반복 fairness/halt, 취소/heartbeat loss,
  takeover/disable/generation/close inflight 및 EOD 경합 coverage는 여전히 부족하다.
- 원 구현자 luna/max를 유휴 상태에서 재사용한다. 이번 수정은 intraday.rs final eligibility,
  runner quote startup wiring, 신규 producer tests/기존 신규 support만 소유한다. 잠금 후
  fresh DB clock으로 최종 expiry/window를 확인하고, window 로딩 실패는 quote task만
  비활성화하여 EOD를 유지한다. 실제 SQL 대기를 관찰한 race와 주입식 startup tests를 추가한다.
  B1 24개/producer/runner 회귀를 검증한다. 관련 없는 source 변경이나 전체 coverage 확장은
  이번 패키지에 섞지 않는다. 독립 재검토 후 별도 범위로 나머지 coverage를 보완한다.
- 7dd9483은 미통합 상태다. window evidence age 임계값은 별도 계약 판단으로 남으며
  이번 두 코드 결함 수정을 막지 않는다. 운영 활성화/날짜 evidence 생성은 승인되지 않았다.

### WP-3B2b 범위 한정 수정 완료 및 재검토

- 구현자는 `b34198f9a6910df48927f472e97b766518d9153e` (parent `7dd9483`)로 완료했고
  idle/clean이다. coordinator가 실제 3파일 diff 전체를 읽고 소유 범위를 확인했다.
  producer FOR UPDATE 뒤 eligibility clock을 샘플하고, 주입식 quote startup 오류는
  quote task만 비활성화한다. 원본과 수정 커밋 모두 아직 미통합이다.
- worker 보고: 수정 전 eligibility 2개 실패 및 runner source 회귀 1개 실패;
  수정 후 producer DB6/B1 DB24/runner11 및 job-queue all-target clippy/fmt/diff 통과.
  이 수치는 worker 보고이며 독립 재검증을 요청했다.
- 유휴 reviewer `f9652834-45af-425c-8be0-fdfbe2753984` (Codex terra/high)를 재사용해
  두 결함과 delta만 read-only 재검토한다. 최종 clock 이후 lineage query의 지연 가능성,
  FOR UPDATE 영향, 항상 true인 test-only eod_continues helper의 증거 한계 및 demand-only
  expiry 검증 여부를 확인/반박하도록 요청했다. 결론을 미리 정하지 않는다.
- scoped ACCEPT여도 전체 producer 수락은 아니다. 성공/재시도/fairness/취소/heartbeat/
  takeover/세대 변경/EOD 경합의 별도 coverage gate 및 window age 계약 판단은 남아 있다.

### WP-3B2b bounded ACCEPT 및 coverage 실행 분할

- reviewer는 두 코드 결함에 한정해 ACCEPT했다. 독립 producer DB6/runner11/scoped clippy/
  fmt/diff 통과. lineage 후속 읽기는 정상 동시 실행 경로에서 row lock을 취하지 않는
  MVCC read이며, 운영 중 ACCESS EXCLUSIVE DDL은 허용된 경로가 아니다. coordinator는
  이 한정 판단을 채택한다. 원본/수정 커밋은 전체 gate 전까지 미통합으로 유지한다.
- 남은 테스트를 아래 순서로 분리한다. Execution skill은 계속 `$paseo-delegate`이며
  native subagents는 금지다. 대상은 기존 producer workspace, base `b34198f`다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| B2b-C1 | intermediate | 기존 guarded client/parser/DB를 결합하는 명세 확정 fixture 테스트 | medium | public seam/의존성 부족, 시간 모델 충돌은 보고; 반복 검증 실패 시 한 tier 상향 |
| B2b-C2 | intermediate | 기존 SQL barrier와 daemon에 대한 순서/취소/lease 경합 검증 | medium | 재현 불가 경합/lock-order 설계 필요 시 재분류 |
| B2b-C3 | intermediate | quote startup 뒤 EOD callback 도달을 주입식 harness로 관찰 | medium | 기존 EOD lifecycle 변경 필요 시 중단·scope 재검토 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| B2b-C1 | 1 | intermediate | 실제 guarded reader를 통한 성공·실패·재시도 증거 | 새 job-queue tests/intraday_producer_pipeline.rs, 새 tests/intraday_producer_pipeline_support/mod.rs, 필요 최소 기존 intraday_quotes_support/mod.rs | bounded ACCEPT | Codex luna/max | 테스트 전용 owned commit, 실패 발견 시 정확한 재현 보고 | synthetic QA DB + fake Transport/issuer + 실제 ReadCoordinator/client/parser/producer; scoped lint/fmt |
| B2b-C2 | 2 | intermediate | fairness/halt/cancel/heartbeat/fencing/EOD contention | 기존 tests/intraday_producer.rs 및 C1 support, 필요 최소 기존 quote support | C1 검토 후 상세 brief 확정 | Codex luna/max, 반복 실패 시 terra | 별도 테스트 commit | 실제 관찰 barrier와 순서/횟수 증거, B1 24개 유지 |
| B2b-C3 | 3 | intermediate | 실제 startup/EOD 도달 관찰 | runner quote-startup의 최소 주입식 연결 및 runner tests만 | C2 검토 후 상세 brief 확정 | Codex luna/max | tautological helper 대체 검증 | credentials/운영 main 없이 EOD callback 관찰 |

#### B2b-C1 worker brief / coordinator gates

- cwd `/data/worktrees/3puw275b/stock-beta-intraday-producer`, workspace `wks_7ee1494fc4f22bcc`.
  기존 구현자/reviewer 모두 idle이다. 새 테스트 작성 worker가 단독으로 위 C1 파일만 소유한다.
  기존 guarded API와 실제 disk reservation metadata를 그대로 사용하며 metadata/receipt를
  직접 조립하거나 DB clock과 앞서는 fake receipt를 만들지 않는다. 기존 파일의 테스트를
  옮기거나 약화하지 않는다. Cargo/production source/ledger schema 변경은 허용하지 않는다.
- 성공 응답을 fake Transport에서 complete bytes로 전달하여 실제 client -> parser -> producer
  -> QA cache publication의 값/identity/session/version/receipt와 exact GET/path/TR/J를 검증한다.
  성공 이후 malformed/identity-invalid 응답은 같은 cycle 재시도 없이 last-good을 보존해야 한다.
- 503 후 성공 및 3회 연속 retryable failure의 실제 GET 수/영속 reservation count/fence,
  transport timeout(3초) 및 HTTP 429 Retry-After의 spacing/cooldown/중단을 검증한다.
  401은 실제 shared invalidation과 60초 issue debt를 유지하며, 증거/lease가 다음 시도를
  허용하지 않으면 안전한 중단을 기대한다. 임의로 guard를 완화하여 reissue를 강제하지 않는다.
  HTTP timeout과 transport-shaped timeout의 typed 결과를 구별하고 provider prose는 내보내지 않는다.
- 검증은 합성 loopback QA URL과 per-test DB, 0700/0600 임시 coordinator, fake issuer/Transport만
  사용한다. 현실 DB clock에 맞는 시간 모델을 유지한다. 필요하면 bounded real 5초 대기를 쓰고,
  DB 시간을 멈추지 못하는 paused Tokio 시간으로 lease/receipt 증거를 조작하지 않는다.
- `CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true cargo test --locked --offline -p job-queue
  --test intraday_producer_pipeline -- --test-threads=1`, 기존 producer6/B1DB24 및 scoped
  clippy/fmt/diff를 단일 compiler로 실행한다. 소스 결함 발견 시 실패 테스트/정확한 evidence를
  보고하고 production fix는 coordinator의 새 bounded assignment를 기다린다.
- 보고: full commit/parent, 파일/라인, 명세 차이와 이유, 실행 명령/결과, 실패 재현,
  미해결/후속 및 확인하지 못한 사항(없으면 없음). 독립 검토 후 C2 brief를 확정하며,
  전체 coverage와 window evidence age 판단 전에는 B2b 전체를 통합/수락하지 않는다.
  모든 기존 provider/운영DB/root/배포/merge/push 금지와 QA lifecycle 경계는 유지한다.
- C1 worker `de50de28-cfb0-4e02-a236-d91dd2b8d775` (Codex luna/max, auto-review)를
  위 producer workspace/base `b34198f`에서 새 격리 agent context로 시작했다.
  기존 구현자와 reviewer는 idle이며, 이 테스트 writer만 활성 상태다.

### B2b-C1 완료 및 독립 coverage 검토

- C1 worker는 `18b8b33cff461b3d66c957802b93ddd9cf33aaba` (parent `b34198f`)로 완료했고
  idle/clean이다. coordinator가 새 test627/support378, 총 1005줄의 두 파일 전체를 읽었다.
  기존 source/test/support는 수정되지 않았으며 아직 미통합이다.
- worker 보고: 신규9/9 (40.53초), 기존 producer6/B1DB24 및 strict scoped clippy/fmt 통과.
  dead_code 허용은 재사용한 기존 B1 test-support module import에만 한정됐다.
- 유휴 reviewer `f9652834-45af-425c-8be0-fdfbe2753984` (Codex terra/high)에 C1만 read-only
  검토하도록 요청했다. 실제 client/reservation/parser/DB 연결과 테스트 실행을 확인한다.
  coordinator는 요청 시각의 spacing 단언 부재, 429 stop-only 대비 성공 재시도/cycle 간
  cooldown 증거, receipt byte-arrival 경계, EOD row count의 UPDATE 탐지 한계를 전달했다.
  기존 하위 계층 증거로 충분한지 또는 C1 보완이 필요한지 독립 판단하도록 한다.
- C2/C3 및 window evidence age 판단은 여전히 남아 있으며 전체 B2b 수락/통합을 뜻하지 않는다.
