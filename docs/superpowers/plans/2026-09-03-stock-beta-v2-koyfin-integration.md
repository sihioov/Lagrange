Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Stock Beta V2 × Koyfin UI 통합 실행 계획

작성일: 2026-09-03
상태: 계획만 작성됨, 실행 미시작

## Goal and boundaries

### 목표와 완료 기준

현재 `main`의 Owner-managed equity universe V2 기능과 `57fd817`의 Koyfin형 공통 셸·위젯
시스템을 하나의 활성 `/stock-beta` 제품으로 통합한다. 완료 시 다음 조건을 모두 만족해야 한다.

1. `/stock-beta`와 `/stock-beta/[instrument]`는 V2 API만 사용한다.
2. 종목 추가, 정확한 6자리 검증, policy capacity, lifecycle/coverage, polling, retry, soft
   disable, READY 이후 rank/detail 갱신을 보존한다.
3. 화면은 단순한 다크 테마가 아니라 얇은 상단 바, 고밀도 좌측 레일, 실제 V2 snapshot strip,
   순위·선택 종목 분석·신호 분해가 동시에 보이는 Koyfin형 워크스페이스를 사용한다.
4. 실제 V2 DTO에 없는 종목명, factor explanation, condition reason, V1 provenance 상태, 숫자·추세
   filter를 만들거나 V1 호출로 보충하지 않는다.
5. 위젯 추가·제거·순서 변경은 중앙 registry/layout 설정으로 가능하며 위젯은 직접 fetch하지 않는다.
6. Owner/Member 권한, fail-closed 오류 상태, 서버 순서, 원본 숫자, 정책 경고를 보존한다.
7. TypeScript, lint, unit, production build, provider-free Playwright와 독립 리뷰가 통과한다.
8. 검증된 통합 커밋만 `main`에 병합하고 non-force로 `origin/main`에 push한 뒤 로컬·원격 HEAD가
   일치함을 확인한다.

### 기준 ref와 대상 workspace

- 통합 기준 main: `cd0abb5` (`origin/main`의 `1db23ce`를 로컬 main에 병합한 상태)
- Koyfin 기능 커밋: `57fd817`
- 현재 `/data/workspace/lagrange`는 두 ref의 병합 충돌이 진행 중이다.
- 구현 worker는 이 dirty main worktree를 직접 사용하지 않는다. 실행 시 coordinator가 현재 상태와
  ref를 재검증하고 병합을 안전하게 중단한 다음, `cd0abb5` 기반의 전용 Paseo integration
  workspace에서 `57fd817`을 통합한다.
- main worktree의 미추적 `CLAUDE.md`와
  `docs/kis_openapi_entiredocs_20260818_030007.xlsx`는 사용자 소유 파일로 간주하여 읽기 외에는
  변경·stage·commit하지 않는다.

### 활성 제품 계약

- V2가 활성 제품의 유일한 데이터 원본이다.
  - memberships/policy: `getOwnerEquityV2Memberships`
  - latest signals: `getOwnerEquityV2LatestSignals`
  - detail: `getOwnerEquityV2SignalDetail`
  - mutation: V2 add/retry/disable client
- 기존 V1 API와 contract 코드는 호환성 표면으로 그대로 두지만 활성 Stock Beta 페이지에서 호출하지
  않는다.
- V2 snapshot에서 표시할 수 있는 값은 `snapshot_id`, `as_of`, `universe_sha256`, `row_count`,
  `published_at`뿐이다. Signal은 `instrument_id`, `generation`, rank/score/condition 및 실제
  price-volume 지표만 사용한다.
- 조건별 개수처럼 현재 응답 행에서 손실 없이 계산 가능한 값은 표시할 수 있다. 순서를 바꾸거나
  점수·조건을 재계산하지 않는다.
- V2에는 종목명이 없으므로 검색과 헤더는 `instrument_id`를 사용한다.
- V1 전용 range/trend GET filter와 detail factor/provenance/reason 위젯은 V2 화면에서 숨긴다.
  작동하지 않는 필터나 빈 장식 패널을 남기지 않는다.

### 고정 정보 구조

