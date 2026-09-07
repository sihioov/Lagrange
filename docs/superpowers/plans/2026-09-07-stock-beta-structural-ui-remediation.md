문서 성격: 2026-09-07에 수행한 실행 계획과 결과를 보존하는 역사적 기록
역사적 실행 방식: worker package는 `$paseo-delegate`를 사용했고 native subagent는 사용하지 않았다.

아래 목표·패키지·검증·gate 항목은 이 기록에 포함된 실행 요구사항이다. 역사적 완료 HEAD에서
적용이 끝났으며, 현재 작업에서 다시 실행하라는 지시가 아니다.

# Stock Beta 구조·반응형 UI 개선 실행 계획

작성일: 2026-09-07
상태: 계획된 구조 개선 및 기능 QA 완료 — 기존 데스크톱 차트 가시성 한계 별도 기록
역사적 완료 HEAD: `6d9f9eb717e7068bd10e7cde9b182b5cea6ee72b`
현재 통합 대상: `14f5bd5` (역사적 완료 HEAD를 포함; coordinator가 제공한 기준)

### 실행 기록

이 기록에서 coordinator의 직접 확인은 명시적으로 그렇게 적고, worker/WP-6의 명령·수치와
그 밖의 결과는 당시 실행 보고로 구분한다. baseline 수치와 최종 수치는 각각 해당 실행 시점의
기록이며 서로 다른 검증 단계다.

- 기준 HEAD 및 clean state 확인: `e4585864c4db60a7458e0167d75f469ab152b529`.
- `$paseo-delegate`를 사용자 제공 경로에서 읽었으며 CLI와 계획의 Codex 모델 가용성을 확인했다.
- WP-1: 역사적 delegated worker, Codex `gpt-5.6-sol` high (worker ID와 절대 worktree 경로는
  기록에서 제외).
- WP-2: 역사적 delegated worker, Codex `gpt-5.6-luna` max (worker ID와 절대 worktree 경로는
  기록에서 제외).
- WP-3: 역사적 delegated worker, Codex `gpt-5.6-luna` max (worker ID와 절대 worktree 경로는
  기록에서 제외).
- WP-4: 역사적 delegated worker, Codex `gpt-5.6-luna` low (worker ID와 절대 worktree 경로는
  기록에서 제외).
- WP-4 완료: worker commit `9478eb7`, integration commit `c6fc86c`. coordinator가 네 파일의
  diff를 직접 확인하고 통합 후 typecheck와 shell Vitest 9/9 통과를 확인했다.
- WP-2 완료: `faaf80e` 및 새 ID 추가/삭제된 선택/공백 ID 후속 테스트 `b12a228`을
  `d5fe994`, `eb1b2cc`로 통합했다. coordinator typecheck와 chart surface 23/23 통과.
- WP-1 완료: `14615fc`를 `eb9b652`로 통합했다. coordinator typecheck 및 Widget/탭/차트
  통합 scoped Vitest 57/57 통과. 당시 남아 있던 독립 구조 리뷰와 최종 QA는 아래 후속 기록에서
  완료됐다.
- WP-3 재실행 담당은 Codex gpt-5.6-terra medium으로 전환했다. 중간 실행은 CLI가 상속한 root
  workspace 때문에 source edit 전에 중단했고, 재실행은 명시적인 격리 workspace에서 수행해
  경로 상속을 방지했다. transient 실행 ID와 worktree 경로는 기록에서 제외했다.
- WP-3 완료: `40ff9ee`를 `bad5015`로 통합했다. 네 package 통합 후 coordinator typecheck와
  navigation/Widget/탭/차트 scoped Vitest 66/66 통과. WP-5 독립 리뷰로 넘겼다.
- WP-5 1차 REJECT: derived layout의 unknown
  breakpoint 거부 누락과 nav client transition/page-scroll/nested clipping 회귀 검증 누락.
  WP-1에 validator+architecture test 두 파일 보완, WP-3에 기존 owned scope 내 E2E 및 필요
  responsive/nav 보완을 반환했다. 두 후속은 파일이 겹치지 않는다. 1360/1361/1400px와
  pathname이 같은 locale 변경도 확인했다. 통합 및 WP-5 재승인 후 WP-6을 진행했다.
- WP-1 review 보완 완료: `4aa7727`을 `a64285c`로 통합했다. unknown derived layout key를
  validator와 serializable projection 모두 거부한다. coordinator typecheck 및 두 suite
  35/35 통과, clean state 확인.
- WP-3 review 보완 `1640803`을 `6d9f9eb`로 통합했다. membership container query, locale
  label 변화 반영, client transition/nav-only 측정 및 nested clipping 검사를 추가했다.
  coordinator typecheck 및 네 suite 67/67 통과. clean state에서 WP-5 재리뷰를 진행했다.
- WP-5 재리뷰 ACCEPT (`6d9f9eb`): 두 finding FIXED, 새 acceptance blocker 없음.
  내부 UI 변경으로 API/deploy/diagram edge 변경 없음. WP-6 production browser QA를 이어서
  수행했다.
- WP-6: Codex gpt-5.6-terra medium, source read-only. API/app QA에는 전용 localhost port
  pairs와 local evidence를 사용했으며, transient 번호·임시 경로는 기록에서 제외했다.
