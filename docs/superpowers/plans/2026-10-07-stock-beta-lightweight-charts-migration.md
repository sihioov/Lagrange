Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Stock Beta Lightweight Charts 전환 실행 계획

- 작성일: 2026-10-07
- 상태: WP-1~4 및 coordinator 최종 수용 완료 (2026-10-08 KST)
- 기준 checkout: `27ca8cf9f3c032daff32fb679f53ee54a78fdc96`
- 작업 디렉터리: `/data/worktrees/3puw275b/itchy-bulldog`
- 계획 작성 후 사용자가 `$paseo-delegate` 실행을 요청했다. 아래 패키지를 순차 수행한다.

## Goal and boundaries

### 목표와 완료 기준

Owner `/stock-beta` Price 탭의 직접 구현한 SVG 봉차트를 `lightweight-charts` 5.2.1 기반
Canvas 차트로 전환한다. 기존 일봉의 의미와 접근 가능한 값 조회를 보존하고, 확대·축소와
이동을 제공한다. 다음 조건을 모두 충족해야 구현 완료로 인정한다.

1. 기존 검증된 `chart.bars`의 OHLCV를 그대로 표시하고 서버가 계산한 SMA20/60을 사용한다.
   기준일·종목·snapshot·generation·freshness 및 `ORIGINAL_UNADJUSTED`의 의미가 유지된다.
2. 가격 pane에는 봉과 SMA20/60, 하단 pane에는 거래량을 표시하며 시간축을 공유한다.
3. 기존 1M/3M/6M/1Y 조회와 기본 1Y를 유지한다. 휠 확대·축소, 수평 드래그 이동,
   터치 확대·이동, 전체 조회 범위로 돌아가는 초기화 동작이 실제 브라우저에서 작동한다.
4. 마우스·터치 및 ArrowLeft/ArrowRight/Home/End로 날짜와 OHLCV를 확인할 수 있다.
   키보드로 선택한 봉은 화면 안으로 이동하고, HTML 요약·방향 레이블·스크린리더 안내를 제공한다.
5. 기간을 빠르게 바꾸거나 종목·snapshot이 바뀌어도 늦은 응답이 현재 차트를 덮어쓰지 않는다.
   unavailable/integrity/error에서 이전 Canvas 가격과 tooltip이 남지 않는다.
6. 375×800, 640×360, 768×1024, 1280×720, 1440×900 및 기존 200% zoom 검증에서
   페이지 수평 overflow, 축/툴팁의 심각한 잘림, 포커스 소실이 없다.
7. 데이터 변환·SSR shell·선택 상태 unit test, provider-free 브라우저 회귀, TypeScript,
   Biome, production Web build 및 독립 리뷰를 통과한다. 기존 검증을 단순 삭제해 통과시키지 않는다.
8. 사용자에게 보이는 TradingView 출처 문구와 링크, 해당 버전의 LICENSE/NOTICE가 포함된다.
   추가 JS 번들 크기와 브라우저 초기화/상호작용 관측 결과를 기록하되 성능 향상 수치를 추정하지 않는다.

### 범위

- 대상: `apps/web/components/stock-beta/chart/`, 관련 번역·Price 탭 연결부, npm 의존성,
  관련 unit/E2E test, 출처 파일, 차트 개발 문서.
- 기존 `PriceChartProps`의 bars/copy/locale/formatter aliases/controlled selection/callback 계약을
  유지한다. 초기화 버튼 등에 필요한 copy 항목은 기본값을 갖는 호환 가능한 추가만 허용한다.
- API, Rust, DB, 수집기, artifact, 신호 계산, 서버의 SMA 계산, 인증, 실시간 quote/stream,
  배포·운영 서비스·프로덕션 데이터 접근은 이번 구현 범위 밖이다.
- 차트 런타임은 npm 번들에 포함한 라이브러리와 전달받은 데이터만 사용한다. CDN script,
  외부 시세 API, 차트에서의 별도 fetch, 무한 과거 데이터 조회를 추가하지 않는다.
- 이 계획은 신규 네트워크 권한·외부 게시·merge·배포를 승인하지 않는다. 설치 및 테스트 실행은
  후속 구현 요청의 범위와 실행 환경의 권한을 따른다.

### 적용 지침과 실행 전제

- 사용자가 이 대화에 제공한 AGENTS 지침: coordinator/worker 역할, 모델·effort 배정,
  실패 시 단계적 상향, 파일 소유권, 검증 근거, 필수 보고 형식을 적용한다.
- 저장소 [`AGENTS.md`](../../../AGENTS.md): provider/account/order 금지 경계, 운영 build 제한,
  구조 변경 시 diagram/evidence 갱신 의무를 적용한다. 이 작업에서는 live provider를 호출하지 않는다.
- [`apps/web/AGENTS.md`](../../../apps/web/AGENTS.md)와
  [`apps/web/CLAUDE.md`](../../../apps/web/CLAUDE.md): 설치된 Next 16.3의 관련 로컬 guide를
  코드 작성 전에 읽는다. `CLAUDE.md`는 같은 AGENTS를 참조한다.