- 데스크톱 첫 행은 `ranked signals / selected signal profile / signal decomposition`의 3영역이다.
- 둘째 행은 `condition matrix / current snapshot tape`를 기본으로 하고, V2 universe management와
  membership status는 조밀한 실제 작업 패널로 그 아래 또는 실제 동작하는 title tool/drawer에 둔다.
- 빈 universe나 아직 snapshot이 없는 상태에서는 관리·membership workflow를 먼저 노출하고 과거
  signal을 남기지 않는다.
- detail은 instrument ID, generation, rank, score, condition, V2 snapshot과 returns/risk/activity만
  표시한다. V1 전용 evidence/provenance를 합성하지 않는다.
- policy boundary는 접을 수 있어도 Owner-only, read-only, original-price, non-PIT, activity-proxy
  경고 접근 경로를 항상 유지한다.

### 적용 지침과 기준 문서

- `/data/workspace/lagrange/AGENTS.md`
  - production build 자원 정책을 준수한다. 이번 Web build는 production image build가 아니며 Docker
    release build를 실행하지 않는다.
  - KIS/OpenDART/live trading/network 수집 표면은 이번 작업 범위 밖이다.
  - Web-to-API contract를 변경하지 않으므로 architecture diagram은 변경하지 않는다. 구현 중 실제
    contract 구조가 바뀌어 diagram 갱신이 필요해지면 즉시 blocker로 보고한다.
- `/data/workspace/lagrange/CLAUDE.md`는 `@AGENTS.md`를 가리킨다.
- `/data/workspace/lagrange/apps/web/AGENTS.md`와 `CLAUDE.md`
  - 코드 수정 전 설치된 Next.js 문서를 읽는다.
- 기준 제품/디자인 문서:
  - `docs/decisions/0008-owner-managed-equity-universe-v2.md`
  - `docs/superpowers/plans/2026-08-31-owner-managed-equity-universe-v2.md`
  - `docs/superpowers/specs/2026-09-02-lagrange-koyfin-ui-system.md`
  - `docs/superpowers/specs/2026-09-02-stock-beta-koyfin-ui-overhaul.md`
  - `apps/web/components/stock-beta/README.md`
- 구현 worker가 먼저 읽을 로컬 Next.js 문서:
  - `node_modules/next/dist/docs/01-app/01-getting-started/03-layouts-and-pages.md`
  - `node_modules/next/dist/docs/01-app/01-getting-started/05-server-and-client-components.md`
  - `node_modules/next/dist/docs/01-app/01-getting-started/11-css.md`
  - `node_modules/next/dist/docs/01-app/02-guides/forms.md`
- 테스트 worker가 먼저 읽을 로컬 문서:
  - `node_modules/next/dist/docs/01-app/02-guides/testing/vitest.md`
  - `node_modules/next/dist/docs/01-app/02-guides/testing/playwright.md`
  - `node_modules/next/dist/docs/03-architecture/accessibility.md`

### 범위 제외

- Backend, OpenAPI, DTO, database, migration, worker, deployment, KIS/OpenDART 또는 live trading 변경
- V2에 없는 데이터의 신규 수집이나 합성
- V1 화면을 활성 제품으로 되돌리는 작업
- 사용자별 widget 저장, drag-and-drop, 작동하지 않는 Add Widget 버튼
- production release/image build 또는 서비스 배포
- 별도 결함으로 이미 확인된 `/paper`, `/live` 390px overflow 수정. 단, 이번 변경이 해당 경로를
  추가로 악화시키지 않는 smoke 검증은 수행한다.

### 미해결 요구사항

