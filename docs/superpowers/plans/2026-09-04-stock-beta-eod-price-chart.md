Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Stock Beta Koyfin형 EOD 시세·주가 차트 실행 계획

작성일: 2026-09-04
상태: 계획만 작성됨, 실행 미시작

## Goal and boundaries

### 목표

Owner 전용 `/stock-beta` Koyfin형 워크스페이스의 선택 종목 영역에 실제 V2 데이터로 만든
일봉 차트와 최신 종가를 제공한다. 완료 시 다음 조건을 모두 만족해야 한다.

1. 선택된 종목의 최신 완료 EOD 종가, 전 거래일 대비 등락액·등락률, 거래량, 기준일을 보여준다.
2. `Price` 탭을 기본 탭으로 두고 1M/3M/6M/1Y 일봉 캔들, 거래량, SMA20, SMA60을 제공한다.
3. 종목 선택과 기간 변경이 실제 차트에 반영되고, 늦게 도착한 이전 요청이 현재 선택을 덮어쓰지
   않는다.
4. 데스크톱은 `rank / chart / decomposition` 3열 정보 구조를 유지하고, tablet/mobile에서는
   페이지 수평 overflow 없이 재배치한다.
5. 차트와 상태 표시는 기존 widget registry/layout/view-model 경계를 지켜 추후 추가·제거·순서
   변경이 국소적인 설정 변경으로 끝나게 한다. 위젯은 직접 API를 호출하지 않는다.
6. 브라우저는 KIS나 외부 시세 공급자를 호출하지 않고, API가 DB에 게시된 정확한 snapshot의
   admission pin으로 검증한 불변 V2 artifact만 읽는다.
7. Owner-only, original/unadjusted price, read-only research, no-account/no-order 경계를 유지한다.
8. Rust contract·보안 테스트, Web unit·접근성 테스트, provider-free Playwright, production Web build,
   architecture diagram evidence가 모두 통과한다.

### 사용자에게 보이는 가격의 의미

- 이번 단계의 `최신 가격`은 실시간 현재가가 아니라 **가장 최근 게시 snapshot 기준의 완료 EOD
  종가**다.
- UI 문구는 `현재가`를 사용하지 않고 `최신 종가` 또는 `기준일 종가`를 사용한다.
- 등락액은 `기준일 종가 - 직전 관측 거래일 종가`, 등락률은
  `(기준일 종가 - 직전 종가) / 직전 종가`의 소수 비율이다. 프론트의 percent formatter가 이를
  백분율로 표시한다.
- 가격은 `FID_ORG_ADJ_PRC=1`로 수집된 original/unadjusted 값이다. 기업행동 전후 단절이 차트와
  수익률을 왜곡할 수 있다는 경고를 항상 접근 가능하게 둔다.
- 임의 보간, 휴장일 봉 생성, 미래 일자, 중복 일자, 조정주가 추정, 실시간처럼 보이는 애니메이션은
  금지한다.

### 확정 데이터 경로

이 기능은 새 KIS 요청이나 새 시세 제공자를 추가하지 않는다.

```text
기존 owner-equity-v2-runner
  -> 검증된 KIS EOD Raw
  -> 불변 OwnerEquityGenerationCandidate(최대 261 거래일 OHLCV)
  -> artifact manifest hash + DB admission
  -> API가 owner/snapshot/instrument/generation/pin을 DB에서 확인
  -> 동일 manifest hash의 artifact를 read-only로 재검증
  -> 제한된 chart DTO
  -> Next.js page/workspace view-model
  -> presentation-only chart widget
```

현재 V2 artifact에는 필요한 OHLCV가 이미 있지만 API DB에는 신호 요약만 있다. 과거 데이터를
중복 DB 테이블로 복제하지 않고, API container에 V2 artifact root를 read-only로 마운트한 뒤 현재
snapshot row가 가리키는 정확한 manifest만 읽는 방식을 채택한다. 이 방식은 기존 데이터에 대해
별도 live backfill 없이 즉시 동작하며, 파일 경로·source pin·entitlement reference는 응답에 노출하지
않는다.

### API 계약

신규 read-only endpoint:

```http
GET /api/v1/research/owner-beta/equity-universe-v2/signals/instruments/{instrument_id}/chart
    ?snapshot_id={uuid}&range={1m|3m|6m|1y}
```

- `snapshot_id`와 `range`는 필수다. 현재 페이지가 렌더링한 snapshot을 pin하여 refresh 도중 서로
  다른 snapshot의 signal과 chart가 섞이지 않게 한다.
- `{instrument_id}`는 기존 V2와 동일한 정확한 `NNNNNN.KRX` 형식만 허용한다.
- 요청 snapshot은 해당 Owner에게 공개된 published snapshot이어야 하고 instrument는 그 snapshot의
  row에 실제로 존재해야 한다.
- 기간은 snapshot `as_of`를 기준으로 1/3/6/12 calendar month 경계를 계산하고, 실제 관측된
  거래일만 오름차순으로 반환한다. SMA20/60은 기간을 자르기 전 전체 admitted history로 계산한다.
- 응답 봉 수는 261개 이하로 제한한다. 알 수 없는 query, 범위, 비정상 숫자, 날짜 역순·중복,
  `high/low` 불변식 위반, snapshot 기준일 봉 누락은 fail closed다.
- 응답에는 `Cache-Control: no-store`를 설정한다.

응답의 의미 계약:

```json
{
  "snapshot_id": "uuid",
  "instrument_id": "005930.KRX",
  "generation": 3,
  "range": "1y",
  "as_of": "2026-09-03",
  "freshness": "CURRENT",
  "expected_as_of": "2026-09-03",
  "price_semantics": "ORIGINAL_UNADJUSTED",
  "latest": {
    "session_date": "2026-09-03",
    "close": 72100,
    "change": 500,
    "change_rate": 0.0069832402,
    "volume": 12345678
  },
  "bars": [
    {
      "session_date": "2026-09-03",
      "open": 71800,
      "high": 72500,
      "low": 71600,
      "close": 72100,
      "volume": 12345678,
      "sma_20": 71425.5,
      "sma_60": 69882.25
    }
  ],
  "warnings": ["NOT_REALTIME", "CORPORATE_ACTIONS_NOT_ADJUSTED", "RESEARCH_ONLY"]
}
```

- 숫자는 예시일 뿐 fixture 외 production 코드에 하드코딩하지 않는다.
- `sma_20`/`sma_60`은 필요한 선행 관측치가 없을 때만 `null`이다.
- `freshness`는 `CURRENT | STALE | UNVERIFIABLE`의 닫힌 enum이다. 기존 DB-confirmed KRX close를
  네트워크 없이 참고하며, reference가 없으면 `UNVERIFIABLE`이지 `CURRENT`로 추정하지 않는다.
- `expected_as_of`는 reference가 없을 때 `null`이다. snapshot이 reference보다 미래이면 integrity
  failure다.
- artifact의 owner UUID, membership UUID, generation, instrument ID, manifest/content/source pin이 DB
  descriptor와 모두 일치해야 한다. 하나라도 다르면 봉 일부도 반환하지 않는다.
- `404 RESOURCE_NOT_FOUND`: 해당 Owner/snapshot/instrument 조합이 없음.
- `503 OWNER_EQUITY_CHART_UNAVAILABLE`: read-only root 또는 exact artifact가 준비되지 않음.
- `503 OWNER_EQUITY_INTEGRITY_FAILED`: hash, lineage, schema, 날짜/가격 불변식이 맞지 않음.
- provider message, request/response body, credential, account identifier, filesystem path를 오류나 로그에
  넣지 않는다.

### UI/UX 계약

- 기존 `signal-profile` widget을 확장한다. 새로 별도 fetch widget을 만들지 않는다.
- 탭 순서는 `Price`, `Returns`, `Volatility`, `Activity`이며 `Price`가 기본이다. 탭 정의는 중앙
  registry에 두어 항목 추가·제거와 순서 변경이 한 파일에서 가능하게 한다.
- Price header에는 instrument ID, 최신 종가, 등락액·등락률, 거래량, 기준일, EOD/original 상태를
  보여준다.
- 차트는 code-native React/SVG로 구현하고 새 chart dependency를 추가하지 않는다.
  - 일봉 wick/body
  - 하단 volume bars
  - SMA20/SMA60 overlay 및 범례
  - y축 가격, x축 대표 거래일
  - pointer/touch crosshair와 OHLCV tooltip
  - focus 가능한 chart surface, ArrowLeft/ArrowRight/Home/End로 관측치 이동
  - 현재 관측치의 날짜·OHLCV를 텍스트로도 읽을 수 있는 screen-reader/live summary
- 상승/하락은 색뿐 아니라 `+/-`, 레이블, candle shape/tooltip 값으로 구분한다.
- range 버튼은 최소 44px touch target과 명확한 selected/focus state를 갖는다.
- 종목을 바꿔도 선택 range는 유지한다. signal snapshot refresh에서 같은 종목이 남아 있으면 선택도
  유지하고, 사라졌을 때만 첫 row로 이동한다.
- chart initial fetch는 server render에서 기본 종목·1Y를 가져온다. 이후 selection/range fetch는
  workspace/controller가 수행하고 위젯에는 typed view-model만 전달한다.
- 요청 전환은 `AbortController`와 monotonic request token을 함께 사용한다. abort를 지원하지 않는
  test fetcher에서도 이전 응답이 현재 화면을 덮어쓸 수 없어야 한다.
- 상태는 다음처럼 구분한다.
  - 최초/기간/종목 loading: 기존 차트가 있으면 `aria-busy`와 updating 표시를 하되 데이터 의미를
    바꾸지 않는다.
  - 선택 row 없음: empty state.
  - artifact root/파일 미준비: preparing/unavailable state.
  - integrity error: 과거 차트를 숨기는 fail-closed error state.
  - stale: 날짜와 expected date를 함께 표시하는 warning.
  - unverifiable: 최신이라고 추정하지 않고 기준 확인 불가 표시.
  - forbidden/session expiry/network: 기존 인증·오류 경계를 재사용한다.
- 필수 viewport는 375×800, 640×360, 768×1024, 1280×720, 1440×900이며 200% zoom에서도 페이지
  수평 overflow가 없어야 한다.

### 적용 지침과 기준 ref

- 대상 repository: `/data/worktrees/3puw275b/enhanced-pig`
- 계획 작성 시 feature HEAD: `8e92897`
- 계획 작성 시 `main`/`origin/main`: `6f40d2b`
- 두 ref의 code tree는 동일하고 main에는 merge commit 하나만 더 있다. 실행 coordinator는 작업
  시작 직전에 refs와 clean state를 다시 확인하고 **최신 `origin/main`에서 새 Paseo feature
  workspace/branch를 만든다**. 현재 통합 branch에 후속 구현을 직접 쌓지 않는다.
