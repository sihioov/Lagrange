Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# 종목 신호 베타 Koyfin형 UI/UX 재개편 실행 계획

> **대체됨:** 서비스 전체 Koyfin 룩앤필 적용은
> `docs/superpowers/plans/2026-09-02-lagrange-koyfin-ui-system.md`를 따른다. 이 계획의
> Stock Beta 전용 작업 결과는 새 전역 디자인 시스템의 제품별 구현으로 유지한다.

기준 명세: `docs/superpowers/specs/2026-09-02-stock-beta-koyfin-ui-overhaul.md`
계획 재작성일: 2026-09-02
상태: 계획만 작성됨, 구현 미실행

## Goal and boundaries

### 목표와 측정 가능한 완료 기준

`/stock-beta`와 `/stock-beta/[instrument]`를 기준 Koyfin 콘셉트의 셸, 밀도, 패널 위계 및
선택 중심 상호작용을 따르는 전용 시장 리서치 워크스페이스로 재개편한다. 기존 결과처럼 일반
Lagrange 셸에 다크 팔레트와 둥근 위젯을 얹는 방식은 완료로 인정하지 않는다.

다음 조건을 모두 충족해야 완료다.

- Stock Beta 경로에서는 일반 Lagrange 헤더·내비게이션·RoutePage 대신 실제 전용 터미널 셸
  변형이 렌더링된다.
- 1280×720에서 약 50px 상단 바, 210~220px 좌측 레일, 한 줄 스냅샷 스트립, 제목·도구 바,
  3패널 첫 행과 하단 보조 행 일부가 동시에 보인다.
- 첫 행은 좌측 순위 보드, 중앙 선택 종목 신호 프로필, 우측 신호 분해로 구성된다.
- 하단에는 조건 매트릭스와 현재 스냅샷 근거 테이프가 보인다.
- 패널 간격은 6~8px, 패널 반경은 4px 이하이며 큰 그림자·그라디언트·대형 정책 카드가 없다.
- 기준 콘셉트와 구현을 같은 1280×720에서 나란히 놓았을 때 상단 바, 좌측 레일, 스트립,
  3패널 첫 행 및 하단 행을 각각 대응시킬 수 있다.
- 현재 DTO에 없는 시세·시장·업종·뉴스·이력·시계열을 만들지 않고, 범위가 보장되지 않은 점수와
  팩터를 0~100 게이지로 정규화하지 않는다.
- 서버 순위, Top 5, 정확한 원본 값, 조건, 팩터 해석, 조건 사유 및 provenance를 보존한다.
- 기존 Owner/Member 권한, 로그인 복구, fail-closed 상태, GET 필터와 안전한 복귀 문맥을 유지한다.
- 위젯은 중앙 registry와 breakpoint layout으로 추가·제거·교체할 수 있고 직접 fetch하지 않는다.
- 한국어·영어, 375×812, 768×1024, 1280×720, 1440×900, 200% 확대, 키보드,
  forced colors 및 reduced motion 검증을 통과한다.
- Web 단위 테스트, 타입 검사, 린트, 프로덕션 빌드 및 전체 Playwright 회귀가 통과한다.

### 대상 workspace와 경로

- 작업 디렉터리: `/data/worktrees/3puw275b/enhanced-pig`
- 요구사항 원본:
  `docs/superpowers/specs/2026-09-02-stock-beta-koyfin-ui-overhaul.md`
- 1차 시각 기준: `docs/ui-concepts/koyfin.html`
- 기준 스타일: `docs/ui-concepts/concepts.css`의 `.koyfin-app`, `.k-*`
- 주요 구현 영역:
  - `apps/web/components/shell/**`
  - `apps/web/components/stock-beta/**`
  - `apps/web/app/(authenticated)/layout.tsx`
  - `apps/web/app/(authenticated)/stock-beta/**`
  - `apps/web/lib/i18n/dictionaries/stock-beta.ts`
  - stock-beta 및 shell 관련 Web 테스트

### 범위 안

- Stock Beta 경로에만 적용되는 route-aware 터미널 셸
- 실제 메뉴 목적지, 권한, locale 및 로그아웃을 보존하는 얇은 상단 바와 좌측 레일
- 실제 DTO를 사용하는 스냅샷 스트립, 순위 보드, 기간별 지표 프로필, 신호 분해,
  조건 매트릭스 및 현재 스냅샷 테이프
- 목록·상세 화면의 검색, 선택, 필터 drawer, 탭 및 안전한 상세/복귀 상호작용
- Stock Beta에 국한된 다크 터미널 토큰과 고밀도 패널 프레임
- 기존 위젯 registry/layout 구조의 확장 또는 필요한 범위의 교체
- 단위·접근성·브라우저 회귀와 기준 콘셉트 비교 시각 검수
- 기존 실패·빈 결과 fixture와 E2E의 보존 및 강화

### 범위 밖

- 백엔드, API, OpenAPI, DTO, 인증 정책 또는 데이터 파이프라인 변경
- KIS, OpenDART 또는 다른 외부 데이터 요청
- 현재가, 1일 등락률, 실시간 데이터, 가격 시계열, 시장 지수, 환율, 업종, 뉴스 또는 이력
- 주문, 계좌, 잔고, 매수·매도, 목표가 또는 거래 기능
- Stock Beta 이외 제품의 시각 개편
- 사용자별 위젯 저장, 드래그앤드롭 및 실제 Add widget/Layout 기능
- Koyfin 상표, 로고, 문구 또는 독점 자산 복제
- `docs/ui-concepts/**` 수정
- dependency 변경
- commit, push, PR, 배포 또는 Tailscale Funnel 변경