- [`Stock Beta README`](../../../apps/web/components/stock-beta/README.md): renderer는 표시만 담당하고,
  fetch/DTO 검증은 page/controller에 남긴다. EOD·원가격·기업행동 주의문을 유지한다.
- [기존 EOD 차트 계획](2026-09-04-stock-beta-eod-price-chart.md)의 데이터·오류·접근성 계약을
  재사용한다. 그 문서의 “React/SVG, 새 chart dependency 없음” 조항만 이 전환 계획의 변경 대상이다.
- 계획 스킬: `/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`.
  실행 스킬 파일은 `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`에서 확인했다.
  실행자는 전체 실행 스킬을 새로 읽고 현재 Paseo MCP와 profile/model/effort 지원을 확인한다.
  스킬/MCP가 없으면 worker 시작 전에 중단하고 전제조건을 보고한다. native agent/CLI로 대체하지 않는다.
- 계획 작성 시 npm 의존성·브라우저 테스트 환경은 준비되지 않았다. Node 24, 루트 npm workspace 설치,
  Chromium, 사용 가능한 loopback 포트, package registry 접근을 실행 전 확인한다.
- 계획을 막는 제품 요구사항은 없다. 아래 UX 기본값은 계획의 제안이며 구현 요청 전에 읽을 수 있게
  명시한다. 실제 번들 증가량, 브라우저 동작, 운영 배포 상태는 아직 검증되지 않았다.

### 구현 기본값과 보존할 계약

**데이터 변환**

- `session_date`는 원래 `YYYY-MM-DD` 문자열을 library의 일자형 `time`으로 전달한다.
  local timezone timestamp로 재해석하지 않는다. 가격·거래량을 반올림·보간·재계산하지 않는다.
- product 계약은 최대 261개, 고유한 날짜 오름차순, 안전한 정수 OHLCV와 nullable SMA를 강제한다.
  근거: `apps/web/lib/products/equity-signals-contracts.ts:637,687`.
- renderer의 독립 입력에는 기존 유한값/OHLC 관계/음수 거래량 검사를 유지한다. 기존처럼 잘못된
  수치의 봉을 제외하는 경우 원본 `sourceIndex`를 보존하고 해당 위치에서 SMA 연결을 끊는다.
  유효 봉들의 날짜가 잘못됐거나 중복·역순이면 빈 결과와 typed invalid 상태를 반환한다.
  조용한 sort/dedup으로 입력이나 callback index를 바꾸지 않는다.
- SMA는 `null` 또는 제외된 봉을 경계로 연속 구간으로 분리한다. 각 구간은 독립 LineSeries로
  렌더링하여 중간 결측 양쪽을 연결하지 않는다. 선행 warm-up 구간에는 가짜 점을 만들지 않는다.
  `null`을 0/NaN으로 대체하지 않는다. 원본 배열을 변경하지 않는다.
- adapter 출력 계약은 candles, volumes, sma20Segments, sma60Segments, validSourceIndices,
  date↔original-index mapping과 empty/invalid 결과다. API/product 모듈을 adapter에서 import하지 않는다.

**차트와 상호작용**

- 기본 의존성은 정확히 `lightweight-charts@5.2.1`이다. 추가 React wrapper 및 a11y/plugin 패키지는
  초기 범위에 포함하지 않는다. 기존 HTML 접근성 layer를 library API와 연결한다.
- 상승은 현재 초록 채움, 하락은 panel 배경색 몸통+빨간 테두리, 보합은 회색과 명시적 방향 텍스트를
  사용한다. SMA20은 파란 실선, SMA60은 회색 점선이다. 보합봉/거래량의 기존 SVG 점선 테두리는
  Canvas 기본 표현으로 단순화한다. 픽셀 단위의 SVG 복제와 custom candle renderer는 목표가 아니다.
- CSS token의 실제 색상을 읽어 library 옵션에 반영한다. forced-colors에서는 차트 색상도 명시적으로
  갱신한다. HTML의 CSS만 바꾸고 Canvas도 바뀌었다고 간주하지 않는다.
- 첫 표시와 새 bars dataset 로드 시 선택 기간 전체를 한 번 fit한다. 사용자 확대·이동 상태를
  pointer 이동, 선택 상태, locale 변경 같은 일반 rerender마다 초기화하지 않는다.
- 1M/3M/6M/1Y 버튼은 기존 API 조회 의미를 유지한다. 확대·이동은 현재 로드된 범위 안에서만
  수행하며 추가 네트워크 요청을 만들지 않는다. 초기화는 기존 데이터 전체를 다시 보여준다.
- 기본 선택은 최신 관측치다. 기존 controlled/uncontrolled selection과 원본 index callback 의미를
  유지한다. 동일 관측치 위에서 pointer가 움직일 때 중복 callback/전체 setData를 반복하지 않는다.
- 실제 library crosshair event와 일자 mapping을 이용해 원본 봉의 HTML tooltip/summary를 갱신한다.
  keyboard 선택과 controlled selection도 같은 경로에 연결한다. custom labels와 formatter 우선순위를
  보존하고, 표시 문자열을 가격 계산에 역으로 사용하지 않는다.