- main worktree `/data/workspace/lagrange`의 미추적 `CLAUDE.md`와
  `docs/kis_openapi_entiredocs_20260818_030007.xlsx`는 사용자 소유다. 읽기 외 변경·stage·commit 금지다.
- 모든 worker는 repository root `AGENTS.md`를 먼저 읽고, Web 코드 worker는 코드 수정 전에 설치된
  Next 16.3 문서를 읽는다. hoisted 설치에서는 경로가 `node_modules/next/dist/docs/...`이고 workspace
  설치에서는 `apps/web/node_modules/next/dist/docs/...`일 수 있다.
- Web worker 필수 문서:
  - `01-app/01-getting-started/05-server-and-client-components.md`
  - `01-app/03-api-reference/04-functions/fetch.md`
  - `01-app/02-guides/data-security.md`
  - 테스트 worker는 추가로 `01-app/02-guides/testing/vitest.md`와
    `01-app/02-guides/testing/playwright.md`

### 안전 경계

- KIS 허용 surface는 기존 token manager와 `GET inquire-daily-itemchartprice` /
  `GET inquire-price`뿐이며, 이번 작업에서는 둘 다 새로 호출하지 않는다.
- account, CANO, balance, order, correction/cancellation, WebSocket, live profile, trading UI는 전부
  범위 밖이다.
- OpenDART, KIND, data.go.kr, KRX 또는 두 번째 price provider 호출을 추가하지 않는다.
- browser/API/Web에 App Key, App Secret, token, KIS response body를 전달하지 않는다.
- V1 `equity-price-signals` endpoint를 fallback으로 호출하지 않는다.
- production image build, release, service restart, runtime profile activation, Tailscale/Funnel 변경은 이
  계획의 구현 범위가 아니다.

### 범위 제외

- 실시간 현재가, 체결가, 호가창, intraday/tick, WebSocket streaming
- 종목 매수·매도·주문 연결 또는 투자 추천/목표가
- 조정주가 계산과 기업행동 자동 보정
- 사용자별 drag-and-drop dashboard persistence
- 다른 제품 메뉴의 UI 재개편
- main 병합, origin push, production 배포. 검증된 feature branch까지 만든 뒤 별도 사용자 지시에
  따라 수행한다.

### 미해결 요구사항