### 적용 지침과 우선순위

1. 사용자가 승인한 최신 요구사항 문서와 이번 요청이 Stock Beta의 제품·시각 요구에 우선한다.
2. 루트 `AGENTS.md`의 KIS/OpenDART deny-by-default, 계좌·주문·Live 금지 및 비밀정보 보호 규칙은
   계속 적용된다. 이번 작업은 외부 시장 API를 호출하지 않는다.
3. 주입된 저장소 작업 지침의 모델·effort·상향·검증 규칙을 worker 선택에 적용한다.
4. `apps/web/AGENTS.md`와 `apps/web/CLAUDE.md`에 따라 구현 worker는 코드 작성 전 설치된
   Next.js 16.3 문서를 `node_modules/next/dist/docs/`에서 완전히 읽는다.
5. `apps/web/DESIGN.md`의 권한, landmark, 키보드, 44px target, WCAG 2.2 AA, fail-closed,
   forced colors, reduced motion 및 다른 제품 셸 규칙은 유지한다.
6. 최신 요구사항은 Stock Beta 범위에서 `DESIGN.md`의 일반 셸, 라이트·다크 동등 지원,
   1rem 패널 반경, 넓은 간격, 14px 이상 기본 밀도 및 기존 accent 선택을 명시적으로 대체한다.
   이 예외를 다른 제품으로 확장하지 않는다.
7. 이 계획의 모든 `WP-*`는 `$paseo-delegate`로만 실행한다. native subagent, Task, Agent,
   collaboration 또는 다른 위임 기능으로 대체하지 않는다.

### 확인된 현재 상태와 prerequisite

- 기존 Koyfin 1차 구현과 QA 수정이 working tree에 modified/untracked 상태로 존재한다. 이는
  사용자 작업으로 취급하며 reset, checkout 또는 일괄 삭제하지 않는다.
- 기존 구현의 데이터 계약, filter-context, fixture 및 검증된 fail-closed 동작은 보존 대상이다.
  시각 표현과 셸은 교체 대상이다.
- Next.js는 저장소 루트 `node_modules`에 16.3.0으로 설치되어 있다.
- `$paseo-delegate` 스킬 파일은 `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`에 있다.
  실제 실행 직전에 해당 스킬을 완전히 읽어야 한다. 계획 작성만 하는 현재 턴에는 worker를
  실행하지 않는다.
- 현재 외부 데모와 로컬 fixture가 33001/38182 등을 사용할 수 있다. 실행자는 이를 종료하거나
  Funnel을 변경하지 않고, baseline/E2E에 별도 미사용 포트를 지정한다.
- 제품 요구사항의 미해결 결정은 없다. 구현 중 새 API 필드, 전역 개편 또는 가짜 데이터가
  필요하다는 결론이 나오면 요구를 메우지 말고 affected branch를 중단한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | 공통 Authenticated layout 안에서 Stock Beta만 실제로 다른 셸을 렌더링해야 하며, Server/Client 경계·권한·locale·logout·다른 경로 무회귀를 동시에 보존하는 아키텍처 결정이다 | high | CSS로 기존 셸을 숨기는 방식밖에 없거나, 모든 route 이동·전역 token 변경·직렬화 불가 props가 필요하거나, 구조 테스트가 두 번 실패하면 계획을 중단하고 재설계한다 |
| WP-2 | intermediate | 대시보드의 영역·데이터 매핑·밀도 수치가 확정되어 있고 기존 widget architecture와 검증 가능한 DTO가 있지만 여러 위젯의 선택 상태와 필터를 조정해야 한다 | high | 새 API 필드, 목록 외 detail fetch, client rerank, 무근거 정규화 또는 WP-1 소유 셸 변경이 필요하거나 동일 수정이 두 번 실패하면 hard로 상향한다 |
| WP-3 | intermediate | 상세 DTO와 3영역 구성이 고정되어 있고 기존 filter-context·detail widgets가 있으나 정확한 값 표현과 터미널 재배치에 국소 판단이 필요하다 | high | 목록 API 추가 호출, 가짜 시계열, factor 정규화, 외부 return URL 또는 WP-1/WP-2 파일 재설계가 필요하거나 동일 수정이 두 번 실패하면 hard로 상향한다 |
| WP-4 | hard | 기계적 E2E 외에 기준 콘셉트와 구현의 시각 대응 자체가 핵심 판정이며, 네 viewport·확대·locale·접근성을 실제 브라우저에서 교차 검증해야 한다 | medium | 기준 화면을 재현하지 못하거나, 시각 판정과 DOM 검증이 충돌하거나, 결함이 전역 셸/API 변경을 요구하거나, 두 QA 반복에서도 같은 시각 결함이 남으면 coordinator가 계획을 수정한다 |
| WP-5 | intermediate | 명확한 명세·스크린샷·테스트 evidence를 기준으로 하는 read-only 최종 리뷰이며 수정 권한이 없다 | high | 가짜 데이터, 권한 누출, 일반 셸 잔존, 직접 fetch, rerank 또는 아키텍처 우회가 발견되면 finding을 hard blocker로 분류하고 affected package를 재실행한다 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | Stock Beta 전용 실제 터미널 셸과 공통 시각·위젯 기반 구축 | shell integration, authenticated layout의 제한된 변경, `stock-beta/terminal/**`, `stock-beta/shared/**`, scoped theme, shell/architecture tests | 없음 | gpt-5.6-sol, high; `$paseo-delegate`가 실행 시 availability 재확인 | route-aware shell variant, terminal frame/tokens, real nav/locale/logout, 다른 route 무회귀 tests | focused shell/architecture tests, typecheck, lint, source/DOM inspection |
| WP-2 | 2 | intermediate | Koyfin형 메인 3패널 보드와 하단 2패널, 검색·필터·선택 구현 | dashboard 전부, workspace/main page, dashboard dictionary copy, dashboard tests | WP-1 | gpt-5.6-luna, max; 실패 시 저장소 상향 규칙 | 실제 DTO 기반 5영역 workspace, compact policy/provenance, responsive dashboard | focused unit/surface tests, typecheck, lint, 1280×720 browser capture |
| WP-3 | 3 | intermediate | 동일 터미널 언어의 종목 상세 근거 워크스페이스 구현 | detail 전부, detail component/page, detail dictionary copy, detail tests | WP-1, WP-2 | gpt-5.6-luna, max; 실패 시 저장소 상향 규칙 | 상세 3영역 board, exact factors/reasons/provenance, safe back context | focused unit/surface tests, typecheck, lint, detail browser capture |
| WP-4 | 4 | hard | 시각 수용 기준·반응형·접근성·상태 E2E 구축 및 실행 | stock-beta E2E/fixture, 제한된 accessibility tests, legacy stock-beta CSS cleanup; 제품 결함은 원 package로 반환 | WP-2, WP-3 | gpt-5.6-terra, high; 시각 판정과 테스트 설계를 함께 수행 | updated regression suite, viewport/zoom/locale evidence, 기준·구현 비교 보고 | full unit/type/lint/build, full Playwright, screenshots, DOM/overflow checks |
| WP-5 | 5 | intermediate | 전체 diff와 시각 evidence의 독립 최종 판정 | read-only; mutable ownership 없음 | WP-4 | gpt-5.6-terra, high | findings-first accept/reject review | diff inspection, evidence audit, 선택적 read-only 검사 재실행 |