- WP-6 automated ACCEPT (`6d9f9eb`): typecheck/lint exit 0, Vitest 271/271,
  fresh production focused Playwright 29/29 두 번(33.6s/33.3s), full Web 74/74(56.3s).
  nav identity/locale/client transition, outline 경계 수치 확인. coordinator가 다섯 screenshot과
  metric을 직접 열어 검토했다. 데스크톱 차트의 내부 scroll 가시성은 screenshot에서 별도 제약으로
  발견되어 같은 담당에게 evidence-only 추가 확인을 요청했고 후속 측정으로 완료했다. source
  변경/재설계는 하지 않았다.
- 최종 추가 측정 완료: 정확한 candle body 261개, Tab focus 캡처 검토. 기존 profile panel 높이
  128/175px가 SVG 225.7/262.4px보다 작아 전체 그래프 동시 보기는 불가능한 기존 UX 한계로
  분리했다. `.profilePanel` CSS는 baseline부터 동일하다. 차트 구간 내부 스크롤 접근은 가능하다.
  기능 QA 통과를 모든 시각 문제 해결로 확대 해석하지 않는다.
- 최종 HEAD `6d9f9eb717e7068bd10e7cde9b182b5cea6ee72b`, integration clean 및 diff check 통과.
  QA 전용 네 포트 listener 없음, `.next`/test-results/node_modules 임시 링크 없음.
  역사적 QA 종료 시점에는 main 머지·push·배포를 하지 않았다. 현재 통합 대상은 위에 적은
  `14f5bd5`이며, 후속 readiness와 merge/push는 이 역사적 QA 기록의 범위 밖이다. 상세 결과는
  `2026-09-07-stock-beta-structural-ui-remediation-qa.md`에 저장했다.
- 실행 당시 Wave 1에서는 Next production build/browser를 WP-3만 실행했고, 다른 worker는 scoped
  unit/static 검증을 수행하여 build 프로세스 충돌을 방지했다.
- WP-3 첫 실행은 browser harness import/server-lifetime 오류가 반복되어 중단했다. source edit는
  없었고, 재실행은 gpt-5.6-terra medium 담당의 격리 workspace에서 수행했다. baseline은 기존
  확정 실측을 재사용했고 worker는 수정 및 static/unit/E2E assertion 작성에 집중했다. production
  dynamic QA는 WP-6에서 단일 담당이 실행했다.
- baseline QA는 동일 HEAD에서 완료된 typecheck/lint, Vitest 267개, Stock Beta E2E 26개 통과와
  viewport 실측을 사용했다. full E2E 71개 실행 결과도 당시 보고에 포함됐지만, 이 문서의
  독립 증거만으로 확정 통과로 재판정하지 않는다. 변경 전 동일 검증을 불필요하게 반복하지 않았다.
- 금지 API traffic 0 조건은 역사적 Stock Beta focused suite에 적용했다. Full Web suite의 기존
  localhost synthetic account/live fixture 시나리오는 mock UI 회귀 검사이며 실제 backend,
  provider 또는 계좌/주문 서버에 연결하지 않는다.

## Goal and boundaries

### 목표

`feature/stock-beta-eod-chart-20260904`의 Stock Beta Koyfin형 UI를 기능 변화 없이 정리하여,
Widget·Profile tab·차트 기간을 안전하게 추가·삭제·재배치할 수 있고 필수 viewport에서 모든
조작 요소가 처음부터 보이거나 정상적으로 접근 가능한 구조로 만든다.

완료 시 다음 조건을 모두 만족해야 한다.

1. dashboard와 detail Widget의 ID, component, required 정책, desktop/tablet/mobile placement가
   하나의 widget-centric catalog entry에서 파생된다. 별도 ID union, required-ID 목록, layout
   목록을 사람이 서로 맞춰 쓰지 않는다.
2. 새 optional Widget은 component와 catalog entry, 해당 Widget 테스트만 추가하면 된다. 제거는
   catalog entry와 component/test 제거로 끝나며 dangling layout·required ID가 생기지 않는다.
3. Widget catalog의 배열 순서가 DOM과 보조기술의 canonical reading order라는 정책을 갖는다.
   breakpoint placement는 시각 배치만 표현하고, 사용되지 않는 definition `order`,
   `defaultSize`, `defaultVisible` 같은 metadata를 남기지 않는다. 호환 목적상 유지해야 한다면
   실제 runtime 소비처와 테스트를 명시한다.
4. Profile tab registry에서 기본 탭을 제거하거나 순서를 바꿔도 렌더된 tabpanel의
   `aria-labelledby`가 실제 선택된 tab을 정확히 참조한다. tab ID는 registry에서 파생되고,
   중복·빈 registry·잘못된 default를 fail closed 또는 명시적 fallback으로 처리한다.
5. 차트 기간 버튼과 번역 타입은 기존
   `OWNER_EQUITY_V2_CHART_RANGE_VALUES`/`OwnerEquityV2ChartRange` 계약에서 파생한다. API 계약을
   UI에서 별도 literal 목록으로 복제하지 않는다.
6. 375×800, 640×360, 768×1024에서 현재 primary navigation destination이 최초 진입 직후
   viewport 안에 보인다. 이를 위해 focus를 강제로 이동하거나 페이지 전체를 스크롤하지 않는다.
7. 1280×720에서 기존 `rank / chart / decomposition` 데스크톱 정보 구조를 유지하면서 membership
   action 전체와 focus ring이 Widget 및 viewport 안에 보인다. 수동 horizontal scroll을 조작의
   전제로 삼지 않는다.
8. 기존 Owner-only, provider-free, snapshot-pinned EOD chart, fail-closed state, locale,
   pointer/touch/keyboard 동작과 전체 authenticated route 회귀가 유지된다.