없음. 실시간 대신 완료 EOD, V2 admitted artifact 재사용, native SVG chart, Owner-only read-only
경계로 확정한다. 실행 중 현재 artifact contract로 위 lineage를 증명할 수 없으면 다른 데이터 경로를
추측하지 말고 blocker로 반환한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | Owner/RLS DB descriptor, immutable filesystem verification, runtime mount, HTTP/OpenAPI 계약을 한 보안 경계로 연결한다. | high | artifact reader가 API read-only mount에서 안전하게 동작하지 않음, lineage pin 일부가 DB에서 조회 불가, 같은 구현/테스트 실패 2회면 terra medium으로 상향한다. |
| WP-2 | intermediate | 261개 봉 SVG, pointer/touch/keyboard interaction, 반응형 scale을 구현하지만 입력 계약과 파일 범위가 고정돼 있다. | high | 261봉 성능 또는 좌표/접근성 테스트가 2회 실패하면 terra medium으로 상향한다. |
| WP-3 | intermediate | 기존 server/client API 계층과 selection state에 snapshot-pinned fetch 및 race 방지를 추가한다. | high | controlled selection이 polling/disable 흐름과 충돌하거나 stale response 재현 테스트가 반복 실패하면 terra medium으로 상향한다. |
| WP-4 | intermediate | 기존 signal-profile widget과 registry/layout/i18n에 확정된 chart view-model을 연결하는 국소 UI 통합이다. | high | dashboard 구조 변경이 다른 widget contract를 깨거나 shared shell 수정이 필요하면 hard로 재분류한다. |
| WP-5 | intermediate | contract/client/dashboard/accessibility 회귀를 deterministic Vitest로 고정한다. | high | production 코드 결함을 발견하면 직접 우회하지 않고 owning WP로 반송한다. assertion 실패가 2회 반복되면 terra medium으로 상향한다. |
| WP-6 | intermediate | mutable synthetic API에서 selection/range/race/error/responsive 동작을 실제 브라우저로 검증한다. | high | fixture state 누출, polling race, flake 2회면 terra medium으로 상향한다. |
| WP-7 | intermediate | 새 static dependency와 runtime artifact edge를 evidence-bound `.puml` 및 PNG에 정확히 반영한다. | high | 실제 code edge/line evidence가 확정되지 않거나 PlantUML render가 실패하면 coordinator에게 반환한다. |
| WP-8 | hard | 보안·의미·접근성·회귀에 대한 독립 결론이 산출물이며 artifact/DB/UI 전체 추적이 필요하다. | high | source-of-truth 충돌 또는 검증 증거 부족 시 sol high 자문으로 한 단계 상향한다. |
| WP-9 | intermediate | 전체 정적·동적 QA는 명령이 결정적이지만 Rust, Web, Playwright와 시각 판정 결과를 함께 분류한다. | high | 코드 원인 실패나 불명확한 visual anomaly가 나오면 owning WP remediation으로 분리한다. |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | snapshot-pinned, artifact-verified chart API와 runtime contract 구현 | Rust API/repo/runtime, API Cargo, compose, OpenAPI, Rust tests | 없음 | `$paseo-delegate`: gpt-5.6-luna, max; 2회 실패 또는 lineage 불명확 시 gpt-5.6-terra medium | Owner-only chart endpoint와 generated contract | focused Rust tests, OpenAPI exactness, clippy/fmt |
| WP-2 | 1 | intermediate | API 호출 없는 reusable SVG candlestick renderer 구현 | 신규 `apps/web/components/stock-beta/chart/**`와 renderer 전용 test | 없음 | `$paseo-delegate`: gpt-5.6-luna, max; 반복 실패 시 terra medium | typed presentational chart primitive | focused Vitest, typecheck, Biome |
| WP-3 | 2 | intermediate | Web contract/client/server initial load와 race-safe chart state 구현 | Web product contracts/client, product client, page, workspace, selection/coordinator, 전용 tests | WP-1 | `$paseo-delegate`: gpt-5.6-luna, max; 반복 race 실패 시 terra medium | widget-independent chart view-model orchestration | focused Vitest, typecheck, no-V1 request scan |
| WP-7 | 2 | intermediate | 변경된 static/runtime 구조를 evidence-bound diagram에 반영 | 두 `.puml`과 두 PNG만 | WP-1 | `$paseo-delegate`: gpt-5.6-luna, max | 최신 evidence citation과 local render | evidence line check, Docker PlantUML render |
| WP-4 | 3 | intermediate | Koyfin형 Price 탭과 latest-price UI를 기존 widget architecture에 통합 | profile widget, profile-tab registry, dashboard layout/CSS, i18n, Stock Beta README | WP-2, WP-3 | `$paseo-delegate`: gpt-5.6-luna, max; 구조 회귀 시 terra medium | 실제 데이터 기반 Price-first signal profile | focused UI tests, typecheck, lint, 5 viewports |
| WP-5 | 4 | intermediate | Web unit/contract/accessibility 회귀 고정 | 지정 Stock Beta Vitest files만 | WP-4 | `$paseo-delegate`: gpt-5.6-luna, max; production 결함은 owning WP로 반송 | deterministic unit/ARIA/race coverage | focused then full Vitest |
| WP-6 | 4 | intermediate | provider-free 브라우저 기능·반응형 QA 자동화 | Stock Beta E2E spec/fixture/synthetic dispatch만 | WP-4 | `$paseo-delegate`: gpt-5.6-luna, max; flake 2회 시 terra medium | synthetic OHLCV scenarios와 Playwright coverage | focused E2E 2회 연속 |
| WP-8 | 5 | hard | 통합 결과의 독립 read-only acceptance review | 전체 diff/contract/test evidence, 파일 수정 금지 | WP-1~WP-7 | `$paseo-delegate`: gpt-5.6-terra, high | severity별 findings와 ACCEPT/REJECT | static lineage trace와 test evidence 대조 |
| WP-9 | 5 | intermediate | 전체 Rust/Web/build/E2E/visual 최종 QA | repository read-only, test artifacts만 허용 | WP-1~WP-7 | `$paseo-delegate`: gpt-5.6-terra, medium | 명령별 PASS/FAIL, screenshot/overflow 결과 | final verification matrix |

Wave 1의 WP-1과 WP-2는 Rust/runtime 계약과 신규 presentation-only chart directory로 scope가 겹치지
않는다. Wave 2의 WP-3과 WP-7도 Web orchestration과 diagram 파일만 각각 소유한다. Wave 4의
WP-5와 WP-6는 unit test와 E2E fixture/spec으로 분리한다. Wave 5는 두 package 모두 read-only다.

각 package는 반드시 `$paseo-delegate`로 실행한다. native subagent, Task/Agent/team 도구 또는 다른
delegation mechanism으로 worker package를 대체하지 않는다.

## Worker briefs

### WP-1 — artifact-verified Owner Equity V2 chart API

- **Target working directory:** coordinator가 최신 `origin/main`에서 만든 전용 Paseo feature workspace.
- **Initial complexity:** hard, confidence high.
- **Objective:** exact published snapshot admission이 가리키는 V2 candidate artifact에서만 EOD chart
  DTO를 생성한다.
- **Owned scope:**
  - `crates/api-server/Cargo.toml`
  - `crates/api-server/src/http/state.rs`
  - `crates/api-server/src/runtime.rs`
  - `crates/api-server/src/repos/owner_equity_v2.rs`
  - `crates/api-server/src/http/owner_equity_v2.rs`
  - `crates/api-server/src/http/mod.rs`
  - `crates/api-server/src/contract.rs`
  - `crates/api-server/tests/common/mod.rs`
  - `crates/api-server/tests/owner_equity_v2_db.rs`
  - 신규 `crates/api-server/tests/http_owner_equity_v2_chart.rs`
  - `crates/api-server/tests/openapi_contract.rs`
  - `apps/api-server/scripts/openapi-spec.mjs`
  - generated `apps/api-server/openapi.json`, `apps/api-server/generated/openapi.ts`
  - `deploy/compose/compose.yml`의 api-server 환경/mount 구간만