모든 package가 공통 셸, dictionary 또는 앞선 화면 결과에 의존하므로 병렬 wave를 두지 않는다.
WP-2와 WP-3을 격리 workspace에서 병렬화하면 dictionary·surface test·terminal slots 통합 비용이
더 크다. 각 wave는 coordinator가 통합한 뒤 다음 wave로 진행한다.

## Worker briefs

### WP-1 — Stock Beta 전용 터미널 셸과 공통 기반

- **Package ID:** `WP-1`
- **Target working directory:** `/data/worktrees/3puw275b/enhanced-pig`
- **Initial complexity:** `hard`
- **Rationale:** 현재 `AuthenticatedLayout`이 모든 경로를 하나의 `AppShell`로 감싸므로
  Stock Beta만 실제로 다른 셸을 렌더링하려면 Next layout과 Client pathname 경계를 안전하게
  결정해야 한다.
- **Confidence:** `high`
- **Escalation signals:** CSS `:has()`로 기존 셸을 시각적으로만 숨기거나, 동일 control을 두 번
  DOM에 렌더링하거나, 다른 authenticated route를 이동해야 하거나, ReactNode/아이콘을
  직렬화할 수 없거나, shell regression이 두 번 실패하면 수정 범위를 넓히지 말고 보고한다.

#### Objective and known facts

Stock Beta 경로에서 일반 shell chrome을 DOM에 남긴 채 재색칠하지 않고, 실제 Koyfin형 전용
terminal chrome을 렌더링할 수 있는 구조를 만든다. 실제 Lagrange 메뉴 목적지, role별 노출,
LocaleProvider, skip link, logout 및 Owner/Member 권한은 유지한다. 다른 authenticated route는
기존 AppShell DOM과 시각 동작을 그대로 유지해야 한다.

현재 사실:

- `apps/web/app/(authenticated)/layout.tsx`가 session, locale, theme을 읽어 `AppShell`을 렌더링한다.
- `PrimaryNavigation`은 `usePathname()`으로 현재 route를 판단한다.
- Stock Beta 페이지 내부의 `OwnerBetaProductRoute`가 데이터 렌더링 전 Owner 권한을 재검증한다.
- 상단 검색과 스냅샷 값은 page DTO가 있어야 하므로 terminal shell은 data-aware slot 또는 명확한
  composition 경계를 제공해야 한다.
- Stock Beta는 완성된 다크 터미널이 기준이며 작동하지 않는 theme toggle을 표시하지 않는다.

#### Exact owned scope

- `apps/web/components/shell/app-shell.tsx`
- 필요 시 `apps/web/components/shell/primary-navigation.tsx`
- `apps/web/app/(authenticated)/layout.tsx`
- 새 `apps/web/components/stock-beta/terminal/**`
- `apps/web/components/stock-beta/shared/**`
- `apps/web/components/stock-beta/stock-beta-theme.module.css`
- `apps/web/components/stock-beta/README.md`
- Stock Beta에만 적용되는 최소 `apps/web/app/globals.css` integration rule
- 현재 terminal 전환 때문에 더 이상 필요 없는 경우에 한해
  `apps/web/components/pages/route-page.tsx`의 기존 Stock Beta용 확장 정리
- `apps/web/tests/shell-runtime.test.ts`
- `apps/web/tests/role-navigation.test.tsx`의 관련 사례
- `apps/web/tests/stock-beta-widget-architecture.test.tsx`
- 필요 시 새 terminal-shell focused test

기존 파일의 사용자 변경을 통째로 교체하지 말고 관련 hunk만 수정한다.

#### Prohibited adjacent work

- dashboard data widgets와 detail data widgets 구현 금지
- 다른 authenticated page 이동 또는 route group 대규모 재배치 금지
- 다른 제품의 CSS·token·theme·navigation 시각 변경 금지
- API, DTO, auth policy, data client, backend 변경 금지
- CSS로 일반 shell을 숨기고 중첩 terminal shell을 보이는 임시 해법 금지
- 가짜 상단 시세·시장 상태·alert·export·layout control 추가 금지
- package 설치, 외부 network, production/Funnel, commit/push 금지
- 하위 agent 위임 금지