9. TypeScript, Biome, 전체 Vitest, production Webpack build, focused Stock Beta Playwright 2회,
   전체 Web Playwright 1회와 독립 구조 리뷰가 모두 통과한다.

### 대상 기준과 workspace (역사적 실행 기준)

- 구현 기준 checkout: [repository root](../../../)의
  `feature/stock-beta-eod-chart-20260904` checkout (historical absolute worktree path omitted).
- 기준 branch: `feature/stock-beta-eod-chart-20260904`
- 계획 작성 시 기준 HEAD: `e4585864c4db60a7458e0167d75f469ab152b529`
- 비교 기준: 해당 HEAD의 `origin/main...HEAD` Stock Beta 변경 전체.
- 역사적 실행에서 coordinator는 시작 직전 branch, HEAD, clean state를 다시 확인했다. HEAD가
  바뀐 경우에는 변경 내용을 먼저 재검토하고 이 계획의 경로·테스트 가정을 갱신하는 규칙을
  적용했다.
- 각 worker package는 `$paseo-delegate`가 제공하는 격리 workspace에서 작업한다. worker 결과를
  기준 branch에 통합하는 판단은 coordinator가 담당한다.
- 이 문서는 [planning directory](./)에 보관하며, 역사적 실행은 위 구현 기준 checkout/commit을
  source baseline으로 사용했다.

### 확정 설계 방향

#### Widget catalog

- dashboard와 detail은 공통 helper가 검증하는 widget-centric catalog를 사용한다.
- 각 catalog entry는 최소한 `id`, `component`, `required`, 세 breakpoint의 populated/empty
  placement를 함께 소유한다.
- catalog로부터 runtime definitions, serializable configuration, required IDs 및 breakpoint layout을
  파생한다. 기존 외부 소비 shape가 필요하면 derived compatibility view로 제공한다.
- Widget ID type은 catalog literal에서 추론한다. `types.ts`의 수동 ID 배열과 registry의 같은 ID
  반복을 동시에 유지하지 않는다.
- catalog 배열 순서를 canonical DOM/accessibility order로 문서화한다. 각 breakpoint의 grid
  coordinates는 Koyfin형 시각 배치를 표현할 수 있지만 CSS `order`와 별도 definition order를
  중복 source-of-truth로 사용하지 않는다.
- validator의 duplicate ID, 잘못된 grid, overlap, required visibility, serializability 검증은
  유지하거나 더 강하게 만든다.

#### Profile tab과 chart range

- Profile tab definition에는 stable ID, label resolver, renderer와 명시적 default 정책을 둔다.
- active state가 registry에 없으면 실제 fallback tab ID로 normalize한 뒤 `aria-selected`,
  `aria-labelledby`, renderer를 모두 같은 값에서 계산한다.
- registry add/remove/reorder 테스트는 렌더 결과뿐 아니라 `tab`↔`tabpanel` ARIA 참조 존재까지
  검증한다.
- range button 반복은 `OWNER_EQUITY_V2_CHART_RANGE_VALUES`를 직접 사용한다. dictionary signature도
  `OwnerEquityV2ChartRange`를 사용한다. 신규 range를 이번 작업에서 추가하지 않는다.

#### Responsive interaction

- mobile/tablet primary navigation은 route mount 및 pathname 변경 후 active item을 scroll container
  안으로 가져오되 focus를 훔치지 않고 페이지 수직 위치를 바꾸지 않는다. reduced-motion에서도
  움직임 의존성을 만들지 않는다.
- membership widget은 container width 또는 명시적인 중간 폭 규칙을 기준으로 row/action을
  재배치한다. 1280px에서 dashboard 전체를 tablet layout으로 바꿔 핵심 3열 분석 구조를 없애는
  방식은 허용하지 않는다.
- nested horizontal overflow를 단순히 page-level overflow 검사로 숨기지 않는다. action bounds와
  focus visibility를 실제 Chromium에서 측정한다.

### In scope

- Stock Beta dashboard/detail widget architecture helper, registries, layouts, renderer와 README.
- Profile tab registry, signal preview widget, chart range UI/dictionary type 연결.
- shared research terminal primary navigation의 active-item visibility 동작.
- membership widget의 1280px layout/focus visibility.
- 두 개의 사용되지 않는 Stock Beta route helper와 production import가 없는 theme module 정리.
- 관련 Vitest/Playwright 회귀, production Web build, read-only architecture review.

### Out of scope

- Rust API, database, artifact, OpenAPI endpoint/schema 또는 EOD 계산 변경.
- 새 chart range, 새 Widget, drag-and-drop, 사용자별 layout persistence.
- 다른 제품의 정보 구조·색상·타이포그래피 재설계. shared navigation 수정에 따른 회귀 확인만 한다.
- KIS/OpenDART/KIND/KRX/data.go.kr 및 다른 외부 provider 호출.
- account, balance, order, live trading, WebSocket 또는 credential 처리.
- dependency 설치·업그레이드, package-lock 변경.
- `main` merge, origin push, production build/image, 배포, 서비스 재시작, Tailscale/Funnel 변경.

### 적용 instruction sources (역사적 실행에 사용)

- repository `AGENTS.md`: provider/account/order 안전 경계, production resource 정책, diagram evidence
  정책을 준수한다.
- `apps/web/AGENTS.md`: 설치된 Next.js 16.3 문서를 코드 수정 전에 읽는다.
- `apps/web/CLAUDE.md`: `apps/web/AGENTS.md`를 재참조한다.
- `$paseo-delegate-plan`: 모든 `WP-*`는 실행 시 `$paseo-delegate`만 사용한다. native subagent,
  Task/Agent/team/collaboration delegation으로 대체하지 않는다.