없음. 사용자는 V2 기능을 유지하면서 Koyfin UI를 이식하는 방향을 선택했다. V1 전용 데이터가 V2에
없을 때는 숨기고 합성하지 않는 것으로 확정한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | 서로 다른 V1/V2 의미 계약과 9개 충돌 파일을 통합하면서 mutation/polling과 widget architecture를 함께 보존해야 한다. | high | API/DTO 변경 없이는 요구 UI를 구성할 수 없거나 V2 lifecycle 갱신과 terminal selection state가 충돌하면 중단하고 coordinator에게 보고한다. |
| WP-2 | intermediate | 생산 코드가 확정된 뒤 V2 surface·widget·권한 테스트를 새 구조에 맞추는 범위가 명확하며 Vitest로 결정적으로 검증할 수 있다. | high | 생산 코드 수정이 필요하거나 같은 assertion 수정이 두 번 실패하면 terra medium으로 상향한다. |
| WP-3 | intermediate | V2 synthetic fixture의 mutable lifecycle과 Koyfin interaction을 함께 검증해야 하지만 기존 fixture와 Playwright 패턴이 있다. | high | polling race, 비결정적 상태 오염, 두 차례 반복 실패가 발생하면 terra medium으로 상향한다. |
| WP-4 | hard | 결론 자체가 산출물인 독립 semantic/security/accessibility 리뷰이며 V1 데이터 누출과 V2 workflow 회귀를 찾아야 한다. | high | active route/API 판정이 불명확하거나 테스트와 구현이 모순되면 sol high 자문으로 상향한다. |
| WP-5 | simple | 변경 없이 정해진 전체 검증 명령과 결과를 수집하는 기계적 QA다. | high | build/E2E가 환경이 아닌 코드 이유로 실패하거나 결과 분류가 모호하면 intermediate로 재분류한다. |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | V2 기능과 Koyfin production UI를 하나의 widget 기반 화면으로 통합 | Stock Beta pages/components/i18n 및 `product.css`의 Stock Beta 구간 | 없음 | `$paseo-delegate`: gpt-5.6-sol, high | 충돌 없는 V2 production UI와 registry/layout | typecheck, lint, build, diff check |
| WP-2 | 2 | intermediate | V2 production surface와 widget architecture의 unit/contract 회귀 고정 | Stock Beta Vitest·accessibility test 파일 | WP-1 | `$paseo-delegate`: gpt-5.6-luna, max; 2회 실패 시 terra medium | V2 기준 unit test suite | focused Vitest, full Web unit |
| WP-3 | 2 | intermediate | V2 lifecycle와 Koyfin interaction을 provider-free 브라우저에서 검증 | Stock Beta Playwright spec과 synthetic fixture | WP-1 | `$paseo-delegate`: gpt-5.6-luna, max; 2회 실패 시 terra medium | deterministic V2 E2E fixture/spec | focused Stock Beta E2E |
| WP-4 | 3 | hard | 통합 결과의 독립 read-only acceptance review | 전체 통합 diff와 테스트 증거, 파일 수정 금지 | WP-1~WP-3 | `$paseo-delegate`: gpt-5.6-terra, high | severity별 findings와 ACCEPT/REJECT | 정적 추적, 관련 테스트 증거 대조 |
| WP-5 | 3 | simple | 전체 Web regression과 빌드·브라우저 QA 실행 | repository read-only, 테스트 산출물만 허용 | WP-1~WP-3 | `$paseo-delegate`: gpt-5.6-luna, medium | 명령별 PASS/FAIL 및 환경 정리 보고 | lint, typecheck, unit, build, full E2E, viewport checks |

Wave 2의 WP-2와 WP-3는 생산 코드가 고정된 동일 integration base에서 서로 다른 테스트 파일만
소유하므로 병렬 실행할 수 있다. Wave 3의 WP-4와 WP-5는 read-only이므로 병렬 실행할 수 있다.

## Worker briefs

### WP-1 — V2 production UI와 widget architecture 통합

- **Target working directory:** `$paseo-delegate`가 만든 `cd0abb5` 기반 integration workspace
- **Initial complexity:** hard. V1용 UI를 V2 데이터에 맞게 재구성하면서 상태 mutation과 fail-closed
  동작을 보존해야 한다. confidence high.
- **Escalation signals:** API/DTO 수정 필요, V2 snapshot에 없는 값을 요구, lifecycle polling과 선택 상태
  동기화가 양립하지 않음, 같은 구현 실패 2회. 추측으로 메우지 말고 blocker를 보고한다.
- **Objective:** `57fd817`의 공통 terminal shell과 registry/layout primitives를 유지하면서 활성
  Stock Beta production route를 V2 전용 Koyfin workspace로 만든다.