#### Inputs and dependencies

- 최신 요구사항 문서 전체, 특히 §2, §3, §5, §9, §11, §14
- `apps/web/AGENTS.md`, `apps/web/CLAUDE.md`, `apps/web/DESIGN.md`
- `apps/web/components/shell/**`, authenticated layout, session/locale/theme helpers
- 현재 widget registry/layout과 `WidgetFrame`
- 구현 전 다음 설치본 문서를 완전히 읽고 정확한 경로를 보고한다.
  - `node_modules/next/dist/docs/01-app/01-getting-started/03-layouts-and-pages.md`
  - `node_modules/next/dist/docs/01-app/01-getting-started/05-server-and-client-components.md`
  - `node_modules/next/dist/docs/01-app/01-getting-started/04-linking-and-navigating.md`
  - `node_modules/next/dist/docs/01-app/01-getting-started/11-css.md`
  - route group을 제안하는 경우 project structure 문서

#### Expected output

- Stock Beta route에서만 선택되는 실제 terminal shell variant
- 48~52px utility bar, 210~220px desktop rail, 52~56px tablet rail 및 mobile compact nav 기반
- real navigation, role context, locale action, logout, skip link와 stable landmarks
- page가 검색·as-of·snapshot strip·title tools를 주입할 typed slots 또는 동등한 명시적 경계
- Stock Beta 전용 dark tokens와 2~4px panel frame; 다른 route token 무변경
- 직렬화 가능한 widget props, registry validation 및 no-direct-fetch 경계
- required widget/duplicate ID/invalid layout을 거부하는 architecture tests
- widget 추가·제거 절차와 route shell 예외를 설명하는 README

#### Verification

- `cd apps/web && npm test -- shell-runtime.test.ts role-navigation.test.tsx stock-beta-widget-architecture.test.tsx`
- `cd apps/web && npm run typecheck`
- `cd apps/web && npm run lint`
- Stock Beta server/client render에서 일반 shell header/nav가 함께 렌더링되지 않는 DOM 검사
- Dashboard 등 비 Stock Beta route의 기존 shell snapshot/role navigation 무회귀 검사
- `git diff --check`

#### Required report

- 변경한 파일과 라인 범위
- brief와 다르게 처리한 부분 및 이유
- 선택한 route-aware shell 구조와 버린 대안
- 실행한 검사·명령과 결과
- 다른 route 무회귀 evidence
- 미해결 또는 후속 작업; 없으면 `none`
- 찾지 못했거나 확인하지 못한 항목; 없으면 `none`
- 읽은 Next 16.3 로컬 문서의 정확한 경로

### WP-2 — Koyfin형 메인 신호 워크스페이스

- **Package ID:** `WP-2`
- **Target working directory:** `/data/worktrees/3puw275b/enhanced-pig`
- **Initial complexity:** `intermediate`
- **Rationale:** 영역, 수치, 데이터 매핑과 수용 기준이 확정되어 있고 기존 registry와 fixture가
  있지만 5개 영역의 공유 selection, 검색, URL filter 및 server/client 경계를 함께 맞춰야 한다.
- **Confidence:** `high`
- **Escalation signals:** dashboard가 detail API를 호출해야 하거나, 서버 rank를 재계산해야 하거나,
  score/factor를 정규화해야 하거나, WP-1 shell 파일을 재설계해야 하거나, 같은 수정이 두 번
  실패하면 중단하고 `hard` 재분류를 요청한다.

#### Objective and known facts

WP-1 terminal shell 안에 기준 콘셉트와 대응되는 메인 워크스페이스를 구현한다. 색상만 바꾸지
않고 셸 아래 snapshot strip, title/tool bar, 3패널 첫 행, 2패널 하단 행을 만든다. latest와
screen 응답을 그대로 사용하며 선택된 row 이외의 detail data를 가져오지 않는다.

현재 사실:

- latest DTO는 `rows`, 서버 `top5`, `provenance`를 제공한다.
- screen DTO는 현재 필터 결과 `rows`, `provenance`만 제공한다.
- row에는 rank, score, condition, 20/60/120 return·volatility, drawdown, SMA, volume 및
  activity 값이 있다.
- score와 factor의 0~100 범위는 계약에 없다.
- 기존 filter parser/body mapper와 safe detail context는 검증되어 있다.
- 성공 DTO에는 별도 integrity status가 없고 integrity는 오류 코드로만 표현된다.

#### Exact owned scope

- `apps/web/components/stock-beta/dashboard/**`
- `apps/web/components/stock-beta/stock-beta-workspace.tsx`
- `apps/web/app/(authenticated)/stock-beta/page.tsx`
- `apps/web/lib/i18n/dictionaries/stock-beta.ts`의 dashboard와 shared copy
- `apps/web/tests/stock-beta-dashboard.test.tsx`
- `apps/web/tests/owner-beta-equity-signals-surface.test.tsx`의 dashboard 사례
- 필요 시 dashboard focused test 추가

WP-1의 terminal/shared/theme files는 소비만 한다. 결함이 있으면 직접 수정하지 않고 coordinator가
WP-1 `$paseo-delegate` follow-up을 결정한다.

#### Prohibited adjacent work