- Web implementation worker 필수 Next 문서:
  - `node_modules/next/dist/docs/01-app/01-getting-started/05-server-and-client-components.md`
  - client navigation/route 동작을 수정하는 worker는 설치본의 `usePathname` 관련 API 문서
  - test worker는
    `node_modules/next/dist/docs/01-app/02-guides/testing/vitest.md`와
    `node_modules/next/dist/docs/01-app/02-guides/testing/playwright.md`
- dependency가 현재 workspace에 없으면 임의 `npm ci`/install을 하지 않는다. coordinator에게
  blocker를 보고하고 승인된 existing dependency 경로 또는 별도 설치 지시를 기다린다.

### 역사적 실행 전제와 미해결 요구사항

- 사용자 결정이 필요한 제품 요구사항은 없다. active nav는 현재 route를 자동 노출하고,
  membership action은 1280px에서 wrap/reflow하는 방향으로 확정한다.
- 계획 작성 시점에는 `$paseo-delegate-plan`은 있었으나 실행용 `$paseo-delegate` 노출 여부를
  별도 prerequisite로 확인해야 했다. 역사적 실행은 해당 확인 후에만 worker를 launch했고 native
  subagent로 대체하지 않았다.
- 내부 frontend dependency만 바꾸고 component diagram의 `apps/web` 외부 edge와 runtime diagram은
  바뀌지 않을 것으로 예상했다. coordinator가 최종 diff를 확인했고 drawn edge/evidence 변경은
  없었다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | shared architecture helper와 dashboard/detail registry를 함께 바꾸며 type inference, runtime validation, DOM order, serializable projection을 동시에 보존해야 한다. | high | detail/dashboard type cycle, compatibility projection 파손, 구조 테스트가 같은 원인으로 2회 실패하면 design을 축소해 재계획하거나 gpt-5.6-sol xhigh로 상향한다. |
| WP-2 | intermediate | Profile tab fallback과 range source를 정해진 계약에 연결하는 여러 파일의 bounded refactor이며 deterministic ARIA/unit test가 가능하다. | high | registry default 정책이 기존 dynamic mutation test와 충돌하거나 ARIA 오류가 2회 반복되면 gpt-5.6-terra medium으로 상향한다. |
| WP-3 | intermediate | 재현 viewport와 acceptance가 명확하지만 shared navigation scroll, responsive CSS, 실제 focus visibility를 함께 다룬다. | high | active item 노출이 focus/수직 scroll을 변경하거나 1280px 3열 구조가 깨지거나 Playwright가 2회 불안정하면 gpt-5.6-terra medium으로 상향한다. |
| WP-4 | simple | 호출처가 없는 두 helper와 production import가 없는 CSS module을 제거하고 정적 테스트를 갱신하는 기계적 cleanup이다. | high | 숨은 production import 또는 dynamic CSS 소비가 발견되면 삭제를 멈추고 intermediate로 재분류한다. |
| WP-5 | hard | 통합 diff가 구조적 요구를 충족하는지 독립 판단하는 것이 산출물이고 DOM/accessibility/config drift를 전체적으로 추적해야 한다. | high | catalog가 여전히 여러 수동 source를 요구하거나 test evidence가 부족하면 REJECT하고 owning WP로 돌려보낸다. |
| WP-6 | intermediate | 명령과 viewport가 확정된 최종 QA이지만 production build, synthetic API, 브라우저 상호작용, 시각 판정을 함께 통합해야 한다. | high | 환경/fixture와 code defect 구분이 불명확하거나 동일 browser test가 2회 실패하면 gpt-5.6-terra high로 상향한다. |

## Execution graph (historical execution)

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | dashboard/detail Widget 설정을 widget-centric catalog로 통합하고 DOM order 정책을 명시 | shared widget architecture, dashboard/detail registry/layout/renderers, architecture tests, README | 없음 | `$paseo-delegate`: gpt-5.6-sol, high | derived ID/required/layout/config 구조와 migration | focused architecture/dashboard Vitest, typecheck, Biome, invariant scans |
| WP-2 | 1 | intermediate | Profile tab fallback/ARIA를 수정하고 chart range UI를 canonical contract에 연결 | profile registry/widget, chart range dictionary/contract imports, chart surface tests | 없음 | `$paseo-delegate`: gpt-5.6-luna, max; 반복 실패 시 terra medium | 안전한 registry mutation과 단일 range source | focused chart Vitest, typecheck, Biome |
| WP-3 | 1 | intermediate | 작은 화면 active nav와 1280px membership action/focus 결함 수정 | primary navigation/shell CSS, dashboard CSS, navigation tests, Stock Beta E2E spec | 없음 | `$paseo-delegate`: gpt-5.6-luna, max; 반복 실패 시 terra medium | focus를 훔치지 않는 active-nav exposure와 unclipped actions | focused unit, production Chromium viewport assertions, typecheck, Biome |
| WP-4 | 1 | simple | dead Stock Beta helper/theme surface 제거 | 두 Stock Beta route page, theme CSS, shell runtime test | 없음 | `$paseo-delegate`: gpt-5.6-luna, low | dead code 없는 route/theme surface | `rg`, focused Vitest, lint/Biome, typecheck |
| WP-5 | 2 | hard | 통합 결과의 구조·접근성·확장성 독립 read-only review | 전체 integrated diff와 test evidence, 수정 금지 | WP-1~WP-4 | `$paseo-delegate`: gpt-5.6-terra, high | severity별 findings와 ACCEPT/REJECT | source trace, config mutation evidence, diff checks |
| WP-6 | 3 | intermediate | 전체 frontend 정적·동적·시각 최종 QA | repository read-only; 임시 build/test artifacts만 허용 | WP-5 ACCEPT | `$paseo-delegate`: gpt-5.6-terra, medium | 명령별 verification matrix와 최종 ACCEPT/REJECT | full Vitest, lint/typecheck/build, focused E2E 2회, full E2E 1회, 5 viewport audit |

