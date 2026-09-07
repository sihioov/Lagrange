# Stock Beta 구조 개선 실행·QA 결과

작성일: 2026-09-07
문서 성격: 역사적 실행 결과 및 QA 증거 요약

## 반영 상태

- 구현 checkout: [repository root](../../../)의 historical feature checkout (absolute worktree path
  intentionally omitted)
- 브랜치: `feature/stock-beta-eod-chart-20260904`
- 기준: `e4585864c4db60a7458e0167d75f469ab152b529`
- 검증 HEAD: `6d9f9eb717e7068bd10e7cde9b182b5cea6ee72b`
- 현재 통합 대상: `14f5bd5` (검증 HEAD를 포함; coordinator가 제공한 기준)
- 역사적 QA 종료 시점에는 main 머지, push, 배포, Tailscale/Funnel 변경이 없었다. 현재 통합과
  readiness는 이 QA 기록의 범위 밖이다.

## 구현

1. dashboard/detail의 Widget ID, required 정책, breakpoint 배치를 widget-centric catalog에서 파생.
2. catalog 배열을 DOM/접근성 읽기 순서로 사용. 중복 metadata와 별도 수동 layout 목록 제거.
3. 잘못된 Widget 설정과 알 수 없는 derived layout key를 직렬화 전에 거부.
4. Profile tab 추가·제거·재정렬, stale selection, 빈/중복 ID 처리 및 ARIA 연결 보완.
5. 차트 기간 버튼·번역 타입을 기존 공통 계약에서 파생.
6. 현재 메뉴를 nav 내부에서만 노출. route/resize/locale label 변화 반영.
7. membership action 배치를 위젯 container 너비에 맞춰 재배치.
8. 사용하지 않는 두 route helper와 theme CSS 제거.

독립 구조 리뷰는 최초 두 finding을 원 담당에게 반환한 뒤 재검토하여 ACCEPT.
변경은 Web source/test/README 24개 파일에 한정한다. API, backend, deployment 또는 다이어그램
의존 edge 변경 없음.

## 검증 결과 (역사적 실행 기록)

아래 표의 명령·수치·elapsed는 당시 QA 실행 기록에 보고된 값이다. coordinator가 직접 확인한
화면·metric은 다음 단락에서 별도로 표시하며, 이 문서 정리 작업에서 검사를 다시 실행한 것은
아니다.

| 검사 | 실제 결과 |
| --- | --- |
| TypeScript | exit 0, 2초 |
| Lint | exit 0; 기존 경고 4개, deprecated config info 1개 |
| 전체 Vitest | 37 files / 271 tests, 1.62초 |
| 새 production Webpack build | 두 번 성공 |
| Stock Beta production Playwright | 29/29 두 번, 33.6초 / 33.3초 |
| 전체 Web Playwright | 74/74, 56.3초 |
| Standalone preflight | 두 서버 모두 화면과 실제 JS asset HTTP 200 |
| Stock Beta 금지 요청 guard | external/KIS/OpenDART/V1/account/order/live 0 |

실행 보고에 따르면 한 worker로 Chromium을 실제 조작했다. 종목 등록/상태 polling/새로고침/상세 이동, 기간 변경과
늦은 응답 경쟁, snapshot 고정, 검색, 탭, 키보드, locale, touch, forced colors/reduced motion을
기존·추가 E2E에서 검사했다. 전체 suite의 account/live 경로는 기존 localhost synthetic fixture만
사용했으며 실제 계좌·주문·provider에는 연결하지 않았다.

coordinator 직접 확인 요약: 375×800, 640×360, 768×1024, 1280×720, 1440×900에서 document horizontal overflow 0 및
ARIA IDREF가 각각 정확히 하나의 target을 참조함을 확인했다. 초기 메뉴 노출, client-link 이동,
언어 전환은 별도 직접 element-identity 측정으로 확인했다. 1280/1360/1361/1400px에서 Tab으로
이동한 Disable 버튼의 2px outline + 1px offset은 card/list/clipping ancestor/viewport 안에 있었다.

## 시각 QA에서 별도로 발견한 기존 한계

자동 테스트 통과가 데스크톱 그래프 전체의 가시성을 의미하지는 않는다. coordinator가 다섯
캡처를 직접 열어 검토하고 추가 실제 브라우저 측정을 요청했다.

- SVG는 정상 렌더되고 `.candleBody`는 정확히 261개다. 초기 audit의 523은 여러 SVG rect를
  합산한 값이므로 candle count 근거로 사용하지 않는다.
- 데스크톱 profile panel의 표시 높이는 1280px에서 128px, 1440px에서 약 175px다.
  해당 SVG 높이는 각각 약 225.7px, 262.4px다.
- 내부 스크롤로 차트 구간에 접근할 수 있지만 전체 그래프를 한 화면에 보기는 어렵다.
- 관련 `.profilePanel`/`.priceTab` CSS는 기준 `e458586`부터 존재하며 이번 diff에서 변경하지 않았다.
  이는 이번 구조 리팩터링의 신규 회귀가 아닌 기존 시각 UX 개선 항목이다. 차트 영역 높이/전체
  보기 설계는 이번 작업에서 임의로 변경하지 않았다.

따라서 구조 리뷰와 기능 회귀 검사는 통과했지만, 모든 시각 UX 문제를 해결했다고 판정하지 않는다.

## 증거 및 한계

- 실행 기록: `2026-09-07-stock-beta-structural-ui-remediation.md`
- 로그/원본 캡처/측정 JSON: historical local QA evidence (temporary location intentionally omitted)
- nav 직접 측정: `interaction-audit.json`. 초기 `audit-metrics.json.locale.activeIdentity`는
  의미 없는 자기 비교였으므로 폐기하고 이 후속 측정만 사용한다.
- 차트 후속 측정: `chart-visibility-followup.json`, `followup-screenshots/`.
- 이 QA는 합성 데이터 Chromium 검사다. 실제 시장 데이터 최신성, 외부 사용자 기기의 접속,
  실제 Tailscale/Funnel URL 동작이나 production 배포는 검증하지 않았다.
- 초기 sandbox loopback bind 실패와 임시 audit selector/측정 순서 오류는 통과 횟수에서 제외했다.

최종 프로세스/port/작업 트리 정리 여부는 실행 계획의 최종 기록을 참조한다.