- detail component/page/widget 수정 금지
- shell/AppShell/authenticated layout/global token 수정 금지
- API contract, server product client, auth/role policy 수정 금지
- current price, 1D return, market tape, sector, news, signal history 또는 fake chart 추가 금지
- 연결 선으로 20/60/120 horizon을 시계열처럼 표현 금지
- score/factor 원형 게이지·percent·probability·무근거 progress bar 금지
- client sorting/reranking, stale/synthetic fallback, widget별 fetch 금지
- 목업, dependency, 외부 network, production/Funnel, commit/push 수정 금지
- 하위 agent 위임 금지

#### Inputs and dependencies

- 통합되고 검증된 WP-1 결과
- 최신 요구사항 §4~§7, §9~§12, §14~§15
- DTO와 filter parser/body mapper
- 기존 dashboard registry/layout/selection provider 및 fail-closed page
- 구현 전 다음 설치본 문서를 완전히 읽고 경로를 보고한다.
  - Server and Client Components
  - Linking and Navigating
  - CSS
  - 현재 설치본의 page/searchParams 및 form 관련 App Router 문서

#### Expected output

- 실제 값만 사용하는 6-cell snapshot strip: AS OF, UNIVERSE, BULLISH, NEUTRAL, BEARISH,
  registration/publication/materialization 상태
- compact title/tool bar와 현재 응답 종목만 검색하는 keyboard search
- drawer/popover형 전체 GET filter와 한 줄 active filter chips
- 좌측 dense ranking: 약 34~40px row, exact server order, latest Top 5, filtered-result copy,
  keyboard selection, 별도 detail action
- 중앙 signal profile: return·volatility·activity tabs, zero-axis bar/dot plot, exact labels,
  SMA/drawdown metric strip
- 우측 decomposition: raw score, condition text, exact row metrics, BETA/READ ONLY
- 하단 fixed-area condition matrix: server order, equal tile area, condition label, ranking selection 동기화
- 하단 current snapshot tape: latest server Top 5 또는 filtered current-result leaders, no fake time/change
- 기본 선택은 명세 규칙대로 latest `top5[0]` 또는 `rows[0]`, filtered `rows[0]`
- compact always-visible policy summary와 accessible full policy/provenance disclosure
- empty/invalid/unavailable/integrity/generic failure에서 stale rows 없음

#### Verification

- `cd apps/web && npm test -- stock-beta-dashboard.test.tsx stock-beta-widget-architecture.test.tsx owner-beta-equity-signals-surface.test.tsx`
- `cd apps/web && npm run typecheck`
- `cd apps/web && npm run lint`
- latest와 filtered server-rendered markup에서 label/count/top rows 대조
- 1280×720 Chromium capture에서 상단 bar/rail/strip/title/3-panel first row/second row 일부 확인
- 기준 콘셉트 1280×720 capture와 나란히 두고 구조 대응 체크
- `git diff --check`

#### Required report

- 변경한 파일과 라인 범위
- brief와 다르게 처리한 부분 및 이유
- 각 콘셉트 패널을 실제 DTO로 치환한 방식
- 실행한 검사·명령과 결과
- 1280×720 screenshot 경로와 시각 checklist 결과
- 미해결 또는 후속 작업; 없으면 `none`
- 찾지 못했거나 확인하지 못한 항목; 없으면 `none`
- 읽은 Next 16.3 로컬 문서의 정확한 경로

### WP-3 — Koyfin형 종목 상세 근거 워크스페이스

- **Package ID:** `WP-3`
- **Target working directory:** `/data/worktrees/3puw275b/enhanced-pig`
- **Initial complexity:** `intermediate`
- **Rationale:** 상세 DTO와 필수 데이터가 고정되어 있고 기존 detail registry가 있지만 메인과
  동일한 terminal visual grammar로 재구성하면서 raw factor·reason·provenance를 손실 없이
  유지해야 한다.
- **Confidence:** `high`
- **Escalation signals:** condition distribution을 위해 list API가 필요하거나, factor scale을
  발명해야 하거나, 외부 return URL·새 DTO·WP-1 shell 재설계가 필요하거나, 동일 수정이 두 번
  실패하면 중단하고 `hard` 재분류를 요청한다.

#### Objective and known facts

일반 RoutePage와 세로 카드로 되돌아가지 않는 상세 terminal workspace를 구현한다. detail API의
`signal`, `factor_explanations`, `condition_reasons`, `provenance`만 사용하고 목록 필터 문맥은
허용된 query key만 재구성해 보존한다.

현재 사실:

- detail DTO는 전체 universe count나 condition distribution을 제공하지 않는다.
- factor는 이름, finite raw value, interpretation만 제공하며 fixed scale이 없다.
- condition reasons는 API 순서와 원문을 보존해야 한다.
- 기존 filter-context는 허용된 stock-beta query만 back link에 유지한다.
- not-found는 `RESOURCE_NOT_FOUND`, integrity는 별도 실패 코드로 fail closed한다.

#### Exact owned scope

- `apps/web/components/stock-beta/detail/**`
- `apps/web/components/stock-beta/stock-beta-detail.tsx`
- `apps/web/app/(authenticated)/stock-beta/[instrument]/page.tsx`
- `apps/web/lib/i18n/dictionaries/stock-beta.ts`의 detail copy
- `apps/web/tests/owner-beta-equity-signals-surface.test.tsx`의 detail 사례
- 필요 시 detail focused test 추가

WP-1 terminal/shared/theme와 WP-2 dashboard는 소비만 한다. 문제가 있으면 coordinator에게
반환한다.

#### Prohibited adjacent work