Wave 1의 네 package는 mutable scope가 겹치지 않는다. `dashboard.module.css`와
`tests/e2e/stock-beta.spec.ts`는 WP-3만 소유하고, Widget architecture 테스트는 WP-1,
Profile tab 테스트는 WP-2, `shell-runtime.test.ts`는 WP-4만 소유한다. worker가 다른 package의
owned file 수정이 필요하다고 판단하면 직접 수정하지 않고 coordinator에게 dependency conflict를
보고한다.

WP-5가 REJECT하면 Wave 3을 시작하지 않는다. finding을 원래 owning WP에 제한된 follow-up으로
반송하고, 통합 후 WP-5 review를 다시 통과시킨다.

모든 package는 반드시 `$paseo-delegate`로 실행한다. native subagent, Task/Agent/team 또는
collaboration 도구로 worker package를 실행하지 않는다.

## Worker briefs (historical requirements)

### WP-1 — Widget catalog와 DOM order 구조 통합

- **Target working directory:** `$paseo-delegate`가 기준 commit `e4585864...`에서 만든 격리 Web
  workspace.
- **Initial complexity:** hard, confidence high. 두 화면과 shared validator의 public type shape를
  안전하게 재설계해야 한다.
- **Escalation signals:** type-only import cycle, dashboard/detail 중 한쪽의 serializable config 파손,
  같은 architecture test 원인의 두 번째 실패.
- **Objective:** Widget 추가·삭제 시 ID/required/layout 목록을 따로 동기화하지 않는 catalog API를
  만들고 dashboard와 detail을 그 API로 이전한다.
- **Known facts:**
  - dashboard ID는 `dashboard/types.ts`, definition은 `dashboard/widget-registry.ts`, layout은
    `dashboard/dashboard-layout.ts`, required policy는 다시 registry에 있다.
  - detail도 definition/required/layout을 각각 유지한다.
  - renderer는 definitions 배열로 DOM을 만들지만 definition `order`는 DOM sort에 쓰지 않는다.
  - validator는 duplicate, overlap, missing required, invalid grid를 이미 강하게 검증한다.
- **Owned scope:**
  - `apps/web/components/stock-beta/shared/widget-types.ts`
  - `apps/web/components/stock-beta/dashboard/types.ts`
  - `apps/web/components/stock-beta/dashboard/widget-registry.ts`
  - `apps/web/components/stock-beta/dashboard/dashboard-layout.ts` — catalog에 흡수되면 삭제 가능
  - `apps/web/components/stock-beta/dashboard/stock-beta-dashboard.tsx`
  - `apps/web/components/stock-beta/detail/widget-registry.ts`
  - `apps/web/components/stock-beta/detail/types.ts` — dashboard와 동일한 수동 ID union 제거 및
    catalog-derived type 이전에 필요한 범위만. 다른 package와 소유 범위가 겹치지 않는다.
  - `apps/web/components/stock-beta/detail/detail-layout.ts` — catalog에 흡수되면 삭제 가능
  - `apps/web/components/stock-beta/detail/stock-beta-detail-layout.tsx`
  - `apps/web/components/stock-beta/README.md`
  - `apps/web/tests/stock-beta-widget-architecture.test.tsx`
  - `apps/web/tests/stock-beta-dashboard.test.tsx`
- **Prohibited adjacent work:** Widget copy/visual styling, chart/profile logic, route pages, API contracts,
  backend, E2E spec, diagrams.
- **Inputs/dependencies:** repository/app instructions, installed Next server/client component guide, current
  architecture tests. 다른 WP dependency 없음.
- **Required implementation constraints:**
  1. Existing accepted layouts and empty-state visibility remain semantically identical.
  2. `requiredWidgetIds` is derived from `required` entries, not manually repeated at call sites.
  3. Widget ID union is inferred from catalog literals; no second manual ID list remains.
  4. populated/empty placement completeness is validated for every required Widget and breakpoint.
  5. serializable projection contains no component functions and remains deterministic.
  6. catalog entry order is the documented DOM/accessibility order. Dead definition metadata is removed or
     given a tested runtime consumer.
  7. Test-only add/remove/reorder uses the same public catalog API a future Widget author would use and
     verifies rendered DOM order as well as visual placement variables.
- **Expected output:** migrated shared catalog API, dashboard/detail catalogs, updated renderer/tests/README,
  one focused commit.
- **Verification:**
  - existing dependency path에서 `vitest run --config vitest.config.ts --configLoader runner`로
    `stock-beta-widget-architecture.test.tsx`와 `stock-beta-dashboard.test.tsx` 실행.
  - `npm --prefix apps/web run typecheck`.
  - owned-file Biome check와 `git diff --check`.
  - `rg`로 수동 dashboard Widget ID 배열, call-site `requiredWidgetIds`, dead definition order/default
    metadata가 남지 않았는지 확인.
- **Required report:** 변경 파일과 라인 범위; brief와 다르게 처리한 부분과 이유; 실행한 검사와
  결과; 미해결/후속 작업; 찾지 못했거나 확인하지 못한 것. 빈 항목은 `none`이라고 명시한다.