- **Required implementation:**
  - `collectors`의 기존 verified artifact reader를 production dependency로 사용한다. 검증 로직을
    느슨하게 복제하지 않는다.
  - API config에 optional absolute `OWNER_EQUITY_V2_API_ARTIFACT_ROOT`를 추가한다. 미설정은 다른 V2
    read/mutation route를 깨지 않고 chart route만 typed unavailable로 만든다.
  - compose api-server에 `/data/owner-equity-v2-artifacts:ro` mount와 고정 in-container env를 추가한다.
    KIS secret, Raw root, account 정보는 추가하지 않는다.
  - bounded semaphore + `spawn_blocking`으로 filesystem read를 수행한다. route handler에서 blocking I/O를
    직접 실행하지 않는다.
  - repo는 actor-scoped transaction으로 requested published snapshot row와 admission descriptor를 한 번에
    읽는다. descriptor에는 owner, membership, generation ID/number, instrument, artifact/raw/entitlement
    hash와 capture/materializer commit이 포함된다.
  - artifact reader 결과와 descriptor의 모든 lineage field를 exact compare한다.
  - snapshot `as_of`보다 미래 봉을 응답에서 제외하고 exact `as_of` 봉 및 직전 관측 봉을 요구한다.
  - 전체 as-of history에서 SMA를 계산한 뒤 calendar range를 자른다. non-finite 계산은 실패한다.
  - latest/change/freshness/warning을 위 계약대로 구성하고 response DTO 외 field를 serialize하지 않는다.
  - route inventory, OpenAPI metadata/auth/cache/errors, generated TS를 함께 갱신한다.
- **Negative tests:** member/anonymous forbidden; 다른 owner snapshot; invalid ID/UUID/range/unknown query;
  absent root/artifact; wrong manifest hash; owner/membership/generation/instrument/pin mismatch; tampered bytes;
  future/duplicate/reversed bars; bad OHLC; missing as-of; previous close zero; no provider call; no secret/path/prose
  in body/log-safe error.
- **Positive tests:** 1m/3m/6m/1y calendar slicing; ascending unique dates; 261 cap; SMA warm-up across range;
  signed change/rate; CURRENT/STALE/UNVERIFIABLE; no-store; exact snapshot pin.
- **Prohibited adjacent work:** migration/DB bar duplication, worker collection change, new KIS/OpenDART call,
  V1 endpoint change, order/account code, production service start/restart, diagram edits.
- **Verification:** `cargo fmt --all -- --check`; focused repo/HTTP/OpenAPI tests; `cargo clippy -p api-server
  --all-targets -- -D warnings`; `git diff --check`; generated OpenAPI clean rerun.
- **Escalation:** same defect twice, read-only permission contract incompatibility, missing DB lineage field, or API
  dependency cycle면 추측하지 말고 coordinator에 evidence와 함께 반환하여 terra medium으로 재실행한다.
- **Required report:** 변경 파일·라인 범위; 명세와 다르게 처리한 부분과 이유; 명령별 결과; 미해결/후속;
  찾지 못했거나 확인하지 못한 것. 없으면 각 항목에 `none`.

### WP-2 — presentation-only SVG price chart primitive

- **Target working directory:** Wave 1 전용 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** network와 product state를 모르는 재사용 가능한 candlestick/volume/SMA renderer를 만든다.
- **Owned scope:**
  - 신규 `apps/web/components/stock-beta/chart/price-chart.tsx`
  - 신규 `apps/web/components/stock-beta/chart/price-chart.module.css`
  - 신규 `apps/web/components/stock-beta/chart/geometry.ts`
  - 신규 `apps/web/components/stock-beta/chart/types.ts`
  - 신규 `apps/web/components/stock-beta/chart/index.ts`
  - 신규 `apps/web/tests/stock-beta-price-chart-renderer.test.tsx`
- **Required implementation:** pure finite-value validation/scale helpers; SVG viewBox; candle wick/body; volume;
  SMA polylines with gaps for null; sparse date/price axes; pointer/touch nearest-index selection; keyboard
  Arrow/Home/End; visible focus; tooltip and text summary; resize without layout shift; reduced-motion/forced-colors;
  stable test IDs limited to component contract.
- **Input contract:** sorted bars and locale/formatted labels are props. renderer must not import API clients,
  selection provider, workspace, V1 types, `fetch`, or hardcoded production data.
- **Prohibited adjacent work:** dashboard/widget/i18n/registry files, package dependency 추가, canvas bitmap,
  image generation, production API calls.
- **Verification:** geometry unit cases including flat prices/zero volume/null SMA/261 bars; interaction/ARIA test;
  focused Vitest; Web typecheck; owned Biome; `git diff --check`.
- **Escalation:** 261 bars render budget, touch coordinate mapping, keyboard semantics가 2회 실패하면 terra
  medium 재실행을 요청한다.
- **Required report:** 기본 보고 형식 5개 항목을 모두 채운다.

### WP-3 — snapshot-pinned Web data orchestration

- **Target working directory:** accepted WP-1 commit 기반 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** server initial chart와 client selection/range refresh를 strict contract로 연결하되 fetch는
  workspace/controller에만 둔다.
- **Owned scope:**
  - `apps/web/lib/products/equity-signals-contracts.ts`
  - `apps/web/lib/products/equity-signals-client.ts`
  - `apps/web/lib/api/product-client.ts`
  - `apps/web/app/(authenticated)/stock-beta/page.tsx`
  - `apps/web/components/stock-beta/stock-beta-workspace.tsx`
  - `apps/web/components/stock-beta/dashboard/selection-provider.tsx`
  - `apps/web/components/stock-beta/dashboard/types.ts`
  - 신규 `apps/web/components/stock-beta/chart-load-coordinator.ts`
  - 신규 `apps/web/tests/stock-beta-chart-contract.test.ts`
  - 신규 `apps/web/tests/stock-beta-chart-coordinator.test.ts`