- dashboard registry/layout/widgets 수정 금지
- AppShell/authenticated layout/global tokens 수정 금지
- list/latest API를 장식용 count 때문에 추가 호출 금지
- API/auth/backend/filter contract 수정 금지
- price chart, current price, 1D return, sector, market/news/history 추가 금지
- raw factor 정규화, gauge/progress/percent/probability 변환 금지
- arbitrary return URL, open redirect 또는 unvalidated query reflection 금지
- 목업, dependency, 외부 network, production/Funnel, commit/push 수정 금지
- 하위 agent 위임 금지

#### Inputs and dependencies

- 통합된 WP-1과 WP-2 결과
- 최신 요구사항 §4, §8~§13, §15
- detail DTO, safe filter context, 기존 error/OwnerBetaProductRoute tests
- 구현 전 설치본 Next 문서에서 Server/Client Components, Linking and Navigating, CSS,
  dynamic page params/searchParams 관련 문서를 완전히 읽고 경로를 보고한다.

#### Expected output

- terminal context bar: safe back, instrument name/ID, rank, raw score, condition, AS OF
- detail strip: rank, condition, registration/publication/materialization 및 Read only; fake universe count 없음
- 데스크톱 3영역:
  - 20/60/120 return·volatility와 max drawdown exact comparison
  - SMA, average volume, volume ratio 및 activity proxy
  - raw factor value·interpretation과 exact condition reasons
- raw value를 항상 텍스트로 제공하고 scale을 암시하지 않는 시각 표현
- full policy와 모든 provenance hash에 접근 가능한 disclosure
- safe filtered back link와 direct-load fallback
- not-found/unavailable/integrity/login/Member에서 stale detail 없음
- tablet/mobile ordering과 page-level horizontal overflow 없음

#### Verification

- `cd apps/web && npm test -- owner-beta-equity-signals-surface.test.tsx stock-beta-widget-architecture.test.tsx`
- 필요 시 새 detail test 포함 focused Vitest
- `cd apps/web && npm run typecheck`
- `cd apps/web && npm run lint`
- 정상·not-found·integrity detail server-render inspection
- 1280×720과 375×812 detail browser capture
- `git diff --check`

#### Required report

- 변경한 파일과 라인 범위
- brief와 다르게 처리한 부분 및 이유
- exact factor/reason/provenance 보존 근거
- 실행한 검사·명령과 결과
- desktop/mobile screenshot 경로와 overflow 결과
- 미해결 또는 후속 작업; 없으면 `none`
- 찾지 못했거나 확인하지 못한 항목; 없으면 `none`
- 읽은 Next 16.3 로컬 문서의 정확한 경로

### WP-4 — 통합 브라우저·시각·접근성 QA

- **Package ID:** `WP-4`
- **Target working directory:** `/data/worktrees/3puw275b/enhanced-pig`
- **Initial complexity:** `hard`
- **Rationale:** 기능 회귀뿐 아니라 Koyfin 기준과의 시각 대응이 주요 산출물이며 여러 viewport,
  확대, locale과 보조기술 상태를 실제 브라우저에서 판단해야 한다.
- **Confidence:** `medium`
- **Escalation signals:** local reference를 렌더링할 수 없거나, Playwright harness가 요구 viewport를
  재현하지 못하거나, visual acceptance와 접근성 규칙이 충돌하거나, 제품 수정이 WP-1~WP-3
  소유권을 침범하면 직접 고치지 말고 finding으로 반환한다.

#### Objective and known facts

WP-1~WP-3 통합 결과를 synthetic fixture에서 검증하고 요구사항 §14의 시각 체크리스트를 독립적으로
판정한다. 테스트는 스크린샷 존재만 확인하지 않고 구조, geometry, overflow, text, selection 및
fail-closed DOM을 함께 검증한다.

현재 사실:

- `scripts/qa/candidate-web-e2e.sh`는 synthetic API와 Next dev를 띄워 전체 `tests/e2e/`를 직렬
  실행한다.
- 기본 33001/38182는 외부 데모가 사용 중일 수 있으므로 환경 변수로 별도 포트를 지정할 수 있다.
- Playwright 기본 프로젝트는 Chromium 1280×900이며 테스트별 viewport override가 가능하다.
- 기준 콘셉트는 local `docs/ui-concepts/koyfin.html`과 CSS/JS로 외부 데이터 없이 렌더링할 수 있다.
- Stock Beta는 dark terminal이 기준이며 light parity는 완료 조건이 아니다. 다른 route의 기존
  theme toggle 무회귀는 확인한다.

#### Exact owned scope

- `apps/web/tests/e2e/stock-beta.spec.ts`
- `apps/web/tests/e2e/support/stock-beta-fixture.mjs`
- 필요 시 새 stock-beta visual/accessibility E2E file
- `apps/web/tests/accessibility.test.tsx`의 Stock Beta 사례
- `apps/web/app/product.css`의 확인된 legacy Stock Beta dead rules만 정리
- QA artifact는 Playwright output 또는 `/tmp`; binary screenshot을 Git에 추가하지 않는다.

제품 소스 결함은 직접 수정하지 않는다. coordinator가 원 소유 WP에 `$paseo-delegate` follow-up을
보낸 뒤 WP-4를 다시 실행한다.

#### Prohibited adjacent work

- `apps/web/components/stock-beta/**`, shell, page, dictionary 직접 수정 금지
- API/backend/auth/global token/다른 product CSS 수정 금지
- 의미 없는 pixel-perfect golden만으로 승인 금지
- 기준 목업의 가짜 데이터를 제품 fixture로 복사 금지
- 동작 중인 demo process 종료, Funnel/Serve 변경, 외부 provider 접근 금지
- dependency, production, commit/push 변경 금지
- 하위 agent 위임 금지

#### Inputs and dependencies