### WP-2 — Profile tab ARIA와 chart range source 통합

- **Target working directory:** 기준 commit에서 만든 격리 Web workspace.
- **Initial complexity:** intermediate, confidence high. 실패 재현과 desired contract가 명확하다.
- **Escalation signals:** default normalization이 React state loop를 만들거나 dynamic registry mutation
  테스트가 두 번 실패하면 terra medium으로 재실행한다.
- **Objective:** tab add/remove/reorder 후에도 실제 active tab과 panel ARIA 관계를 보존하고 range
  controls를 canonical contract values에서 만든다.
- **Known facts:**
  - state는 항상 `price`로 시작하고 missing tab renderer는 첫 registry entry로 fallback하지만
    `aria-labelledby`는 이전 state ID를 사용한다.
  - chart range values는 contract에 이미 export돼 있으나 profile UI와 dictionary가 literal을
    반복한다.
- **Owned scope:**
  - `apps/web/components/stock-beta/dashboard/profile-tab-registry.ts`
  - `apps/web/components/stock-beta/dashboard/widgets/signal-preview-widget.tsx`
  - `apps/web/lib/products/equity-signals-contracts.ts` — 기존 export 재사용에 필요한 type/export 조정만
  - `apps/web/lib/i18n/dictionaries/stock-beta.ts`
  - `apps/web/tests/stock-beta-chart-surface.test.tsx`
- **Prohibited adjacent work:** dashboard widget catalog/layout/CSS, shell navigation, API/OpenAPI/Rust range
  계약, 신규 range, E2E fixture/spec.
- **Inputs/dependencies:** existing `OWNER_EQUITY_V2_CHART_RANGE_VALUES`, `OwnerEquityV2ChartRange`, current
  dynamic registry tests.
- **Required implementation constraints:**
  1. renderer, `aria-selected`, `aria-labelledby`는 normalized active ID 하나에서 계산한다.
  2. 기본 tab 삭제·첫 tab 변경·reorder·empty/duplicate invalid registry 행동을 명시하고 검증한다.
  3. tab panel의 `aria-labelledby` target이 현재 DOM에 정확히 하나 존재해야 한다.
  4. range button 순서는 exported canonical values와 동일하고 dictionary mapping은 exhaustive다.
  5. EOD chart data semantics, loading range honesty, request callback behavior를 바꾸지 않는다.
- **Expected output:** safe Profile tab registry contract, canonical range-backed controls, regression tests,
  one focused commit.
- **Verification:** focused `stock-beta-chart-surface.test.tsx` Vitest, typecheck, owned-file Biome,
  `git diff --check`, literal range duplication `rg` audit.
- **Required report:** 변경 파일과 라인 범위; deviations/reasons; checks/results; unresolved/follow-up;
  not found/not verified. 빈 항목은 `none`.

### WP-3 — Responsive navigation과 membership action 접근성

- **Target working directory:** 기준 commit에서 만든 격리 Web workspace.
- **Initial complexity:** intermediate, confidence high. viewport별 재현값이 확정돼 있다.
- **Escalation signals:** active-nav 처리로 focus가 이동하거나 page vertical scroll이 변함, 1280px 핵심
  3열 geometry가 사라짐, 동일 Playwright assertion이 두 번 flake.
- **Objective:** active primary route를 작은 화면에서 최초 노출하고 1280px membership actions와 focus
  ring을 잘리지 않게 한다.
- **Known facts:**
  - 375/640/768px에서 nav `clientWidth`보다 `scrollWidth`가 크고 active Stock Beta link가 초기에는
    밖에 있다. direct focus 후에는 container가 scroll된다.
  - 1280px 첫 Disable button은 left 1259/right 1311이며 viewport 밖이다. membership list는
    clientWidth 407/scrollWidth 459이고 direct focus 후에도 `scrollLeft=0`이다.
  - 1440px에서는 같은 action이 정상 노출된다.
- **Owned scope:**
  - `apps/web/components/shell/primary-navigation.tsx`
  - `apps/web/components/shell/research-terminal-shell.module.css`
  - `apps/web/components/stock-beta/dashboard/dashboard.module.css`
  - `apps/web/tests/role-navigation.test.tsx`
  - 필요 시 신규 navigation-focused Vitest 파일 하나
  - `apps/web/tests/e2e/stock-beta.spec.ts`
- **Prohibited adjacent work:** nav item order/permissions, 다른 제품 layout redesign, widget registry,
  Profile tab/range, API/fixture/backend, global theme cleanup.
- **Inputs/dependencies:** required five viewports, current Koyfin 3-column desktop contract, installed Next
  `usePathname`/client component docs, existing provider-free E2E guard.
- **Required implementation constraints:**
  1. active nav becomes visible on initial load and client-side route changes without focus theft.
  2. nav reveal changes only the nav container's horizontal position; body vertical scroll remains unchanged.
  3. 1280px desktop keeps ranked/profile/decomposition geometry while membership action buttons and focus
     outlines stay inside Widget/viewport.
  4. no page horizontal overflow at all five viewports; no hidden-control workaround.
  5. keyboard Tab and pointer/touch paths remain usable; reduced-motion/forced-colors behavior remains.
  6. E2E explicitly measures active link/action bounding boxes and keyboard focus visibility so the previous
     page-level overflow-only test cannot pass the defect.
- **Expected output:** responsive shell/dashboard CSS and navigation behavior with unit/E2E regression,
  one focused commit.
- **Verification:** focused navigation unit test, typecheck, owned-file Biome, `git diff --check`, production
  Webpack build plus one provider-free focused Chromium pass when the worker environment supports it. If the
  runtime pass is unavailable, report it explicitly; WP-6 remains mandatory.