- touch에서는 세로 페이지 스크롤과 수평 차트 이동을 구분하고, 값 조회와 pinch 동작을 검증한다.
  새로운 chart container 전체를 button으로 만들지 않는다. 포커스 가능한 의미 있는 HTML 영역과
  별도 초기화 버튼을 사용하고 기존 `aria-describedby`, 키 안내, `aria-live` 요약을 유지한다.

**React, SSR, 실패 처리**

- 서버와 첫 hydration은 동일한 크기를 확보한 HTML container·summary·legend를 출력한다.
  library import/생성은 브라우저 mount 이후 수행하고 module evaluation에서 DOM에 접근하지 않는다.
- 한 mount의 chart/series를 ref로 관리한다. 데이터 변경은 setData, 스타일 변경은 applyOptions로
  처리한다. effect cleanup에서 chart, 구독, observer, 예약 frame을 정리한다.
  async import 완료 후 unmount된 인스턴스를 재생성하지 않는다. Strict Mode 재설치를 검증한다.
- 기존 `stock-beta-workspace.tsx`, `chart-load-coordinator.ts`, API 계약 파일은 읽기 전용이다.
  동일 context의 기간 loading 동안 기존 데이터를 보여주는 동작과, 오류 시 이전 차트를 제거하는
  동작을 각각 유지한다. 신호가 준비되지 않은 상태의 안내도 renderer 때문에 사라지면 안 된다.
- core library가 직접 그리지 않는 tooltip·summary·legend와 실제 NOTICE 출처 문구는 React HTML로
  제공한다. TradingView 링크와 locally served LICENSE/NOTICE를 포함하며 저작권 연도를 추정하지 않는다.

### 검토한 공식 근거

2026-10-07에 앞선 분석에서 확인했다. 실행 시 5.2.1에 맞는 문서/타입을 기준으로 한다.