- 통합되고 gate를 통과한 WP-1~WP-3 결과와 각 report
- 최신 요구사항 전체, 특히 §10, §12~§15
- 기존 synthetic API, stock-beta fixture, Playwright config와 QA wrapper
- 설치본 Next.js testing/Playwright 가이드를 완전히 읽고 경로를 보고한다.

#### Expected output

- Owner latest, filtered, detail 정상 상태의 E2E
- Member refusal과 데이터 비노출
- filter submit/preserve/clear, current-result wording, keyboard search/row/matrix selection,
  detail/back navigation
- invalid filter, empty result, unavailable, integrity failure, detail not-found, generic failure에서
  stale 숫자 없음
- 375×812, 768×1024, 1280×720, 1440×900과 200% 확대에서 page overflow/content loss 검사
- 한국어·영어의 clipping과 accessible name 검사
- keyboard-only focus order, visible focus, semantic table/landmarks, 44px effective target 검사
- forced colors와 reduced motion에서 의미 보존
- 기준 콘셉트와 구현의 같은 1280×720 screenshots 및 항목별 대응 판정
- Dashboard 등 비 Stock Beta route의 일반 shell/theme 회귀 없음
- dead legacy CSS 정리 evidence

#### Verification

- `cd apps/web && npm test`
- `cd apps/web && npm run typecheck`
- `cd apps/web && npm run lint`
- `cd apps/web && npm run build`
- repo root에서 unused alternate ports를 지정해 `scripts/qa/candidate-web-e2e.sh`
- `git diff --check`
- reference/implementation screenshots와 viewport별 measured overflow 결과 기록

#### Required report

- 변경한 파일과 라인 범위
- brief와 다르게 처리한 부분 및 이유
- 실행한 검사·명령과 결과
- viewport·zoom·locale·keyboard·forced-colors·reduced-motion별 evidence
- 기준 화면과 구현 화면 screenshot 경로 및 §14 체크리스트 각 항목 pass/fail
- 발견해 원 package로 반환한 제품 결함; 없으면 `none`
- 미해결 또는 후속 작업; 없으면 `none`
- 찾지 못했거나 확인하지 못한 항목; 없으면 `none`
- 읽은 Next 16.3 로컬 문서와 사용한 harness의 정확한 경로

### WP-5 — 독립 최종 리뷰

- **Package ID:** `WP-5`
- **Target working directory:** `/data/worktrees/3puw275b/enhanced-pig`
- **Initial complexity:** `intermediate`
- **Rationale:** 최신 명세, 코드 diff, 테스트 및 시각 evidence를 대조하는 read-only 리뷰이며
  체크리스트가 확정되어 있다.
- **Confidence:** `high`
- **Escalation signals:** 권한·데이터 경계 침범, 일반 셸 잔존, 가짜 데이터, rerank, 직접 fetch,
  필수 widget 우회 또는 비 Stock Beta 회귀가 발견되면 hard blocker로 보고하고 수정하지 않는다.

#### Objective and known facts

구현을 수정하지 않고 전체 결과를 독립 검토한다. test pass 수나 자신 있는 설명만으로 승인하지
않고 실제 source, DOM contract, screenshots와 명세 §14 체크리스트를 대조한다.

#### Exact owned scope

- read-only; mutable file ownership 없음
- `apps/web/**` 최종 diff와 새 파일
- 최신 요구사항, 본 계획, WP-1~WP-4 reports와 QA artifacts
- root/web instructions 및 `apps/web/DESIGN.md`의 적용·override 경계

#### Prohibited adjacent work

- 어떤 파일도 수정하지 않는다.
- failed check나 missing screenshot을 추정으로 통과시키지 않는다.
- 원 package 대신 결함을 직접 수정하지 않는다.
- native subagent, 외부 network/provider, production/Funnel 또는 credential 사용 금지

#### Inputs and dependencies

- WP-4까지 통합된 working tree
- 모든 package report와 검증 명령 결과
- 기준·구현 동일 viewport screenshots
- 최신 spec의 전체 수용 기준

#### Expected output and verification

findings-first 형식으로 다음을 검토한다.

- Stock Beta가 실제 route-specific terminal shell을 사용하고 일반 shell을 CSS로만 숨기지 않는가
- 기준 화면과 shell/grid/density/panel hierarchy가 대응하는가
- 큰 둥근 카드, 넓은 여백, 대형 정책 카드 및 기존 세로 dashboard가 남지 않았는가
- API/backend/auth/KIS/OpenDART 경계 침범이 없는가
- 현재 DTO에 없는 data, fake time/history, score/factor normalization이 없는가
- 서버 rank/Top 5/exact values/factor/reasons/provenance가 보존되는가
- widget이 직접 fetch하거나 client rerank하지 않는가
- required widget과 fail-closed 상태를 registry/layout에서 우회할 수 없는가
- 검색·filter·selection·detail/back context가 같은 데이터 문맥을 유지하는가
- 모든 viewport/zoom/locale/accessibility evidence가 실제로 존재하는가
- 비 Stock Beta route의 기존 shell/theme/role navigation이 회귀하지 않았는가
- 실행 명령과 결과가 재현 가능하고 실패가 숨겨지지 않았는가

#### Required report

- severity 순 findings와 파일/라인 및 screenshot 근거; 없으면 `none`
- 요구사항에서 벗어난 구현과 영향; 없으면 `none`
- 확인한 test/visual evidence와 직접 재실행한 read-only 검사
- 미해결 또는 후속 작업; 없으면 `none`
- 찾지 못했거나 확인하지 못한 항목; 없으면 `none`
- 최종 `ACCEPT` 또는 `REJECT`와 근거

## Coordinator gates

### 1. Pre-launch checks