- **Required implementation:**
  - Zod `strict()` schema와 closed enum; safe integer/nonnegative OHLCV; ISO date; finite derived values;
    ascending unique bars; snapshot/instrument/as-of exactness.
  - URL builder는 ID, snapshot UUID, range를 encode하며 V2 chart path만 사용한다.
  - browser client는 optional `AbortSignal`을 전달하고 `cache: no-store`, same-origin credentials를 유지한다.
  - server product client에도 같은 typed method를 추가한다.
  - page는 memberships/signals 다음 기본 selected row와 snapshot ID로 1Y chart를 가져온다. typed chart
    unavailable/integrity를 전체 페이지 실패와 구분하여 widget state로 전달한다.
  - workspace가 selected instrument, range, chart data/state/error를 소유한다. 기존 membership polling,
    disable stale-removal, signal refresh 동작을 보존한다.
  - selection provider는 검색 결과와 외부 controlled selection이 불일치하지 않게 하며 실제 selection
    change를 workspace에 통지한다.
  - abort + monotonically increasing token으로 old selection/range/snapshot response를 폐기한다.
  - 현재 signal snapshot/generation과 response가 다르면 값을 표시하지 않고 typed stale/integrity state로
    처리한다.
- **Prohibited adjacent work:** chart SVG/widget/CSS/i18n/E2E, backend, V1 fallback, synthetic 값 생성.
- **Verification:** schema negative cases; URL exactness; initial load; rapid A→B and 1Y→1M out-of-order;
  abort-ignorant fetcher; snapshot refresh; disable; unmount; focused Vitest; typecheck; Biome.
- **Required report:** 기본 보고 형식 5개 항목을 모두 채운다.

### WP-7 — evidence-bound architecture diagrams

- **Target working directory:** accepted WP-1 commit 기반 별도 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** API가 collectors artifact reader에 의존하는 static edge와 V2 artifact를 read-only로 읽는
  runtime edge를 현재 line evidence로 기록한다.
- **Owned scope:**
  - `docs/diagrams/component_architecture.puml`
  - `docs/diagrams/component_architecture.png`
  - `docs/diagrams/runtime_deployment.puml`
  - `docs/diagrams/runtime_deployment.png`
- **Required implementation:** component diagram에 `api-server -> collectors` 실제 dependency/use evidence;
  runtime diagram에 `owner-equity-v2-artifacts -> api-server : ro verified read` compose/handler evidence;
  header의 folded-edge 설명과 날짜/line citation 갱신. 존재하지 않는 edge를 그리지 않는다.
- **Prohibited adjacent work:** production/config/code/test 수정, network renderer 사용.
- **Verification:** 모든 신규/이동 citation을 실제 파일 line과 대조한 뒤 repository 지침의 local Docker
  PlantUML command로 두 PNG를 재생성한다. PNG와 source가 함께 diff에 있어야 한다.
- **Required report:** 기본 보고 형식 5개 항목을 모두 채운다.

### WP-4 — Koyfin형 Price-first signal profile 통합

- **Target working directory:** accepted WP-2와 WP-3 commit이 합쳐진 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** 기존 metric 탭을 유지하면서 실제 latest price/chart를 default 분석 surface로 만든다.
- **Owned scope:**
  - `apps/web/components/stock-beta/dashboard/widgets/signal-preview-widget.tsx`
  - 신규 `apps/web/components/stock-beta/dashboard/profile-tab-registry.ts`
  - `apps/web/components/stock-beta/dashboard/widget-registry.ts`
  - `apps/web/components/stock-beta/dashboard/dashboard-layout.ts`
  - `apps/web/components/stock-beta/dashboard/dashboard.module.css`
  - `apps/web/lib/i18n/dictionaries/stock-beta.ts`
  - `apps/web/components/stock-beta/README.md`
- **Required implementation:** Price default tab; latest price strip; range control; WP-2 renderer; loading/
  empty/preparing/integrity/network/stale/unverifiable states; existing Returns/Volatility/Activity preservation;
  chart widget size/layout metadata; registry-driven tabs; exact Korean/English EOD/original/corporate-action copy;
  3-column desktop composition and responsive stacking.
- **Architecture rule:** widget receives `viewModel.chart` and callbacks only. `fetch`, URL, Zod schema,
  AbortController를 import하지 않는다. tab registry is serializable except component renderer reference where
  existing widget definition pattern permits it.
- **Visual contract:** existing Koyfin palette/typography/token을 재사용한다. 과도한 card radius/shadow,
  별도 브랜드 색상, 실시간 pulse를 추가하지 않는다. candle up/down, volume, SMA legend는 dense하지만
  legible해야 한다.
- **Prohibited adjacent work:** API/client/page/selection/E2E/unit test files, detail page redesign, shared global
  product CSS, 다른 메뉴.
- **Verification:** focused component test if present; typecheck; lint; production Web build; 5 viewports와
  200% zoom manual/screenshot inspection; keyboard/touch smoke.
- **Required report:** 기본 보고 형식 5개 항목을 모두 채운다.

### WP-5 — Web unit·contract·accessibility regression