- **Known facts:** main 쪽은 V2 membership/poll/mutation을 구현하고, Koyfin 쪽은 V1
  `OwnerBetaEquitySignals*` DTO를 전제로 한다. V2에는 instrument name, 상세 factor/reason/provenance가
  없다. V2 signal numeric fields는 V1과 거의 같지만 snapshot shape와 universe lifecycle이 다르다.
- **Owned scope:**
  - `apps/web/app/(authenticated)/stock-beta/page.tsx`
  - `apps/web/app/(authenticated)/stock-beta/[instrument]/page.tsx`
  - `apps/web/components/stock-beta/**`
  - `apps/web/lib/i18n/dictionaries/stock-beta.ts`
  - `apps/web/app/product.css` 중 Stock Beta 충돌/호환 구간만
- **Read-only inputs:** `apps/web/lib/products/equity-signals-contracts.ts`,
  `equity-signals-client.ts`, `apps/web/lib/api/product-client.ts`, shell 공통 파일, V2 ADR/plan, Koyfin specs.
- **Required implementation:**
  - active pages use only V2 server client methods and V2 error codes;
  - add/retry/disable/poll/refresh와 stale signal 제거를 그대로 보존;
  - 실제 V2 snapshot strip과 compact universe-management/membership widgets 제공;
  - rank/profile/decomposition/matrix/tape selection 동기화와 동적 0~100행 지원;
  - dashboard/detail registry와 breakpoint layout을 V2 view model로 통일;
  - unsupported V1 widgets/filter copy 제거 또는 V2 registry에서 완전히 제외;
  - Pretendard Variable과 Geist Mono data typography, 1px border, 4px 이하 radius, 6~8px panel gap 유지;
  - 375/768/1280/1440과 200% reflow에서 Stock Beta page-level overflow 방지.
- **Prohibited adjacent work:** API/contract/OpenAPI/backend 변경, V1 fallback 호출, DTO 값 합성, server
  order 변경, unrelated shared route 수정, root untracked 파일 stage, network/provider/live 호출.
- **Expected output:** 충돌 marker가 없고 V2 flow와 Koyfin structure가 함께 존재하는 production code.
- **Verification:** `git diff --check`; conflict-marker scan; V1/V2 active call-site scan;
  `npm --prefix apps/web run typecheck`; `npm --prefix apps/web run lint`;
  `API_INTERNAL_URL=http://127.0.0.1:38182 npm --prefix apps/web run build`. WP-2 이전 기존
  conflict test 실패는 정확히 구분해 보고한다.
- **Required report:** 변경 파일·라인 범위; brief와 다르게 처리한 부분과 이유; 실행한 검사와 결과;
  미해결/후속 작업; 찾지 못했거나 확인하지 못한 것. 비어 있으면 각 항목에 `none`을 명시한다.

### WP-1A — 데스크톱 정보 구조 보정

- **Depends on:** WP-1 coordinator review.
- **Complexity:** intermediate. 기존 registry/layout과 CSS grid 배치만 좁게 보정한다.
- **Owned scope:** `apps/web/components/stock-beta/dashboard/dashboard-layout.ts`,
  `apps/web/components/stock-beta/dashboard/dashboard.module.css` 및 이 변경에 직접 필요한 dashboard
  production 파일만.
- **Required correction:** 1280×720 이상 데스크톱에서 첫 행을 `ranked-signals / signal-profile /
  signal-decomposition`, 둘째 행을 `condition-matrix / snapshot-tape`로 만든다. compact
  `universe-management / membership-status`는 그 아래 작업 행에 배치한다. empty/no-snapshot 상태에서
  관리 workflow가 먼저 노출되는 예외와 tablet/mobile reflow는 유지한다.
- **Prohibited:** API/DTO/backend/test 파일 변경, V1 fallback, 합성 데이터, 다른 route 또는 shell 수정.
- **Verification:** production-only typecheck, owned Biome, scoped build 가능 여부, CSS/registry 정적 확인.
- **Required report:** 변경 파일·라인 범위, 명세 차이, 검증, 미해결, 확인하지 못한 것.