- [5.2.1 release](https://github.com/tradingview/lightweight-charts/releases/tag/v5.2.1)
- [client-side 요구사항과 시리즈 API](https://tradingview.github.io/lightweight-charts/docs)
- [resize/scroll/scale 옵션](https://tradingview.github.io/lightweight-charts/docs/api/interfaces/ChartOptionsBase)
- [panes](https://tradingview.github.io/lightweight-charts/tutorials/how_to/panes)
- [React 생성과 cleanup](https://tradingview.github.io/lightweight-charts/tutorials/react/simple)
- [별도 HTML tooltip](https://tradingview.github.io/lightweight-charts/tutorials/how_to/tooltips)
- [접근성 layer](https://tradingview.github.io/lightweight-charts/tutorials/a11y/intro)
- [line whitespace는 선 끊김을 뜻하지 않는다는 maintainer 설명](https://github.com/tradingview/lightweight-charts/issues/1947)
- [5.2.1 whitespace filtering](https://github.com/tradingview/lightweight-charts/blob/v5.2.1/src/model/data-layer.ts)
- [license/attribution](https://github.com/tradingview/lightweight-charts#license)

## Initial classification

모델을 배정하기 전에 데이터 변환, 브라우저 상태, 검증 가능성으로 난도를 분류했다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | intermediate | 명확한 OHLCV 변환이지만 원본 index·일자 검증·SMA 구간 분리를 함께 보존해야 함. 순수 함수 테스트로 확인 가능 | high | library 타입이 구간 설계와 충돌, mapping/결측 회귀를 같은 수정으로 두 번 해결하지 못함 |
| WP-2 | intermediate | 공개 props를 유지하는 국소 UI 전환. SSR/effect/접근성/resize의 설계는 위에서 확정했으나 여러 파일의 연결이 필요 | medium | Strict Mode 누수나 canvas 잔상이 반복, 접근성 보존에 추가 architecture가 필요, 공개 계약 변경이 필요 |
| WP-3 | intermediate | 알려진 fixture와 Playwright를 이용하지만 Canvas 표시·gesture·lifecycle 검증 방법을 바꿔야 함 | medium | 재현 불안정한 오류가 반복, fixture 모드와 실제 route가 모순, test 환경 제약으로 핵심 동작을 검증하지 못함 |
| WP-4 | intermediate | 완료된 diff와 명확한 수용 기준을 대상으로 하는 한정된 코드 리뷰. 외부 시스템 판단 없음 | high | snapshot/데이터 의미/구조 경계 변경이 발견되거나 검증 증거가 서로 모순됨 |

상향 신호가 생기면 coordinator가 실패 원인과 남은 불확실성을 먼저 정리한다. 같은 수정 실패
2회, 반복·맥락 누락·반복 검증 실패가 관찰되면 한 단계씩 상향한다. 모델과 effort는 같은 시점에
함께 바꾸지 않는다. 모델을 올릴 때는 당시 effort를 유지하고, 지원하지 않는 조합이면 실행 전
계획을 조정한다. 환경 미준비를 모델 능력 문제로 처리하지 않는다.

## Execution graph

모든 행의 실행은 `$paseo-delegate`를 사용한다. native worker와 재위임은 금지한다.
현재 instruction의 구현/테스트 배정에 따라 WP-1~3은 `gpt-6-luna/max`, 코드 리뷰는
`gpt-6.1-sol/high`를 초기 선택한다. 실제 Codex provider와 해당 model/effort의 제공 여부는
실행 시 현재 Paseo profile/capability에서 확인하며, 다른 조합으로 조용히 대체하지 않는다.

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | intermediate | 고정 의존성과 순수 데이터 adapter 확립 | web package.json, 루트 lockfile, 신규 lightweight-data.ts와 전용 unit test | 없음 | `$paseo-delegate`, Codex gpt-6-luna/max | 원본 값/index/결측을 보존한 adapter | adapter Vitest, typecheck, 변경 파일 Biome |
| WP-2 | 2 | intermediate | adapter를 사용하는 Canvas PriceChart와 접근성/출처 구현 | WP-1 파일을 제외한 chart/, 제한된 Price 탭/copy/CSS, renderer·surface tests, LICENSE/NOTICE, chart README | WP-1 수용 | `$paseo-delegate`, Codex gpt-6-luna/max | 기존 contract를 유지한 일봉 차트 | chart unit/SSR·선택 계약, typecheck, Biome |
| WP-3 | 3 | intermediate | 실제 브라우저 회귀를 Canvas 기준으로 전환하고 검증 | stock-beta 관련 E2E 2개, 필요한 chart fixture와 한정된 QA 증거 | WP-2 수용 | `$paseo-delegate`, Codex gpt-6-luna/max | browser 수용 증거, 수정된 E2E, build 결과 | unit 전체, focused Playwright, typecheck, lint, Web build |
| WP-4 | 4 | intermediate | 완료 diff와 증거의 독립 리뷰 | 전체 diff 읽기 전용; 지정 review 문서만 작성 | WP-3 수용 | `$paseo-delegate`, Codex gpt-6.1-sol/high | 근거·심각도·잔여 위험을 포함한 리뷰 | 코드/계약/증거 대조, 필요한 focused 재현 |

이 네 패키지는 선행 산출물에 의존하므로 같은 wave로 병렬 실행하지 않는다. 공유 lockfile과
renderer를 여러 worker가 동시에 수정하지 않는다. 수정 후 재검증은 원래 파일 소유자에게
`$paseo-delegate`로 다시 맡긴다. 문제가 새 설계를 요구하면 해당 branch의 brief와 graph부터 고친다.

## Worker briefs

### WP-1 — 의존성과 순수 데이터 변환

- **작업 디렉터리:** `/data/worktrees/3puw275b/itchy-bulldog`.
- **초기 분류:** intermediate, confidence high. 정해진 adapter와 deterministic unit test 작업이다.
  날짜/index/결측 mapping 오류를 두 번 해결하지 못하거나 의존성 타입과 명세가 충돌하면 보고한다.
- **목적/배경:** 현재 PriceChart 입력은 `chart/types.ts`의 readonly 일봉 배열이다. product가 최대
  261개의 정렬된 일자를 제공하지만 renderer 단독 테스트에는 invalid/flat/zero-volume/null-SMA가
  있다. 원본 값과 callback index를 유지하는 library용 adapter를 만든다.
- **소유 파일:** `apps/web/package.json`, `package-lock.json`,
  `apps/web/components/stock-beta/chart/lightweight-data.ts`(신규),
  `apps/web/tests/stock-beta-lightweight-data.test.ts`(신규)만 수정한다.
- **입력:** 이 계획의 데이터 변환 계약, 기존 `chart/types.ts`, `chart/geometry.ts`,
  `stock-beta-price-chart-renderer.test.tsx`, API schema는 읽기 전용이다.
- **수행:** 루트 workspace에서 정확한 5.2.1만 추가한다. 설치된 Next 관련 guide를 읽고 adapter를
  구현한다. library 타입 import는 허용하되 순수 adapter에서 runtime DOM/library 초기화는 하지 않는다.
  준비된 시리즈 데이터·연속 SMA 구간·원본 mapping을 WP-2가 바로 사용할 수 있게 export한다.
- **금지:** 렌더러·제품/API schema·Rust·기존 lockfile의 무관한 package bump·추가 plugin 변경.
  불명확한 값의 정렬/보정/보간으로 명세를 메우지 않는다.
- **산출물:** adapter, exact dependency/lock, 독립 unit test 및 입력/출력 타입 요약.
- **검증:** `npm install --workspace @lagrange/web --save-exact lightweight-charts@5.2.1`,
  `npm --prefix apps/web test -- tests/stock-beta-lightweight-data.test.ts`,
  `npm --prefix apps/web run typecheck`; 설치된 Biome로 변경 TS 파일을 검사한다.
  정상 261개, 0거래량, 보합, 앞/중간 SMA null, invalid OHLC, 모두 invalid, 잘못된/중복/역순 날짜,
  입력 불변성과 sourceIndex round trip을 확인한다. DOM 없이 adapter를 import할 수 있어야 한다.
- **필수 보고:** 변경 파일과 라인 범위; 명세 이탈과 이유; 실행한 검사와 결과; 미해결/후속 작업;
  찾지 못했거나 검증하지 못한 내용. 빈 항목은 각각 `none`으로 명시한다.
- **위임 제한:** Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

### WP-2 — Canvas renderer와 HTML 접근성 연결

- **작업 디렉터리:** `/data/worktrees/3puw275b/itchy-bulldog`.
- **초기 분류:** intermediate, confidence medium. view contract와 데이터 adapter가 고정되어 있으나
  브라우저 effect/선택/resize가 연결된다. 반복 누수·잔상, public contract 변경 필요,
  a11y 보존 불가가 드러나면 coordinator에 재분해를 요청한다.
- **목적/배경:** 기존 `price-chart.tsx`는 use client이면서 서버에도 SVG와 요약을 렌더링한다.
  Price 탭은 `chart.bars`, locale, copy만 전달한다. WP-1 adapter를 사용해 가격+거래량 두 pane과
  연속 구간별 SMA를 만들고, HTML 요약/tooltip과 원본 index 선택 계약을 연결한다.
- **소유 파일:** `apps/web/components/stock-beta/chart/` 중 WP-1의 `lightweight-data.ts`를 제외한
  파일; `apps/web/tests/stock-beta-price-chart-renderer.test.tsx`,
  `apps/web/tests/stock-beta-chart-surface.test.tsx`;
  `apps/web/lib/i18n/dictionaries/stock-beta.ts`의 차트 copy만;
  `dashboard/profile-tab-registry.ts`의 PriceChart copy 전달부와
  `dashboard/dashboard.module.css`의 priceTab/priceChart 크기 규칙만;
  `apps/web/components/stock-beta/README.md`의 차트 설명;
  `apps/web/public/licenses/lightweight-charts/LICENSE.txt`, `NOTICE.txt`(신규).
  dashboard 경로의 기준은 `apps/web/components/stock-beta/`이다.
- **입력/의존:** WP-1 수용된 adapter/interface, 이 계획의 구현 기본값, 기존 formatter/copy/선택
  동작, 해당 5.2.1 문서와 실제 타입. 모든 target path의 AGENTS와 설치된 Next guide를 읽는다.
- **수행:** browser-only 생성/cleanup, 안정된 chart ref, 데이터/스타일 별도 갱신, resize,
  keyboard와 controlled selection, HTML tooltip/summary/legend, 초기화 버튼, 한·영 문구,
  forced-colors 및 출처 표시를 구현한다. library loading 실패도 이전 가격 잔상 없이 텍스트로 처리한다.
  legacy SVG geometry와 exports는 실제 호출처를 확인한 후 불필요한 렌더링 부분만 정리한다.
  아직 사용되는 selection/validation helper나 공개 props를 근거 없이 삭제하지 않는다.
- **금지:** WP-1 소유 파일의 직접 수정, chart coordinator/workspace/API schema/network 계약 변경,
  새 renderer fetch, SSR 전체 비활성화, 접근성 삭제, 테스트 전용 전역 chart 인스턴스 공개,
  외부 CDN, custom candle engine, 추가 npm plugin. adapter 수정이 필요하면 WP-1로 반환한다.
- **산출물:** 작동하는 Canvas renderer, 호환 HTML 접근성, 출처 파일, renderer 문서와 관련 unit test.
- **검증:** `npm --prefix apps/web test -- tests/stock-beta-price-chart-renderer.test.tsx tests/stock-beta-chart-surface.test.tsx tests/stock-beta-chart-contract.test.ts tests/stock-beta-chart-coordinator.test.ts`;
  `npm --prefix apps/web run typecheck`, 변경 파일 Biome.
  현재 Vitest는 Node 환경이므로 SSR에서 DOM 접근이 없고 shell/summary가 남는지, 순수 선택/변환
  계약이 유지되는지 확인한다. 이 단계만을 위해 DOM test 의존성을 추가하지 않는다.
  실제 effect/Strict Mode/cleanup/Canvas/gesture 검증은 WP-3의 브라우저 수용 조건으로 남긴다.
- **필수 보고:** 변경 파일과 라인 범위; 명세 이탈과 이유; 실행한 검사와 결과; 미해결/후속 작업;
  찾지 못했거나 검증하지 못한 내용. 빈 항목은 각각 `none`으로 명시한다.
- **위임 제한:** Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

### WP-3 — 브라우저 회귀와 통합 검증

- **작업 디렉터리:** `/data/worktrees/3puw275b/itchy-bulldog`.
- **초기 분류:** intermediate, confidence medium. provider-free fixture가 있지만 DOM 기반 assertion을
  실제 Canvas 행동 검증으로 옮겨야 한다. 동일 재현 실패/flake가 두 번 반복되거나 현재 fixture와
  route의 모드가 모순되면 재현 증거를 남기고 coordinator에 보고한다.
- **목적/배경:** 기존 `stock-beta.spec.ts:700`은 `g[data-candle-index]` 261개와 기간별 DOM 개수를
  센다. `stock-beta-intraday.spec.ts`에도 차트 표현에 결합한 assertion이 있다. Canvas 존재만으로
  값·gesture·선택·실패 처리가 정확함을 증명할 수 없으므로 검증 관측을 바꾼다.
- **소유 파일:** `apps/web/tests/e2e/stock-beta.spec.ts`,
  `apps/web/tests/e2e/stock-beta-intraday.spec.ts`의 차트 assertion;
  `apps/web/tests/e2e/support/stock-beta-fixture.mjs`의 필요한 차트 fixture만;
  `docs/reviews/2026-10-07-stock-beta-lightweight-charts-qa.md`(신규).
  screenshot/trace/log 등 임시 산출물은 무시되는 테스트 결과 경로 또는 `/tmp`에 둔다.
- **입력/의존:** WP-2가 수용된 전체 diff, WP-1 adapter 테스트 결과, 기존 synthetic API 및
  `apps/web/playwright.config.ts`, 이 계획의 수용 기준. 제품 코드는 읽기 전용이다.
- **수행:** 기존 기간별 응답/adapter의 개수·정확값 검증을 보존한다. 실제 차트에서는 pointer와
  keyboard로 표시되는 날짜/OHLCV, 확대 후 다른 가시 구간, 드래그/초기화, 두 pane 정렬,
  크기 변경, null gap, 빠른 종목/기간 전환, 오류 시 제거를 확인한다. canvas DOM 개수를
  봉 개수로 해석하거나 허구의 data attribute로 옛 test를 통과시키지 않는다.
- **환경:** 설치된 local Next/Playwright binary를 사용한다. 현재 fixture 모드를 확인하고
  별도 loopback 포트의 synthetic API와 Next를 시작한다. readiness는 자체 child PID의 생존과
  해당 URL 응답을 함께 확인하고, 종료 시 자신이 시작한 PID만 정리한다. 기존 외부 요청 차단을 유지한다.
  로컬 서버 예: `SYNTHETIC_API_PORT=38182 node apps/web/tests/e2e/support/synthetic-api.mjs`,
  `API_INTERNAL_URL=http://127.0.0.1:38182 npm --prefix apps/web run dev -- --port 33001`.
  포트 충돌 시 미사용 포트로 함께 바꾸며 다른 worktree 서버를 테스트하지 않는다.
- **검증 명령:** `npm --prefix apps/web test`, `npm --prefix apps/web run typecheck`,
  `npm --prefix apps/web run lint`;
  `PLAYWRIGHT_BASE_URL=http://127.0.0.1:33001 SYNTHETIC_API_ORIGIN=http://127.0.0.1:38182 npm --prefix apps/web run test:e2e -- tests/e2e/stock-beta.spec.ts tests/e2e/stock-beta-intraday.spec.ts`;
  `API_INTERNAL_URL=http://127.0.0.1:38182 npm --prefix apps/web run build`.
  기준 fixture 모드에서 skip되는 기존 test는 이유와 수를 구분해 적는다. 이번 차트의 필수 기능을
  skip한 채 수용하지 않는다. 과거 quote 모드를 강제로 되살리거나 프로덕션 기능을 켜지 않는다.
- **추가 확인:** JS 미실행/SSR의 텍스트 대안, 브라우저 mount 이후 Canvas 실제 그리기,
  console/pageerror, repeated mount/탭 전환 후 잔여 인스턴스, 터치/페이지 스크롤,
  forced-colors, 한국어 copy, 출처 링크를 확인한다. 번들 크기를 측정하고 비교 가능한 baseline이
  없으면 증가량을 추정하지 않는다. 모든 필수 viewport에서 증거를 남긴다.
- **금지:** test를 위한 제품 동작 약화, assertion 삭제만으로 통과, live API·운영 로그인·Raw 접근,
  실시간/REST quote 경로 수정, 무관한 전체 E2E/CI 리팩터링. 제품 결함은 WP-2로 반환한다.
- **산출물:** 수정된 E2E와 QA 문서. 정확한 명령/종료 상태, 실행/실패/skip 수, 필요한 이미지,
  bundle·관측 성능 결과, 환경 한계와 미검증 항목을 포함한다.
- **필수 보고:** 변경 파일과 라인 범위; 명세 이탈과 이유; 실행한 검사와 결과; 미해결/후속 작업;
  찾지 못했거나 검증하지 못한 내용. 빈 항목은 각각 `none`으로 명시한다.
- **위임 제한:** Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

### WP-4 — 독립 리뷰

- **작업 디렉터리:** `/data/worktrees/3puw275b/itchy-bulldog`.
- **초기 분류:** intermediate, confidence high. 완성 diff와 검증 증거를 대조하는 한정된 코드 리뷰다.
  데이터 의미/권한/구조 경계 변경 또는 서로 모순되는 증거가 발견되면 해당 패키지 수용을 보류한다.
- **목적/배경:** 구현자는 WP-1~3이고 리뷰어는 수정 없이 결론을 낸다. 최종 권한과 수용 판단은
  coordinator에게 있다. Canvas 전환이 의미·접근성·실패 처리 회귀를 숨기지 않았는지 확인한다.
- **소유 파일:** `docs/reviews/2026-10-07-stock-beta-lightweight-charts-review.md`만 작성한다.
  제품·테스트·dependency·QA 문서와 전체 diff는 읽기 전용이다.
- **입력/의존:** 기준 checkout 이후 완성 diff, 이 계획, WP-1~3 결과와 QA 증거, applicable AGENTS.
  리뷰 시작 전에 coordinator가 리뷰할 파일 상태/commit 또는 diff hash를 고정해 전달한다.
- **검토:** sourceIndex/date 변환, 중간 SMA gap, controlled selection, formatter, invalid/empty,
  SSR/hydration, async mount와 cleanup, resize/gesture, 상태 전환 후 ghost chart, 한·영/고대비,
  출처 표시, 최소 dependency/번들 및 test의 관측력이 수용 기준을 충족하는지 대조한다.
- **검증:** `git diff --check`, 기준 checkout에 대한 diff와 관련 `rg` 검색, 실패 가설에 필요한
  focused test만 재현한다. 전체 검증이 이미 통과했으면 근거 없이 반복하지 않는다.
  diagram은 npm 내부 renderer 의존성만 바뀌고 기존 구조 경계가 그대로인지 확인한다.
- **금지:** 직접 버그 수정, 계약 완화, 리뷰 중 unplanned network/운영 작업, 무관한 리팩터링 제안.
  발견 사항은 심각도·file:line·재현/영향·권장 수정과 함께 원래 소유자에게 반환한다.
- **산출물:** 수용 가능 여부와 근거, blocking findings, 잔여 위험, 실제 검증 범위를 담은 리뷰 문서.
- **필수 보고:** 변경 파일과 라인 범위; 명세 이탈과 이유; 실행한 검사와 결과; 미해결/후속 작업;
  찾지 못했거나 검증하지 못한 내용. 빈 항목은 각각 `none`으로 명시한다.
- **위임 제한:** Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

## Coordinator gates

1. **Pre-launch**
   - 후속 구현 요청과 target checkout을 확인한다. 계획 작성만으로 worker를 시작하지 않는다.
   - `$paseo-delegate` 전체와 현재 Paseo profile/capability를 읽고 model/effort를 확인한다.
     모든 assignment와 follow-up에 재위임 금지 문장, 정확한 소유권, 필수 보고 형식을 넣는다.
   - `git status --short`, HEAD, 적용 instruction 변경, 기존 작업을 확인한다. 앞선 대화/메모의
     배포·버그 수정은 해당 checkout에서 검증한 것만 전제로 삼는다.
   - Node/npm workspace/Next 로컬 guide/browser/loopback 실행 전제를 준비한다.
     권한 문제는 정확한 명령에 대한 허가로 해결하고 sandbox/state 경로 우회로 처리하지 않는다.
   - 현재 계획상 필수 미결 사용자 결정은 없다. 추가 요구나 충돌이 생기면 영향을 받는 branch만
     멈추고 명세/소유권/graph를 고친다. 추측해서 범위를 넓히지 않는다.
2. **Wave 1 수용**
   - dependency diff가 5.2.1과 그 필수 transitive 변경에 한정되는지 확인한다.
     adapter의 날짜/숫자/index/결측 테스트와 WP-2가 사용할 타입 계약을 직접 대조한다.
3. **Wave 2 수용**
   - 실제 코드에서 mount/cleanup, 실패 시 제거, HTML 접근성, 출처를 확인한다.
     Node unit 통과를 실제 Canvas 성공으로 간주하지 않고 WP-3로 넘긴다.
4. **Wave 3 수용**
   - fixture와 자체 server PID, browser 요청 제한, 실패/skip 이유, 핵심 gesture/결측/반응형
     증거를 검토한다. 환경 차단을 성공으로 보고하지 않는다.
   - 문제가 있으면 파일 소유자를 통해 수정하고 영향받은 검사만 다시 실행한다.
5. **Wave 4 수용**
   - 리뷰 findings를 coordinator가 재검토한다. blocking defect를 해결한 뒤 수정 범위에 대한
     재검증·재리뷰를 받는다. 보고가 얇거나 `none` 항목이 누락되면 완결 보고를 요청한다.
6. **Final end-to-end acceptance**
   - 목표 1~8을 각각 증거에 연결하고 최종 diff/check 결과를 확인한다.
   - API·runtime 구조 경계가 바뀌지 않은 renderer 전환이면 diagram을 건드리지 않는다.
     workspace/package 경계, web↔API, compose 등 실제 구조 변경이 필요해졌다면 이 계획 밖 변경으로
     graph를 재검토하고 AGENTS에 따라 해당 PUML/evidence와 로컬 PNG를 같은 변경으로 갱신한다.
   - 실제 사용자 기준의 종목 선택 → 기간 변경 → 확대/이동 → 키보드 조회 → 초기화 → 오류 표시를
     기존 QA 증거 또는 필요한 최종 재현으로 확인한다. 정확한 테스트 결과·제한·번들 관측을 보고한다.
   - merge/프로덕션 배포는 이 계획의 완료 조건에 포함하지 않는다. 최종 결과는 검토 가능한 diff와
     QA/리뷰 증거이며, 별도 배포 요청은 그때의 release 지침과 권한을 따른다.

## Execution clarification — 2026-10-07

- WP-2의 Node 집중 검사는 59개를 통과했지만 WP-3 브라우저에서 Canvas 384px가
  실제 surface 약 38px로 압축되는 결함과 controlled pointer/crosshair 불일치를 재현했다.
  WP-2는 gpt-6-luna/max에서 gpt-6.1-sol/max로 한 단계 올려 수정한다.
- 레이아웃 수정은 기존 chart/와 Price 탭 크기 규칙을 우선 사용한다. 불가피하면
  같은 dashboard CSS의 signal-profile widget/profilePanel/signalPreview에 한정된
  크기·overflow 규칙까지 허용한다. 다른 widget의 배치/동작이나 API 변경은 제외한다.
- JavaScript 비활성 브라우저 검증은 실제 PriceChart의 서버 렌더 HTML을 제공하는
  임시 component harness로 수행한다. 기존 Stock Beta route는 hydration 전에
  Loading workspace를 표시하며 이는 renderer 전환 전부터의 제약이다.
  workspace/data-loader는 읽기 전용으로 유지하고 이 제약을 QA에 명시한다.
- WP-3의 같은 gesture/locale 실패가 반복됐으므로 재개 시 gpt-6.1-sol/max로
  한 단계 올린다. WP-4의 독립 리뷰는 기존 gpt-6.1-sol/high를 유지한다.
- WP-2의 마지막 내부 table 스타일 수정과 독립적인 WP-3 테스트 정리는 병행할 수 있다.
  WP-3 준비 단계는 E2E의 locale locator, 격리된 import 실패/SSR harness와 Canvas 내부
  geometry assertion 작성만 허용한다. 공유 Next/synthetic 서버·fixture 변경과 실제 제품
  검증은 WP-2의 최종 handoff 전까지 금지한다. 최종 QA 수용과 WP-4 순서는 그대로 유지한다.
- 기본 intraday `off` 모드에서 기존 intraday 전용 suite의 `owner_only` 선행 assertion이
  실패하는 기준 설정 불일치를 확인했다. WP-3는 해당 beforeAll에 `off`일 때의 명시적
  skip gate만 추가할 수 있다. `owner_only` assertion과 test body, 제품 모드/기능은 유지하며,
  건너뛴 12개 실시간 시세 검사는 최종 QA에서 차트 검증 결과와 구분한다.

## Final acceptance — 2026-10-08 KST

- WP-1~4의 보고서와 실제 diff/증거를 coordinator가 검토했고, 독립 리뷰의 **ACCEPT / blocking findings none**을 수용했다.
- [QA 증거](../../reviews/2026-10-07-stock-beta-lightweight-charts-qa.md)의 acceptance matrix가 목표 1~8을 연결한다. [독립 리뷰](../../reviews/2026-10-07-stock-beta-lightweight-charts-review.md)는 실제 소스, 이미지, 선택/수명주기/결측 probe와 검사 로그를 대조했다.
- 최종 검증: unit 57 files / 436 passed / 0 failed / 0 skipped; 두 E2E 파일 34 passed / 0 failed / 12 skipped; 전체 typecheck, lint, production Web build, `git diff --check` 통과. 12개 skip은 기본 `off` 모드의 별도 intraday suite이며 차트 검증을 대신하지 않는다.
- 리뷰 manifest SHA256 `abef217ee140588ce7acfbfe425adcde5a1f620ef7cc1a10cd9843bfbb49cad8`의 19개 파일과 기준 HEAD를 리뷰 시작·종료 및 coordinator 수용 시 확인했다. 제품/테스트/QA 파일 drift는 없다. 계획의 상태와 독립 리뷰 문서만 manifest에서 제외했다.
- API·controller·workspace·Compose 경계는 바뀌지 않았으므로 architecture diagram은 그대로 유지했다. 추가 의존성은 `lightweight-charts@5.2.1`과 필수 `fancy-canvas@2.1.0`뿐이다.
- 시간축 극단의 일부 native 눈금 글자 잘림은 경미한 제약으로 수용했다. 선택 날짜와 OHLCV HTML은 완전하며 봉·축·제어 영역의 심각한 잘림은 없다. 앱의 기존 no-JS loading shell, viewport-equivalent 200% 검사, heap GC 미입증, 전환 전 bundle baseline 부재는 QA/리뷰에 명시했다.
- 모든 소유 worker는 실제 idle이고 pending permission은 없다. task-owned 33017/38182/33018 listeners 종료를 확인했으며, 무관한 33001은 유지했다. 결과는 로컬 diff와 QA/리뷰 문서다. 커밋·merge·배포는 수행하지 않았다.