- **Target working directory:** accepted WP-4 commit 기반 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** UI와 orchestration의 의미 계약을 production code 수정 없이 회귀 테스트로 고정한다.
- **Owned scope:**
  - `apps/web/tests/stock-beta-dashboard.test.tsx`
  - `apps/web/tests/stock-beta-widget-architecture.test.tsx`
  - `apps/web/tests/owner-beta-equity-signals-surface.test.tsx`
  - `apps/web/tests/accessibility.test.tsx`의 Stock Beta cases만
  - 필요 시 신규 `apps/web/tests/stock-beta-chart-surface.test.tsx`
- **Must cover:** Price default; exact latest close/change/rate/volume/as-of; 4 tabs; range callback; all chart
  states; no past price after integrity/forbidden; selected row propagation; registry add/remove/order invariant;
  one main/one h1; tab roles; keyboard chart summary; non-color-only direction; 0/31/100 rows; Korean/English.
- **Prohibited adjacent work:** production code, renderer 전용 test, coordinator 전용 test, E2E fixture/spec,
  assertion 완화로 결함 은폐.
- **Verification:** owned focused Vitest, 관련 Stock Beta 전체 Vitest, full `npm --prefix apps/web run test`,
  typecheck, lint.
- **Failure routing:** production 결함은 파일과 재현을 적어 WP-2/WP-3/WP-4 중 소유 package에 반송한다.
- **Required report:** 기본 보고 형식 5개 항목과 정확한 pass/fail test 수.

### WP-6 — provider-free functional Playwright

- **Target working directory:** accepted WP-4 commit 기반 WP-5와 분리된 Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** 실제 브라우저에서 차트 기능, race 방지, 상태, responsive layout을 검증한다.
- **Owned scope:**
  - `apps/web/tests/e2e/stock-beta.spec.ts`
  - `apps/web/tests/e2e/support/stock-beta-fixture.mjs`
  - chart dispatch에 꼭 필요한 범위의 `apps/web/tests/e2e/support/synthetic-api.mjs`
- **Fixture contract:** deterministic 261-session OHLCV; weekend gaps; SMA null warm-up; positive/negative/flat;
  volume zero; stale/unverifiable; delayed A/B and range responses; unavailable/integrity/404/403. 각 test reset,
  secret/provider prose 없음.
- **Must cover:** initial 1Y chart/latest price; 1M/3M/6M/1Y; row and search selection; rapid A→B late A
  discard; rapid range change; signal refresh snapshot pin; pointer tooltip; touch; keyboard arrows/Home/End;
  Returns/Volatility/Activity preservation; member/expired session; unavailable/integrity fail-closed;
  375×800, 640×360, 768×1024, 1280×720, 1440×900, 200% zoom; no horizontal overflow.
- **Network assertion:** external requests 0, KIS/OpenDART requests 0, V1 `equity-price-signals` requests 0,
  account/order/live routes 0. only app origin and synthetic API origin allowed.
- **Prohibited adjacent work:** production/unit files, arbitrary sleep, test-only production branch, schema 완화.
- **Verification:** focused Stock Beta spec를 clean fixture로 2회 연속 통과; 포트/child process 정리; failure
  시 screenshot/trace path 보고.
- **Required report:** 기본 보고 형식 5개 항목과 각 run의 pass/fail/time.

### WP-8 — independent semantic/security/accessibility review

- **Target working directory:** 모든 implementation/test/diagram commit이 통합된 read-only Paseo workspace.
- **Initial complexity:** hard, confidence high.
- **Objective:** 구현자의 의도와 별개로 source-to-screen lineage와 UX 요구를 검증하고 ACCEPT/REJECT를
  내린다.
- **Owned scope:** 파일 수정 금지. 전체 integration diff, relevant source/tests/docs read-only.
- **Review checklist:** exact owner/snapshot/instrument/generation/artifact binding; read-only mount; no provider;
  no secrets/path/source pins in DTO/error; fail-closed tamper; original/unadjusted wording; EOD not realtime;
  change/rate/SMA correctness; range boundary; no stale response overwrite; no V1 fallback; widget no-fetch;
  registry extension; keyboard/touch/forced-colors/reduced-motion; responsive overflow; OpenAPI/router exactness;
  diagram evidence currentness.
- **Output:** findings ordered `critical/high/medium/low`, each with file:line, impact, reproduction/evidence,
  required fix owner WP. 마지막에 ACCEPT 또는 REJECT. 찾지 못한 영역도 명시한다.
- **Escalation:** source contracts conflict or evidence insufficient이면 sol high 자문 package로 한 단계만
  상향하고 같은 질문을 반복하지 않는다.

### WP-9 — full final QA and visual acceptance

- **Target working directory:** WP-8과 동일 integration commit의 read-only Paseo workspace.
- **Initial complexity:** intermediate, confidence high.
- **Objective:** merge-ready 여부를 명령과 브라우저 증거로 판정한다.
- **Owned scope:** source 수정 금지. test output, Playwright trace/screenshot 같은 ignored artifact만 허용.
- **Required commands/checks:**
  1. `git diff --check` 및 conflict marker/secret/account/order/live-call scan.
  2. `cargo fmt --all -- --check`.
  3. WP-1 focused Rust repo/HTTP/OpenAPI tests.
  4. `cargo clippy -p api-server --all-targets -- -D warnings`.
  5. `npm --prefix apps/web run typecheck`.
  6. `npm --prefix apps/web run lint`.
  7. `npm --prefix apps/web run test`.
  8. `API_INTERNAL_URL=<provider-free fixture> npm --prefix apps/web run build`.
  9. focused Stock Beta Playwright 2회와 가능한 범위의 full Web E2E.
  10. 5개 viewport, Korean/English, pointer/touch/keyboard, 200% zoom, forced-colors, reduced-motion visual
      inspection.
  11. PlantUML sources/PNGs와 evidence lines 확인.