### WP-2 — V2 unit·surface·architecture 테스트 통합

- **Target working directory:** accepted WP-1 commit을 기반으로 한 Paseo workspace
- **Initial complexity:** intermediate. 생산 계약은 WP-1에서 고정되고 테스트 결과가 기계적으로
  판정 가능하다. confidence high.
- **Escalation signals:** production 수정이 필요하거나 assertion/fixture 정합성이 두 번 실패하면
  worker가 임의 수정하지 않고 terra medium 재실행을 요청한다.
- **Objective:** V1 전용 기대를 제거하고 V2 기능·Koyfin layout·widget 확장성·권한 경계를 unit과
  server-render surface 테스트로 고정한다.
- **Owned scope:**
  - `apps/web/tests/owner-beta-equity-signals-surface.test.tsx`
  - `apps/web/tests/stock-beta-dashboard.test.tsx`
  - `apps/web/tests/stock-beta-widget-architecture.test.tsx`
  - 필요한 신규 `apps/web/tests/stock-beta-v2-*.test.tsx`
  - Stock Beta 관련 범위의 `apps/web/tests/accessibility.test.tsx`
- **Must cover:** 0/31/100 dynamic rows, V2 snapshot/detail, Owner-only DOM, expired session redirect,
  typed unavailable/integrity/not-found, no V1 detail evidence, management policy/lifecycle presentation,
  registry required-widget validation, serializable config, selection synchronization, one `main`/one `h1`,
  no stale signal data after failure.
- **Prohibited adjacent work:** production file 수정, E2E fixture/spec 수정, V1 endpoint를 활성 동작으로
  기대, snapshot에 없는 상태나 값을 fixture에 추가.
- **Verification:** focused Vitest on all owned files, then `npm --prefix apps/web run test` and
  `npm --prefix apps/web run typecheck`.
- **Required report:** 변경 파일·라인 범위; brief 편차와 이유; 테스트 명령과 정확한 pass/fail 수;
  미해결/후속; 찾지 못했거나 미검증한 항목. 없으면 `none`.

### WP-3 — V2 Koyfin provider-free Playwright 통합

- **Target working directory:** accepted WP-1 commit을 기반으로 한 WP-2와 분리된 Paseo workspace
- **Initial complexity:** intermediate. mutable synthetic state와 polling을 다루지만 기존 V2 fixture가
  있다. confidence high.
- **Escalation signals:** test 간 state 누출, polling race, 고정 sleep 의존, 같은 실패 2회 발생 시
  terra medium으로 상향한다.
- **Objective:** main V2 lifecycle E2E를 보존하고 Koyfin shell·selection·responsive interaction을 V2
  fixture로 검증한다.
- **Owned scope:**
  - `apps/web/tests/e2e/stock-beta.spec.ts`
  - `apps/web/tests/e2e/support/stock-beta-fixture.mjs`
  - V2 synthetic dispatch에 꼭 필요한 경우에만 `apps/web/tests/e2e/support/synthetic-api.mjs`
- **Must cover:** empty capacity; 31/100 rows; add→poll→READY→rank/detail refresh; retry→READY;
  disable confirmation과 stale snapshot 제거; Owner/Member; expired session; V2 detail not found;
  terminal shell continuity; row/matrix selection; instrument-ID search `/`/Escape; keyboard and touch target;
  Korean/English; 375×812, 768×1024, 1280×720, 1440×900, 200% reflow; reduced motion/forced colors;
  failure states에 과거 숫자 미노출.
- **Fixture constraints:** provider/network 호출 금지, 테스트별 deterministic reset, V2 response schema만
  사용, 비밀값·provider prose 금지, 임의 sleep 대신 observable state를 기다린다.
- **Prohibited adjacent work:** production 코드와 unit test 수정, V1 range/trend/factor/provenance 기대,
  테스트 통과를 위한 contract 완화.
- **Verification:** focused Stock Beta Playwright를 최소 2회 연속 실행하고 종료 뒤 사용 포트와 child
  process가 정리됐는지 확인한다.