1. 실제 실행 턴에서 `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`를 완전히 읽고 모든 WP를
   그 스킬로만 시작·모니터링·복구·수집한다. 사용할 수 없으면 native subagent로 대체하지 않고
   중단한다.
2. `git status --short`와 baseline diff를 기록한다. 현재 modified/untracked Stock Beta 구현,
   spec, plan 및 `docs/ui-concepts/**`를 사용자 변경으로 보존한다.
3. 최신 spec과 plan의 hash 또는 diff를 기록해 worker가 구 명세/구 계획을 사용하지 않게 한다.
4. repo-root `node_modules/next`가 16.3.0이고 required docs가 읽히는지 확인한다.
5. 실행 중인 public demo를 유지한다. baseline unit/E2E는 33001/38182가 아닌 확인된 미사용 포트를
   지정하고 Funnel/Serve 설정을 건드리지 않는다.
6. focused Stock Beta unit, typecheck, lint와 가능한 baseline browser capture를 기록한다. 기존
   화면이 시각 기준을 실패하는 것은 예상 baseline이며 기능 실패와 구분한다.
7. 각 WP 실행 전 owned scope가 실제 tree와 일치하는지 확인하고 overlap이 생기면 wave를
   병렬화하지 말고 계획을 먼저 개정한다.
8. 추가 사용자 결정은 현재 없다. worker가 범위 밖 결정이 필요하다고 보고하면 affected branch를
   멈추고 계획을 개정한다.

### 2. Per-wave integration gates

1. **WP-1 gate:** coordinator가 route selection 구조, terminal slots, real nav/locale/logout,
   no-duplicate-shell DOM, scoped tokens, registry invariants를 직접 읽는다. Dashboard 등 비 Stock Beta
   shell tests가 통과하지 않거나 CSS-only hide라면 거부한다.
2. **WP-2 gate:** focused tests만 보지 않고 1280×720 screenshot을 기준 콘셉트와 나란히 본다.
   50px bar, 210~220px rail, snapshot strip, 3-panel first row, lower row 일부, 6~8px gaps 및
   ≤4px radius 중 하나라도 명백히 없으면 WP-3 전에 WP-2 follow-up을 실행한다.
3. **WP-2 data gate:** latest Top 5와 filtered leaders 문구, raw score/metrics, no fake data,
   no normalization, no rerank, no direct fetch, policy/provenance 접근을 source와 markup으로 확인한다.
4. **WP-3 gate:** detail이 일반 RoutePage/세로 카드로 회귀하지 않고 terminal 3영역을 사용하는지,
   list API 추가 호출 없이 raw factors/reasons/provenance와 safe back context를 보존하는지 확인한다.
5. **WP-4 gate:** full unit/type/lint/build와 전체 Playwright 결과, 모든 viewport/zoom/locale/a11y
   evidence, 기준·구현 screenshots를 확인한다. §14의 시각 체크 하나라도 fail이면 완료로 보지 않고
   원 package에 `$paseo-delegate` follow-up 후 WP-4를 재실행한다.
6. **WP-5 gate:** reviewer가 `ACCEPT`하지 않으면 finding을 원 package에 `$paseo-delegate`
   follow-up하고 affected integration gate부터 다시 실행한다. reviewer는 직접 수정하지 않는다.

### 3. Final end-to-end acceptance checks

1. `git diff --check`와 `git status --short`로 의도한 파일만 변경됐는지 확인한다.
2. API/OpenAPI/backend/auth/KIS/OpenDART, 다른 제품, dependency 및 `docs/ui-concepts/**`에 범위 밖
   변경이 없는지 직접 대조한다.
3. Stock Beta에서 일반 shell이 DOM에 중복되지 않고 다른 제품에서는 기존 shell이 유지되는지
   확인한다.
4. 동일 1280×720 기준·구현 screenshots를 최종 나란히 검토하고 spec §14 체크리스트를 모두
   명시적으로 기록한다. “다크 테마만 적용” 판정이면 자동 실패다.
5. 375×812, 768×1024, 1280×720, 1440×900 및 200% 확대에서 page overflow, clipping,
   scroll ownership과 content loss가 없는지 확인한다.
6. latest, filtered, empty, invalid, unavailable, integrity, generic error, detail normal/not-found와
   Owner/Member 상태를 synthetic fixture로 확인한다.
7. 현재 DTO의 모든 필요한 지표와 상세 factor/reason/provenance가 접근 가능하고 제공되지 않은
   데이터, fake history/time 및 unbounded gauge가 없는지 확인한다.
8. 검색, filter chips/drawer, ranking/matrix selection, metric tabs, detail/back navigation을
   keyboard와 pointer로 확인한다.
9. registry/layout에서 선택 widget 추가·제거가 국소적이고 필수 widget 제거는 검증에서
   거부되는지 확인한다.
10. Web full unit, typecheck, lint, production build 및 전체 Playwright 결과를 기록한다.
11. 외부 demo 검수는 worker가 수행하지 않는다. 사용자가 실행을 요청한 턴의 권한과 기존
    인프라를 coordinator가 다시 확인한 뒤 별도 수행하며, 기존 8443 handlers를 덮어쓰지 않는다.
12. 실패, 미검증 또는 접근성 debt가 남으면 완료라고 보고하지 않는다.
13. 사용자 별도 요청 없이는 commit, push, PR, deployment 또는 Funnel 변경을 수행하지 않는다.

이 문서는 계획만 제공한다. 실제 실행 시 모든 `WP-*`는 당시의 `$paseo-delegate` 지침을 source of
truth로 사용해야 하며, native subagent 또는 다른 delegation 기능으로 대체할 수 없다.