- **Required report:** changed files/lines; deviations/reasons; checks/results with viewport measurements;
  unresolved/follow-up; not found/not verified. 빈 항목은 `none`.

### WP-4 — Dead Stock Beta route/theme cleanup

- **Target working directory:** 기준 commit에서 만든 격리 Web workspace.
- **Initial complexity:** simple, confidence high. production consumers가 없는 것이 현재 `rg`로 확인됐다.
- **Escalation signals:** production import/dynamic theme reference 발견 시 삭제하지 말고 intermediate
  재분류 요청.
- **Objective:** lint 경고를 만드는 두 unused helper와 production import가 없는 legacy theme module을
  제거하고 테스트를 실제 shared shell source에 맞춘다.
- **Known facts:**
  - `StockBetaProductPage`와 `StockBetaDetailProductPage`는 같은 파일에도 호출처가 없다.
  - `stock-beta-theme.module.css`는 production import가 없고 `shell-runtime.test.ts`만 source text를
    읽는다. 실제 tokens는 `research-terminal-shell.module.css`에 있다.
- **Owned scope:**
  - `apps/web/app/(authenticated)/stock-beta/page.tsx`
  - `apps/web/app/(authenticated)/stock-beta/[instrument]/page.tsx`
  - `apps/web/components/stock-beta/stock-beta-theme.module.css` — 확인 후 삭제
  - `apps/web/tests/shell-runtime.test.ts`
- **Prohibited adjacent work:** remaining cookie/`!important` warnings, route behavior, shell styles,
  dashboard/profile/chart, backend.
- **Inputs/dependencies:** production import/call-site `rg`, current shared terminal shell test.
- **Expected output:** dead helpers/module 제거, shared shell contract를 직접 검사하는 test, one focused
  commit.
- **Verification:** exact-symbol/import `rg`, focused `shell-runtime.test.ts` Vitest, typecheck, full lint or
  owned-file Biome, `git diff --check`.
- **Required report:** changed files/lines; deviations/reasons; checks/results; unresolved/follow-up; not
  found/not verified. 빈 항목은 `none`.

### WP-5 — 독립 구조 acceptance review

- **Target working directory:** WP-1~WP-4가 통합된 coordinator workspace.
- **Initial complexity:** hard, confidence high. 결론 자체가 산출물이다.
- **Escalation signals:** architecture API가 불명확하거나 테스트가 실제 extension path를 사용하지 않으면
  REJECT하며 구현을 추측해 고치지 않는다.
- **Objective:** 최초 review의 모든 finding이 구조적으로 해결됐고 새 drift/접근성 결함이 없는지
  read-only로 판정한다.
- **Owned scope:** 전체 integrated `baseline...HEAD` diff와 관련 source/tests/docs. 파일 수정 금지.
- **Prohibited adjacent work:** 모든 source/test/doc edit, external/provider/account/order/live 호출.
- **Inputs/dependencies:** 최초 finding 목록, WP-1~WP-4 reports/commits, clean integration state.
- **Required review:**
  - catalog entry 한 곳에서 ID/required/3-breakpoint layout이 파생되는지.
  - optional Widget add/remove/reorder test가 production public API와 renderer를 실제 사용하며 DOM order
    및 serializable projection을 검증하는지.
  - Profile tab 제거 시 모든 ARIA IDREF가 resolve되는지.
  - range UI/dictionary가 exported contract type/value에 연결됐는지.
  - active nav와 1280px action fix가 hidden overflow나 focus theft로 우회되지 않았는지.
  - dead files/functions가 실제로 사라졌는지.
  - internal change가 architecture diagram의 drawn edge/evidence를 바꾸는지.
- **Expected output:** Critical/High/Medium/Low findings, former finding별 FIXED/NOT FIXED, 최종
  `ACCEPT` 또는 `REJECT`.
- **Verification:** `git status --short`, `git diff --check`, full diff/source trace, test evidence 대조.
- **Required report:** inspected files/ranges; findings; good structure; extension matrix; deviations;
  checks/results; unresolved/follow-up; not found/not verified; 변경 파일은 `none`.

### WP-6 — 최종 frontend QA

- **Target working directory:** WP-5 ACCEPT 상태의 clean integration workspace.
- **Initial complexity:** intermediate, confidence high. 명령과 acceptance viewport가 고정돼 있다.
- **Escalation signals:** code/environment 분류 불가, browser flake 2회, screenshot과 DOM metric 불일치.
- **Objective:** production build를 실제 Chromium으로 조작하여 정적·동적·시각 회귀를 최종 판정한다.
- **Owned scope:** source read-only. `.next`, test-results, screenshot, temporary logs/processes/ports만 생성
  후 정리 가능. source defect를 직접 수정하지 않는다.
- **Prohibited adjacent work:** source/test/doc commit, dependency install, external/provider/account/order/live
  요청, production/Tailscale/Funnel 조작.
- **Inputs/dependencies:** WP-5 ACCEPT, existing synthetic API and provider-free guards, five required
  viewports.