- **Required report:** 변경 파일·라인 범위; brief 편차와 이유; 각 E2E 실행의 pass/fail 수와 시간;
  미해결/후속; 찾지 못했거나 미검증한 항목. 없으면 `none`.

### WP-2A — shell runtime 회귀 assertion 보정

- **Depends on:** WP-2 coordinator full-unit rerun.
- **Complexity:** simple. 실제 terminal shell 계약은 유지하고 JSX formatting에 의존하는 assertion만
  의미 기반으로 좁게 보정한다.
- **Owned scope:** `apps/web/tests/shell-runtime.test.ts`만.
- **Required correction:** `StockBetaTerminalPage`에 `StockBetaInstrumentSearch`가 `search` prop으로
  전달되는 계약을 줄바꿈과 formatter 출력에 독립적으로 검증한다. assertion을 삭제하거나 완화해
  잘못된 컴포넌트 연결도 통과시키면 안 된다.
- **Verification:** focused shell-runtime Vitest, full Web Vitest, owned Biome.

### WP-3A — E2E typecheck·lint 보정

- **Depends on:** WP-3 coordinator typecheck/lint rerun.
- **Complexity:** simple. 검증 의미를 유지한 채 TypeScript narrowing과 formatter/import 규칙을 맞춘다.
- **Owned scope:** `apps/web/tests/e2e/stock-beta.spec.ts`,
  `apps/web/tests/e2e/support/stock-beta-fixture.mjs`, `apps/web/tests/e2e/typography.spec.ts`만.
- **Required correction:** optional access TS2532와 non-null assertion을 안전한 narrowing으로 바꾸고,
  import/order/format 오류를 정리한다. 16개 Stock Beta 시나리오, loopback-only guard, full-page 증거,
  fixture 동작을 약화하거나 삭제하지 않는다.
- **Verification:** `node --check`, Playwright `--list`, typecheck, owned Biome, focused Stock Beta E2E.

### WP-4 — 독립 semantic·security·accessibility 리뷰

- **Target working directory:** WP-1~WP-3가 통합된 read-only Paseo workspace
- **Initial complexity:** hard. 테스트 통과 여부와 별개로 활성 contract와 정보 표현의 정확성을
  판정해야 한다. confidence high.
- **Escalation signals:** V1/V2 route 선택이 불명확하거나 runtime 증거와 코드가 모순되면 sol high
  자문을 요청한다.
- **Objective:** 파일을 수정하지 않고 통합 diff가 V2 기능과 Koyfin 요구를 실제로 모두 충족하는지
  검토한다.
- **Review checklist:** active route의 V2-only 호출; membership mutation/poll race; disable 직후 stale
  signal; Owner/Member DOM 격리; typed fail-closed 상태; server order/raw numeric 보존; V2에 없는 값
  합성 부재; widget fetch 부재; registry/layout 확장성; Server/Client 경계; keyboard/landmark/contrast;
  main untracked 파일·backend·API 무변경; conflict marker와 dead V1 UI 경로 부재.
- **Prohibited work:** 어떤 파일도 수정·stage·commit하지 않는다. live/network/provider 호출과 production
  동작도 금지한다.
- **Expected output:** severity별 finding, 근거 파일·라인, `ACCEPT` 또는 `REJECT` 판정.
- **Verification evidence:** WP-2/WP-3 결과와 실제 diff/route call graph를 대조한다.
- **Required report:** findings; 판정; 검토 범위; 미해결/후속; 확인하지 못한 것. 비어 있으면 `none`.

### WP-5 — 전체 Web QA와 시각 수용 검사

- **Target working directory:** WP-1~WP-3가 통합된 read-only Paseo workspace
- **Initial complexity:** simple. 명령과 판정 기준이 결정적이다. confidence high.
- **Escalation signals:** 코드/환경 원인 구분이 안 되거나 build/E2E 실패가 반복되면 intermediate로
  재분류한다.