- **Production resource policy:** 이 QA는 production image build가 아니다. release image가 이후 별도
  요청되면 2~3 service logical batch, Compose 한 service씩, `COMPOSE_PARALLEL_LIMIT=1`,
  `CARGO_BUILD_JOBS=2`, background systemd/low priority, batch 사이 memory/swap/OOM/health 확인을 적용한다.
- **Output:** command, exit code, pass/fail count, duration, skipped reason, environment-vs-code 판정,
  viewport별 결과를 표로 보고하고 최종 ACCEPT/REJECT를 낸다.

## Coordinator gates

### Gate 0 — launch hygiene

1. `git fetch`는 실행 환경에서 허용될 때만 수행하고 `origin/main` SHA를 기록한다.
2. 최신 `origin/main`에서 전용 feature branch/workspace를 생성한다.
3. tracked clean state와 user-owned untracked files 비접촉을 확인한다.
4. 모든 WP prompt에 절대 workspace path, base SHA, owned scope, 금지 범위, 기본 보고 형식 5개를 넣는다.
5. 각 WP를 `$paseo-delegate`로만 시작한다.

### Gate 1 — Wave 1 acceptance

- WP-1의 Rust/OpenAPI negative tests가 통과하고, API 응답에 artifact/source/entitlement/path가 없음을
  coordinator가 직접 diff로 확인한다.
- WP-2 renderer에 `fetch`/API/selection dependency가 없고 261-bar/keyboard tests가 통과해야 한다.
- 둘 중 하나라도 실패하면 Wave 2로 가지 않는다. 동일 실패 2회부터 지정된 모델 상향 규칙을 적용한다.

### Gate 2 — orchestration and structure

- WP-3의 out-of-order/abort-ignorant tests와 기존 membership polling/disable tests를 함께 통과시킨다.
- WP-7의 두 PNG가 local render 결과이고 모든 신규 edge에 현재 file:line evidence가 있어야 한다.
- API response type과 Web Zod schema field set을 coordinator가 1:1 대조한다.

### Gate 3 — UI integration

- Price가 default이면서 기존 세 metric tab 기능이 남아 있는지 확인한다.
- widget source에 fetch가 없고 registry/layout/profile-tab 설정이 중앙화됐는지 확인한다.
- latest label이 `현재가`가 아니며 EOD/as-of/original/corporate-action 의미가 보이는지 확인한다.
- desktop 3열과 mobile overflow 기본 smoke를 통과해야 Wave 4를 시작한다.

### Gate 4 — tests and remediation loop

- WP-5와 WP-6 결과를 통합한 뒤 production defect는 owning WP에 `$paseo-delegate` follow-up으로만
  반송한다.
- worker가 다른 package 파일을 고치지 못하게 하고, 수정 후 해당 focused suite와 인접 regression을
  다시 실행한다.
- 같은 수정 실패 2회는 `luna -> terra`처럼 한 단계만 상향한다. effort와 model을 동시에 올리지 않는다.
- `미해결 항목 없음`과 `확인하지 못한 것 없음`이 명시되지 않은 worker report는 완료로 받지 않는다.

### Gate 5 — independent acceptance

- WP-8의 critical/high/medium finding은 모두 해결하고 review를 재실행한다. low는 사용자에게 명시적으로
  공개하고 수용 여부를 묻는다.
- WP-9의 필수 command와 focused E2E 2회가 모두 green이어야 한다.
- 환경 문제로 full E2E 일부가 불가능하면 focused provider-free suite, 정확한 blocker, 미실행 범위를
  보고한다. 이를 자동 PASS로 간주하지 않는다.

### Gate 6 — handoff

- 최종 feature commit SHA, 변경 파일, API 의미, 테스트 결과, 남은 risk를 요약한다.
- 사용자가 직접 볼 수 있는 demo는 별도 지시가 있을 때만 기존 HTTPS/Tailscale 8443 경로로
  활성화한다. localhost URL을 사용자 검수 링크로 제시하지 않는다.
- main merge, origin push, release/deploy/restart는 자동 수행하지 않는다. 각각 사용자 지시 후 현재
  branch/remote/production 상태를 다시 검증한다.

## Definition of done

- 실제 V2 admitted EOD data가 선택 종목 Price 탭과 최신 종가에 표시된다.
- API/DTO/Web schema가 exact하고 Owner-only이며 tamper와 lineage mismatch에 fail closed다.
- selection/range/snapshot race가 deterministic test와 실제 browser test에서 재현·차단된다.
- chart는 mouse, touch, keyboard, screen reader에서 핵심 OHLCV를 확인할 수 있다.
- 5개 viewport와 200% zoom에서 Stock Beta page horizontal overflow가 없다.
- V1, external provider, account/order/live 호출이 0이다.
- widget registry/layout/view-model 규칙과 관련 README가 최신이다.
- component/runtime architecture source와 PNG가 코드 구조를 정확히 반영한다.
- 모든 required test/build/review가 green인 feature branch와 검증 보고서가 준비된다.