- **Required verification:**
  1. `npm --prefix apps/web run typecheck`.
  2. `npm --prefix apps/web run lint`; Stock Beta dead-code warning 0, 새 error/warning 0. 기존 unrelated
     warning은 baseline과 비교해 별도 기록한다.
  3. `./node_modules/.bin/vitest run --config vitest.config.ts --configLoader runner`; 전체 파일/테스트
     수와 elapsed를 기록한다.
  4. synthetic API 전용 `API_INTERNAL_URL`로 `next build --webpack` production build.
  5. fresh synthetic/standalone process와 고유 localhost port를 사용해 focused
     `tests/e2e/stock-beta.spec.ts`를 한 worker로 2회 연속 실행한다. 두 번째 pass는 clean `.next`
     rebuild를 사용한다.
  6. 전체 app-local Playwright 1회, 한 worker. unrelated failure도 숨기지 말고 Stock Beta 회귀와
     분리한다.
  7. 375×800, 640×360, 768×1024, 1280×720, 1440×900 screenshot과 DOM metric:
     - document horizontal overflow 0.
     - active primary link가 initial viewport/nav bounds 안에 있음.
     - initial active-nav reveal이 focus와 body vertical scroll을 변경하지 않음.
     - 1280px membership actions와 focus ring이 widget/viewport bounds 안에 있음.
     - rank/chart/decomposition desktop geometry 유지.
     - current chart SVG와 candle count가 실제로 표시됨.
     - every `aria-labelledby`/`aria-controls` IDREF target이 정확히 하나 존재함.
  8. external, KIS, OpenDART, V1 equity-price-signals, account/order/live API traffic 0.
  9. exact QA PID/port readiness 확인 및 종료, temporary symlink/build/test/log 제거, final clean status.
- **Expected output:** command/result/count/duration matrix, viewport measurements/screenshots 판독,
  code defect와 environment limitation 분류, 최종 `ACCEPT`/`REJECT`.
- **Required report:** changed files/lines=`none`; deviations/reasons; all checks/results; unresolved/follow-up;
  not found/not verified. 빈 항목은 `none`.

## Coordinator gates (historical acceptance checklist)

다음 gate는 완료된 실행의 요구사항과 감사용 체크리스트를 보존한 것이다. 현재 pending 작업을
뜻하지 않는다.

### Gate 0 — Pre-launch

1. `$paseo-delegate` execution skill이 현재 context에 실제로 제공되는지 확인한다. 없으면 즉시 중단하고
   prerequisite blocker를 사용자에게 보고한다.
2. implementation workspace가 `feature/stock-beta-eod-chart-20260904`의 예상 HEAD에 있고 clean인지,
   `origin/main...HEAD`에 사용자의 미통합 변경이 없는지 확인한다.
3. `AGENTS.md`, `apps/web/AGENTS.md`, `apps/web/CLAUDE.md`와 worker별 Next docs를 읽었는지 확인한다.
4. baseline typecheck, lint, focused/full Vitest와 현재 두 viewport defect 재현값을 기록한다.
5. Wave 1 ownership이 겹치지 않는지 다시 확인한다. 겹치면 병렬 실행하지 않고 package dependency를
   수정한다.

### Gate 1 — Wave 1 integration

1. WP-1~WP-4 결과를 report 형식, commit scope, clean worktree, validation evidence 기준으로 각각
   ACCEPT/REJECT한다.
2. 각 commit을 integration workspace에 한 번에 하나씩 적용하고 매번 `git diff --check`, scoped
   typecheck/Vitest를 실행한다. conflict를 자동 해결하지 않는다.
3. 통합 후 다음 invariants를 직접 확인한다.
   - manual dashboard Widget ID/required/layout source duplication 없음.
   - dead definition metadata 없음 또는 실제 runtime 소비/test 존재.
   - Profile tab panel IDREF가 fallback 후에도 resolve됨.
   - range controls가 exported contract values에서 생성됨.
   - active nav가 focus를 훔치지 않음.
   - 1280px action clipping을 숨은 horizontal scroll로 우회하지 않음.
4. `git diff --name-only`로 Rust/API/OpenAPI/deploy/diagram 등 out-of-scope 수정이 없는지 확인한다.

### Gate 2 — Independent review

1. WP-5는 read-only로 실행한다.
2. Critical/High/Medium finding 또는 former finding `NOT FIXED`가 하나라도 있으면 WP-6을 시작하지
   않는다.
3. finding은 해당 owned scope의 원 WP에 제한된 follow-up으로 반환한다. 두 번 같은 실패가 반복되면
   classification 표의 escalation rule에 따라 worker model/effort를 한 단계 올린다.
4. 수정 통합 후 WP-5를 다시 실행하여 명시적 ACCEPT를 받아야 한다.

### Gate 3 — Final acceptance

1. WP-6의 typecheck, lint, full Vitest, production build가 모두 terminal exit 0인지 확인한다.
2. focused Stock Beta Playwright가 fresh build로 2회 연속 green이고 full Web Playwright 1회가 terminal
   summary를 남겼는지 확인한다.
3. 다섯 viewport에서 active nav, membership action/focus, no-overflow, chart visibility, ARIA IDREF
   metric을 screenshot과 함께 확인한다.
4. provider/account/order/live/V1 request guard가 0인지 확인한다.
5. internal frontend refactor가 component/runtime diagram의 drawn edge나 evidence line을 바꾸지 않았음을
   확인한다. 바뀌었으면 계획을 개정해 `.puml`/PNG update와 local PlantUML render를 별도 package로
   완료하기 전 ACCEPT하지 않는다.
6. temporary process, ports, symlink, `.next`, test-results, logs를 정리하고 final `git status --short`와
   `git diff --check`가 clean인지 확인한다.
7. 역사적 최종 ACCEPT 이후 merge, push, deploy는 별도 범위로 남겼다. 현재 통합 대상
   `14f5bd5`에 대한 readiness와 merge/push는 coordinator의 후속 작업으로 처리한다.