- **Objective:** 수정 없이 전체 회귀와 Koyfin 수용 기준을 실행하고 증거를 남긴다.
- **Commands/evidence:**
  - `git diff --check`와 conflict-marker scan
  - `npm --prefix apps/web run lint`
  - `npm --prefix apps/web run typecheck`
  - `npm --prefix apps/web run test`
  - `API_INTERNAL_URL=http://127.0.0.1:38182 npm --prefix apps/web run build`
  - repository의 provider-free full Playwright QA script
  - Stock Beta 필수 viewport와 200% screenshot/overflow 검사
  - 모든 인증 메뉴의 동일 `research-terminal` shell smoke와 정적 asset 200 확인
- **Resource/safety:** production Docker image build 금지; 외부 API 금지; 기존 서버/포트를 먼저
  식별하고 충돌 없는 포트를 사용; 종료 시 자신이 시작한 process만 정리한다.
- **Expected output:** 명령별 exit code, 테스트 수, screenshot 경로, viewport별 overflow/구조 판정,
  알려진 `/paper`·`/live` 기존 overflow와 신규 회귀의 구분.
- **Required report:** 변경 파일은 `none`; brief 편차와 이유; 모든 검사 결과; 미해결/후속; 찾지
  못했거나 미검증한 항목. 없으면 `none`.

### WP-6A — disable stale-signal race 보정

- **Triggered by:** WP-4 high finding.
- **Complexity:** hard. 여러 V2 latest refresh와 disable/pending-removal 상태의 시간 순서를 fail-closed로
  조정해야 한다.
- **Owned scope:** `apps/web/components/stock-beta/stock-beta-workspace.tsx`, 이 동시성 guard에 직접 필요한
  새 Stock Beta production helper, 그리고 전용 deterministic unit test만.
- **Required correction:** disable 성공 전에 시작된 latest refresh가 늦게 완료돼도 signals를 복구하지
  못하게 epoch/abort 또는 동등한 latest-wins guard를 둔다. pending-disabled instrument가 포함된 응답은
  표시하지 않는다. 정상 READY refresh, typed errors, pending-removal poll은 유지한다.
- **Required test:** deferred promise의 완료 순서를 제어해 stale success와 stale failure 모두 현재 상태를
  덮지 못함을 재현한다. source-text 순서 검사만으로 대체하지 않는다.
- **Prohibited:** dashboard layout/E2E/API/DTO/backend 변경, V1 fallback, 외부 호출.
- **Verification:** focused concurrency/lifecycle unit, full Web unit, typecheck, lint, production build.

### WP-6B — registry-driven 실제 grid placement 보정

- **Triggered by:** WP-4 medium finding.
- **Complexity:** hard. breakpoint와 empty-state 배치를 central metadata로 옮기면서 고정 Koyfin 구조를
  보존해야 한다.
- **Owned scope:** dashboard의 `dashboard-layout.ts`, `stock-beta-dashboard.tsx`,
  `dashboard.module.css`, 필요한 `shared/widget-types.ts`/validator/registry, 그리고 dashboard/widget
  architecture unit tests만.
- **Required correction:** desktop/tablet의 widget ID별 `grid-row/grid-column` CSS selector를 제거하고
  row/column/span/visibility/order를 registry/layout metadata에서 CSS variables 또는 동등한 방식으로
  렌더링한다. add/remove/reorder가 page JSX/CSS 수정 없이 실제 위치를 바꿔야 한다. populated desktop
  1행 rank/profile/decomposition, 2행 matrix/tape, 그 아래 management와 empty management-first를 유지한다.
- **Required test:** optional widget의 metadata 순서/위치를 바꾼 architecture를 실제 렌더해 style/placement가
  바뀜을 검증한다. 설정 serialization 검사만으로 대체하지 않는다.
- **Prohibited:** workspace lifecycle/E2E/API/DTO/backend/공통 shell 변경.
- **Verification:** focused dashboard/architecture unit, full Web unit, typecheck, lint, production build.

### WP-6C — exact six-digit invalid-input E2E

- **Triggered by:** WP-4 low finding.
- **Depends on:** accepted WP-6A/WP-6B.
- **Complexity:** simple.
- **Owned scope:** `apps/web/tests/e2e/stock-beta.spec.ts`와 꼭 필요한 경우에만
  `apps/web/tests/e2e/support/stock-beta-fixture.mjs`.
- **Required correction:** short, long, suffixed, non-digit 입력 각각에서 typed validation message를
  확인하고 membership POST가 0회임을 browser request 관찰로 증명한다. 기존 16개 시나리오와
  loopback-only guard를 보존한다.
- **Verification:** typecheck, owned Biome, Playwright discovery, focused Stock Beta E2E 2회.

### WP-7 — remediation 독립 재리뷰와 상향 시각 QA

- **Depends on:** accepted WP-6A/WP-6B/WP-6C commit.
- **Complexity:** hard review + intermediate QA.
- WP-4 reviewer가 세 finding을 read-only로 재검증하고 `ACCEPT` 또는 잔여 finding을 보고한다.
- 이전 WP-5 visual harness가 quoting으로 두 번 실패했으므로 최종 QA worker는 한 단계 높은 모델을
  사용한다. lint/typecheck/unit/build/full provider-free E2E와 375/768/1280/1440/200%-equivalent를
  재검증하고, internal scroll의 initial viewport와 management region을 각각 캡처한다.

## Coordinator gates

### 1. Pre-launch

1. `$paseo-delegate`가 현재 실행 context에 있는지 확인한다. 없으면 worker를 시작하지 않고
   prerequisite를 보고한다. native subagent로 대체하지 않는다.
2. main worktree의 `HEAD=cd0abb5`, `MERGE_HEAD=57fd817`, 예상 충돌 9개와 사용자 미추적 파일 2개를
   다시 확인한다. 예상 밖 tracked change가 있으면 중단한다.
3. 현재 merge는 ref와 변경이 모두 recoverable함을 확인한 뒤에만 안전하게 중단한다. 사용자 미추적
   파일은 건드리지 않는다.
4. `cd0abb5` 기반 전용 Paseo integration workspace를 만들고 `57fd817`을 integration input으로
   고정한다. worker는 main worktree에서 직접 작업하지 않는다.
5. WP-1 brief와 활성 V2 계약을 worker launch 직전에 다시 대조한다.

### 2. Per-wave integration

1. **Wave 1:** WP-1 결과를 coordinator가 직접 diff로 확인한다. V1 fallback, DTO 합성, V2 mutation
   손실, conflict marker가 하나라도 있으면 reject한다. typecheck/lint/build 증거가 있어야 Wave 2로
   간다.
2. **Wave 2:** WP-2와 WP-3를 disjoint workspace에서 실행한다. 결과를 integration branch에 합친 뒤
   focused unit/E2E를 coordinator가 재실행한다. test가 production bug를 숨기기 위해 완화됐으면
   reject한다.
3. **Wave 3:** WP-4 review와 WP-5 QA를 같은 accepted integration commit에서 read-only로 실행한다.
   high/medium finding 또는 재현 가능한 QA failure가 있으면 임의로 고치지 않고 finding별 owned scope를
   가진 새 remediation WP를 이 계획에 추가한 뒤 다시 `$paseo-delegate`로 실행한다.

### 3. Final acceptance, main merge, push

1. WP-4가 `ACCEPT`이고 WP-5의 lint/typecheck/unit/build/full E2E가 모두 PASS인지 확인한다.
2. 1280×720에서 상단 바, 좌측 레일, snapshot strip, 3패널 첫 행, 하단 보조 행이 Koyfin 기준
   구조와 대응하는지 screenshot으로 확인한다. “기존 카드에 다크 색만 적용”이면 실패다.
3. V2 add/retry/disable/poll/detail과 0/31/100행을 실제 provider-free 브라우저에서 재확인한다.
4. integration branch를 최신 main에 non-force merge한다. 원격이 다시 전진했다면 먼저 fetch하고 새
   origin/main을 통합한 뒤 핵심 검증을 반복한다.
5. main worktree의 사용자 미추적 파일이 그대로이며 commit 대상에 포함되지 않았는지 확인한다.
6. `git push origin main` 후 fetch하여 local main과 `origin/main` commit hash가 같은지 확인한다.
7. production 배포나 Tailscale Funnel 변경은 별도 요청 없이 수행하지 않는다.
