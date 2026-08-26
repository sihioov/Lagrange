Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Owner 사용자 기능 전수 수용성 테스트·분석 계획

## Goal and boundaries

- 목표: 현재 설치된 Owner-only beta에서 사용자가 실제로 볼 수 있는 기능을 사용자 여정으로
  검증하고, 단순 HTTP 성공이 아니라 입력→처리→지속성→의미 있는 결과→오류 복구까지 정상인지
  판정한다.
- 완료 기준:
  1. 로그인/세션/로그아웃, 대시보드, 전략 저장, 추천 생성·완료·결과, 후보, 스크리너,
     종목 상세, 백테스트, 운영 관리의 상태가 각각 `PASS`, `EXPECTED_BLOCK`, `FAIL`,
     `USER_ACCEPTANCE_PENDING` 중 하나로 증거와 함께 분류된다.
  2. Paper와 Live는 현재 계약대로 탐색에서 숨겨지고 직접 접근도 fail-closed하는지 확인한다.
  3. 알려진 `OWNER_BETA_PRICE_INPUT_UNAVAILABLE` 권한 결함을 재현 테스트로 고정하고,
     최소 권한 수정과 재발 방지 검사를 통과시킨다.
  4. 실제 브라우저와 같은 CSRF/쿠키/멱등키/API 순서로 전략→추천→보고서 핵심 여정이
     disposable PostgreSQL과 격리된 브라우저 fixture에서 성공한다.
  5. 독립 리뷰가 보안 경계, 결과 의미, 사용자 가시성, 오류 메시지와 테스트 누락을 검토해
     `ACCEPT` 또는 구체적인 remediation 목록을 낸다.
- 대상 workspace: `/data/worktrees/3puw275b/rural-mouse`, branch
  `audit-project-status`, 기준 `67fe14f8e90cc159c2d36606f1d67995a1732a16`; 통합 후 coordinator가
  `main`/`origin/main` 정렬 여부를 확인한다.
- 사용자 기능 범위:
  - 인증: Auth0 handoff 계약, session, CSRF, logout, 비인증 401/redirect.
  - Owner UI: `/`, `/strategies`, `/recommendations`, `/candidates`, `/screener`,
    `/stocks/[instrument]`, `/backtests`, `/admin`.
  - 기대 차단: `/paper`는 owner-beta Paper disabled, `/live`와 하위 API는 beta 범위 밖.
- 운영 범위:
  - 읽기 전용 health/revision/mount/permission/DB count 검사는 허용한다.
  - 이미 재현된 generic artifact-root의 그룹 통과 권한은 코드·fixture로 먼저 검증한 뒤,
    coordinator가 정확한 한 디렉터리의 group만 데이터 GID로 정렬하는 되돌릴 수 있는
    최소 운영 수정을 적용할 수 있다. 파일 내용·mode·owner·하위 산출물은 바꾸지 않는다.
  - 실제 Auth0 Owner 세션을 대리하거나 쿠키/비밀번호를 추출하지 않는다. 자동화할 수 없는
    최종 production 클릭은 `USER_ACCEPTANCE_PENDING`으로 남기되 동일 HTTP/DB/browser 흐름을
    fixture에서 완주한다.
- 제외:
  - 계좌, 잔고, 주문, 체결, 정정/취소, 주문 WebSocket, Compose `live` profile.
  - 새 KIS/OpenDART/FSC/KIND 실호출, 새 Raw 수집, 비밀·토큰·사용자 식별자 출력.
  - Member KR 공개, 엄격 PIT·총수익률·일반 READY 주장, Paper 활성화.
  - 기존 untracked `docs/kis_openapi_entiredocs_20260818_030007.xlsx` 접근·이동·stage.
- 적용 지침: repository `AGENTS.md`; `apps/web/AGENTS.md`와 `apps/web/CLAUDE.md`;
  `$paseo-delegate-plan`; `$paseo-delegate`. Web production code를 수정하는 remediation이
  생기면 현재 Next 설치본의 `node_modules/next/dist/docs/`에서 관련 문서를 먼저 읽는다.
- 자원 경계: 이 호스트는 35W/14-thread/약 16GiB다. Rust 컴파일·DB test worker는 한 번에
  하나만 실행하고 `CARGO_BUILD_JOBS=2`, `nice=12` 또는 동등한 제한을 사용한다. 전체
  workspace/build-all, 병렬 service image build, 무제한 Playwright worker는 금지한다.
- 현재 알려진 사실:
  - 전략 config 저장은 운영 DB에서 `1 total / 1 active`로 확인됐다.
  - API policy는 `owner_only + sealed_v1`이다.
  - sealed artifact leaf는 `10001:10001/0750`이나 generic artifact root는
    `995:982/0750`; API UID/GID 10001은 parent bind mount를 traverse하지 못한다.
  - owner-beta runner는 leaf를 직접 mount하므로 healthy이고 API만 parent traversal에서
    `Permission denied` 후 static 503을 반환한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | intermediate | 원인과 최소 권한 계약이 재현됐고 수정 범위가 provisioning/test/docs로 한정되지만 host/container UID·GID 경계를 함께 보존해야 한다 | high | group 변경만으로 descriptor-safe approval이 통과하지 않거나 다른 artifact consumer 권한이 깨지면 hard로 올리고 운영 적용을 중단 |
| WP-2 | simple | route·API·정책·기존 테스트를 기계적으로 대조해 기능/행동/증거 matrix를 만드는 읽기 중심 작업 | high | 노출 기능과 정책이 서로 모순되거나 실제 product route가 문서와 다르면 intermediate로 재분류 |
| WP-3 | intermediate | CSRF→config→sealed recommendation→poll→report를 Rust DB harness와 Web browser 계약 사이에서 연결해야 하지만 기대 상태와 검증 수단이 존재한다 | high | 승인 artifact fixture가 integration test에서 구성 불가능하거나 queue/runner를 별도 설계해야 하면 hard로 올리고 추측 금지 |
| WP-4 | intermediate | 여러 사용자 화면의 데이터 의미·빈 상태·오류·접근성·권한을 종합해야 하나 기존 fixture/E2E pattern이 있다 | medium | 실제 데이터 의미가 synthetic fixture로 판정 불가능하거나 production-only 재현이 필요하면 해당 항목만 `USER_ACCEPTANCE_PENDING`으로 분리 |
| WP-5 | hard | 전체 결과를 독립적으로 검토해 “동작”과 “의미 있게 쓸 수 있음”을 구분하고 보안·운영 누락까지 판정하는 결론 자체가 산출물이다 | high | 새로운 P0/P1 결함이 나오면 coordinator가 graph를 개정하고 별도 remediation package를 만든 뒤 재리뷰 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | intermediate | API artifact-root traverse 계약을 최소 권한으로 수정하고 회귀 테스트 작성 | `scripts/ops/provision-linux.sh`, `scripts/ops/self-test.sh`, `scripts/ops/README.md`, `docs/runbooks/kis-range-canonical-stage4b.md`만 | none | Codex `gpt-5.6-luna`, max, `$paseo-delegate` | 코드·fixture·문서 변경과 운영 적용 전 정확한 owner/group/mode 계약 | focused shell self-test, syntax, static checks, diff check |
| WP-2 | 1 | simple | 모든 현재 Owner 기능, 정상 결과, 기대 차단, 필요한 데이터와 자동화 증거를 inventory | 새 파일 `docs/reviews/2026-08-26-owner-user-feature-matrix.md`만; 코드 수정 금지 | none | Codex `gpt-5.6-luna`, medium, `$paseo-delegate` | route/API/action/result/error/evidence matrix와 누락 목록 | source/test cross-reference, no network/DB/writes outside report |
| WP-3 | 2 | intermediate | 핵심 Owner 여정을 실제 브라우저 순서와 DB persistence까지 자동 검증 | 새 전용 API integration test와 새 전용 Web/E2E test 파일만; production code 수정 금지 | WP-1 accepted and coordinator permission gate | Codex `gpt-5.6-luna`, max, `$paseo-delegate` | auth/session→strategy config→owner-beta recommendation→poll→report의 성공·오류·재시도 증거 | disposable PostgreSQL 18.4 exact tests; bounded Web tests; no production auth impersonation |
| WP-R1 | 2R | intermediate | Web owner-beta 응답 스키마의 ETF11을 승인 레지스트리·고정 universe와 일치시키고 실제 성공 응답이 파싱·렌더 가능한지 회귀 고정 | `apps/web/lib/products/owner-beta-contracts.ts`, 관련 기존/new Web test·fixture 파일만 | WP-3 defect evidence | Codex `gpt-5.6-terra`, medium, `$paseo-delegate` | 정확한 ETF11 계약, registry-bound 성공 parse·surface regression | installed Next docs first; focused Vitest/typecheck/Biome; max 2 workers |
| WP-4 | 3 | intermediate | 나머지 노출 화면을 사용자로서 탐색하고 데이터 의미·빈 상태·오류·접근성·정책 차단을 검증 | 새 전용 Web/E2E test 파일과 WP-2 review report의 결과 section만; 기존 production code 수정 금지 | WP-2, WP-3, WP-R1 | Codex `gpt-5.6-terra`, medium, `$paseo-delegate` | dashboard/candidates/screener/stock/backtests/admin 및 Paper/Live 기대 차단 결과 | bounded Vitest/static browser-contract tests; 필요 시 disposable DB; max 2 workers |
| WP-R2 | 3R | simple | dashboard workspace cards를 shell과 동일한 owner-beta product policy로 필터해 비활성 Paper dead-end 제거 | dashboard page와 새 전용 focused Web test만 | WP-4 P2 finding | Codex `gpt-5.6-luna`, max, `$paseo-delegate` | owner-only/Paper-disabled에서 Paper card 숨김, 활성/일반 mode 보존 | installed Next docs first; focused Vitest/typecheck/Biome; max 2 workers |
| WP-R3 | 4R | simple | 이미 승인된 installed artifact 상태와 operator 문서를 일치시켜 잘못된 재빌드 지시 제거 | `scripts/ops/README.md`, `docs/runbooks/kis-range-canonical-stage4b.md`만 | WP-5 P2 docs finding | Codex `gpt-5.6-luna`, medium, `$paseo-delegate` | 현재 installed approval-check truth와 process-local commit 호출 계약 | exact text/source cross-check, diff check |
| WP-R4 | 4R | simple | real five-pin/ETF11 registry-bound SUCCEEDED report의 parse와 실제 report render를 한 회귀 테스트에서 직접 연결 | `apps/web/tests/wp3-owner-contract-boundary.test.ts`와 필요 시 새 전용 test만 | WP-5 P2 test finding | Codex `gpt-5.6-luna`, max, `$paseo-delegate` | exact pins/ETF11/weights/cash/factors/reasons rendered regression | focused Vitest/typecheck/Biome; max 2 workers |
| WP-R5 | 4R | simple | Owner-only beta 탐색에서 빈 Admin placeholder 링크를 숨기고 일반 Owner mode는 유지 | `apps/web/components/shell/app-shell.tsx`, `apps/web/tests/role-navigation.test.tsx`만 | WP-5 P2 Admin finding | Codex `gpt-5.6-luna`, max, `$paseo-delegate` | beta dead-end 제거, normal owner Admin 유지 | focused Vitest/typecheck/Biome; max 2 workers |
| WP-R6 | 4R | intermediate | layout/dashboard의 동일 no-store session 조회를 request-local memoization으로 deduplicate | `apps/web/lib/api/server-session.ts`와 새 전용 test만 | WP-5 P3 finding | Codex `gpt-5.6-terra`, medium, `$paseo-delegate` | per-request single session fetch without cross-request/user cache | installed React/Next docs first; focused Vitest/typecheck/Biome; max 2 workers |
| WP-5 | 4 | hard | 구현·테스트·운영 증거를 독립 리뷰하고 출시 가능한 사용자 경험인지 최종 판정 | 전체 diff·test/report read-only; 파일 수정 금지 | WP-1..WP-4, WP-R1, WP-R2, coordinator production checks | Codex `gpt-5.6-terra`, high, `$paseo-delegate` | severity별 findings, ACCEPT/NEEDS_REMEDIATION, 사용자에게 실제로 가능한 다음 행동 | evidence citations, rerun focused low-cost checks, no production mutation |

Wave 1의 두 작업은 mutable scope가 분리돼 병렬 가능하다. Wave 2와 3은 같은 Rust/Web build
자원을 쓰므로 파일이 달라도 직렬 실행한다. 발견된 결함의 수정은 기존 worker scope를 넓히지
않고 coordinator가 graph에 `WP-R*` package를 추가한 뒤 `$paseo-delegate`로 실행한다.

## Worker briefs

### WP-1 — artifact-root 권한 계약 복구

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: intermediate, high confidence. 기존 재현은 API UID/GID 10001이
  `995:982/0750` generic root를 traverse하지 못하는 것이다. group 정렬 후에도 approval이
  실패하거나 다른 consumer가 깨지면 hard로 보고하고 변경을 확대하지 않는다.
- Objective: service owner UID는 보존하면서 generic artifact root group을 worker/data GID로
  정렬해 API read-only mount와 host service account가 모두 접근 가능하도록 하고,
  provisioning 재실행이 이를 exact하게 검사하도록 한다.
- Known facts: service user는 provisioning에서 worker/data group의 supplementary member가 된다;
  artifact leaf와 backtest leaf는 이미 worker UID/GID 10001; API mount는 read-only다.
- Owned scope: `scripts/ops/provision-linux.sh`, `scripts/ops/self-test.sh`,
  `scripts/ops/README.md`, `docs/runbooks/kis-range-canonical-stage4b.md`만.
- Prohibited: production host mutation, Compose service/group broadening, chmod 0755/0770,
  recursive chown/chgrp, artifact bytes, secrets, other scripts/docs, commits.
- Required implementation: RED fixture proving old `service_gid` root contract blocks UID/GID
  10001 traversal; exact new owner/group/mode check; idempotent apply/check; documentation of why
  parent differs from leaf but remains non-world-accessible.
- Verification: focused root/fakeroot fixture if available, `bash -n` on changed scripts,
  `git diff --check`; no full workspace build.
- Report: changed files/line ranges; deviations/reasons; checks/results; unresolved/follow-up;
  not found/not verified, explicitly `none` when empty.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-2 — Owner 기능·증거 inventory

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: simple, high confidence. route, navigation, OpenAPI, handler, existing tests and
  STATUS를 기계적으로 대조한다. 정책과 구현이 모순되면 intermediate 신호로 보고한다.
- Objective: 실제 Owner가 보는 각 화면/행동에 대해 선행 데이터, 성공 결과, expected block,
  기존 자동화 증거, production-only acceptance를 한 표로 만든다.
- Owned scope: 새 `docs/reviews/2026-08-26-owner-user-feature-matrix.md`만.
- Inspect: authenticated Next routes/navigation/product gates, API contract routes, relevant
  Rust/Web/E2E tests, owner-beta release plans, current STATUS §§0.38–0.41.
- Prohibited: source/test edits, network, DB/Docker, credentials, production access, performance
  claims, treating Paper/Live disabled as a defect.
- Expected output: route별 `WORKING_EVIDENCE`, `TEST_GAP`, `EXPECTED_BLOCK`,
  `USER_ACCEPTANCE_PENDING`; 핵심 여정 순서와 사용자 가치 설명.
- Verification: every row has source and test evidence or explicitly states none; `git diff --check`.
- Report format: standard changed/deviation/check/unresolved/not-verified fields.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-3 — 핵심 전략→추천 사용자 여정

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: intermediate, high confidence. Escalate if a valid approved artifact cannot be
  constructed from existing approved fixture/test support; never invent pins or registry bytes.
- Objective: browser-equivalent session/CSRF sequence로 active strategy config를 만들고,
  `as_of=2026-08-19` owner-beta recommendation을 enqueue하여 worker publication, polling,
  `SUCCEEDED` report, ETF11 items/weights/cash/reasons/pins, idempotent replay까지 검증한다.
- Owned scope: 새 전용 files only under `crates/api-server/tests/` and `apps/web/tests/` or
  `apps/web/tests/e2e/`; existing files or production code require coordinator graph revision.
- Inputs: WP-1 accepted contract; checked-in sole approved registry and existing sealed artifact
  fixtures; existing ScratchDb/synthetic API patterns.
- Prohibited: Auth0 user impersonation/invite, production mutation, live provider/network,
  account/order/Paper/Live, fabricated approval pins, printing bodies/secrets.
- Verification: one uniquely named disposable PostgreSQL 18.4 project with exact cleanup trap;
  focused Rust tests with `CARGO_BUILD_JOBS=2` and `nice=12`; focused Web test with max 2 workers;
  CSRF negative and retry behavior; no full suite unless coordinator explicitly adds a gate.
- Expected output: exact PASS/FAIL evidence, elapsed time/resource notes, and any UI/API seam that
  prevents a normal user from understanding or completing the run.
- Report format: standard changed/deviation/check/unresolved/not-verified fields.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-4 — 나머지 사용자 화면·정책 수용성

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: intermediate, medium confidence. Synthetic fixtures로 실제 의미를 증명할 수 없는
  production-only 항목은 실패로 꾸미지 말고 `USER_ACCEPTANCE_PENDING`으로 분리한다.
- Objective: dashboard, candidates, screener save/delete, stock detail, owner-beta backtests,
  admin read/status, locale/theme, keyboard/accessibility, refresh/deep-link/error states를 사용자
  작업 단위로 검증한다. Paper hidden/direct block, Live hidden/direct block도 확인한다.
- Owned scope: 새 전용 Web/E2E test files, 그리고 WP-2 report의 결과 section append만.
- Prohibited: existing production source edit, enabling Paper/Live, real orders/accounts,
  provider network, credentials, production data mutation, broad screenshot retention.
- Verification: focused Vitest/Playwright max 2 workers; existing synthetic API fixture; 필요할 때만
  one disposable DB; non-secret sanitized evidence only. CPU-heavy suites are sequential.
- Expected output: each journey's task, expected value, actual behavior, recovery clarity,
  accessibility and blocking reason; P0/P1/P2 severity recommendations.
- Report format: standard changed/deviation/check/unresolved/not-verified fields.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-R1 — Web ETF11 owner-beta 계약 복구

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: intermediate. WP-3가 승인 레지스트리/고정 universe의 ETF11 중 7개를 현재
  `ownerBetaInstrumentSchema`가 거부함을 재현했다. 이는 실제 API 성공 응답도 Web이 파싱하지
  못하게 하는 사용자 차단 결함이다. 넓은 계약 교차검증이 필요해 Luna 대신 Terra medium으로
  상향한다.
- Objective: Web owner-beta instrument allowlist를 checked-in source of truth와 정확히 맞추고,
  registry-bound 11-item SUCCEEDED 응답의 parse와 결과 surface 의미(ETF11, weight/cash,
  factors/reasons)가 통과하도록 회귀 테스트를 RED→GREEN으로 바꾼다.
- Owned scope: `apps/web/lib/products/owner-beta-contracts.ts`, 기존/new owner-beta Web test 및
  synthetic recommendation fixture 중 필요한 파일만. API/Rust/config/registry/universe 수정 금지.
- Required precondition: `apps/web/AGENTS.md` 지침대로 현재 설치된
  `node_modules/next/dist/docs/`의 관련 testing/config 문서를 먼저 읽고 행동에 반영한다.
- Prohibited: generic instrument 문자열로 완화, registry를 runtime 브라우저에 번들, 서버 계약
  변경, Auth0/production/provider 접속, Paper/Live, 기존 승인 pin 발명, unrelated refactor.
- Verification: focused Vitest, `npm run typecheck`, targeted Biome with max 2 workers; Chromium
  host library 누락이면 E2E는 재시도하지 말고 기존 environment block을 유지한다.
- Report format: standard changed/deviation/check/unresolved/not-verified fields.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-R2 — dashboard owner-beta policy 정렬

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: simple. WP-4가 shell은 Paper를 숨기지만 dashboard가 항상 `/paper` 카드를 보여
  비활성 페이지로 유도하는 P2 dead-end을 확인했다.
- Objective: dashboard가 `permitsOwnerBetaProduct`와 현재 server session을 사용해 shell과 동일한
  recommendations/backtests/Paper 표시 정책을 적용하고, 일반 mode와 Paper-enabled mode는 기존
  카드를 보존한다.
- Owned scope: `apps/web/app/(authenticated)/page.tsx`와 새 전용 dashboard policy test 하나만.
- Required precondition: 현재 설치된 Next server-components/testing 문서를 먼저 읽는다.
- Prohibited: Admin/navigation/API/session 계약 변경, policy 완화, Paper/Live 활성화, network,
  production, package/lockfile, unrelated refactor.
- Verification: focused Vitest max 2 workers, Web typecheck, targeted Biome, `git diff --check`.
- Report format: standard changed/deviation/check/unresolved/not-verified fields.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-R3..R6 — WP-5 closure packages

- WP-R3 (simple): 두 operator 문서의 stale “installed registry empty/new release required” 문장을
  STATUS §0.37과 2026-08-27 real approval-check에 맞춘다. wrapper가 commit을 env file이 아닌
  설치 commit의 process-local `LAGRANGE_CODE_COMMIT`으로 요구하는 계약도 정확히 기록한다.
- WP-R4 (simple): registry bytes에서 real five pins/ETF11을 읽은 `SUCCEEDED` model을 strict parse한
  뒤 실제 `OwnerBetaReport` surface까지 렌더해 ETF11, cash/weight, factor/reason, 제한 label을 한
  테스트에서 직접 검증한다. production runtime에 registry를 import하지 않는다.
- WP-R5 (simple): `owner_beta_access_mode=owner_only` Owner에게 내용 없는 `/admin` nav를 숨긴다.
  normal disabled mode Owner에게는 기존 Admin link를 유지하고 Member/Live/Paper 정책은 건드리지
  않는다. direct `/admin`의 OwnerRoute/empty block은 defense-in-depth로 유지한다.
- WP-R6 (intermediate): React request memoization만 사용해 layout/page의 동일 session fetch를 한
  요청 안에서 deduplicate한다. user/session 값을 persistent/route/global cache에 저장하지 않고,
  cross-request 공유가 없음을 installed React/Next docs와 focused test로 확인한다.
- 각 package는 명시 파일만 수정하며 network/production/Auth0/provider/Paper/Live/order 접근,
  package 설치, Playwright, full build/suite를 금지한다.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

### WP-5 — 독립 최종 수용성 리뷰

- Target cwd: `/data/worktrees/3puw275b/rural-mouse`.
- Complexity: hard, high confidence. 결론 자체가 산출물이며 security, persistence, usability,
  operational evidence를 교차 검증한다.
- Objective: WP-1..4 결과와 coordinator의 최소 운영 수정·read-only production checks를
  검토하고 사용자가 매뉴얼 없이 핵심 가치를 얻는지 판정한다.
- Owned scope: 전체 diff/report/test evidence read-only; no edits.
- Prohibited: native delegation, production mutation, Auth0 impersonation, provider/account/order
  network, unsupported performance or release claims.
- Required output: severity별 file:line/evidence findings; prior known failure closure; 자동화된
  것과 실제 Owner 클릭이 필요한 것의 분리; `ACCEPT` 또는 `NEEDS_REMEDIATION`.
- Verification: low-cost focused reruns only, `git diff --check`, status/diff inspection. Rust
  recompilation or full Playwright suite는 기존 증거가 불충분할 때만 coordinator와 상의한다.
- Report format: changed files/lines (`none`); deviations; commands/results;
  unresolved/follow-up; not found/not verified.
- Mandatory: `Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

## Coordinator gates

1. Pre-launch
   - worktree/main/origin-main이 `67fe14f`로 정렬되고 preserved XLSX가 이 worktree에 없음을
     확인한다.
   - Paseo CLI와 worker provider/model 조합을 확인한다.
   - WP-1/WP-2 prompt가 scope, report format, mandatory no-delegation sentence를 포함하는지
     확인한다.
   - 사용자 추가 결정은 없다. current request에 따라 narrow fail-closed 권장안을 채택한다.
2. Wave 1 integration
   - `paseo wait --json` status가 둘 다 `idle`인지 확인하고 worker 주장과 diff를 직접 검증한다.
   - WP-1의 권한 변경이 parent group만 바꾸며 owner/mode/leaf bytes를 보존하는지 확인한다.
   - 코드 fixture가 통과한 뒤 coordinator만 production generic artifact root에 exact group
     변경을 적용하고 전후 metadata/API traversal을 확인한다. 실패 시 원복하고 Wave 2 중단.
3. Wave 2 gate
   - WP-3을 단독 실행해 CPU/메모리 사용을 제한한다.
   - core journey가 ETF11/5-pin/가격수익률/비엄격 PIT를 보존하고 DB에 정확히 한 결과를
     원자적으로 남기는지 확인한다.
   - defect가 나오면 새 `WP-R*`를 분류·문서화한 뒤 `$paseo-delegate`로 수정하고 재검증한다.
4. Wave 3 gate
   - WP-4를 단독 실행해 나머지 화면을 검증한다.
   - “화면이 뜬다”가 아니라 사용 목적, 데이터 의미, 빈/오류 복구가 분명한지 확인한다.
5. Production read-only acceptance
   - exact revision/health, API artifact traversal, strategy config count, anonymous auth boundary,
     Funnel health를 non-secret 형태로 확인한다.
   - 실제 Auth0 세션이 필요한 마지막 recommendation/backtest 클릭은 대리하지 않는다.
     자동화 증거가 모두 통과하면 사용자에게 다음날 1–2개의 최소 acceptance 행동만 남긴다.
6. Final review and handoff
   - WP-5 verdict를 직접 검증하고 findings가 있으면 graph를 개정한다.
   - focused tests, format/lint/diff checks를 자원 제한 아래 통합 실행한다.
   - `docs/STATUS.md`, 이 계획의 실행 상태, Basic Memory(연결 가능 시), Mem0에 결과를 기록한다.
   - 논리적 변경을 commit하고 main/origin-main 반영 여부를 명확히 보고한다. 기존 XLSX는
     계속 보존한다.

## Execution status

- 2026-08-26 Wave 1 launched through `$paseo-delegate` on clean
  `audit-project-status@67fe14f`:
  - WP-1: Codex `gpt-5.6-luna`, max, agent
    `07364e7a-fc8f-43c9-b0c7-3418f40f68a2`.
  - WP-2: Codex `gpt-5.6-luna`, medium, agent
    `0317777e-949e-4528-804c-0b86373010da`.
- Provider availability was checked with Paseo 0.4.0 before launch. Both packages use the shared
  workspace but own disjoint mutable files. No native subagent was created.
- WP-1 agent `07364e7a-fc8f-43c9-b0c7-3418f40f68a2` was stopped before edits after repeated broad
  inspection without implementation. Per repository escalation rules it was replaced by WP-1R,
  Codex `gpt-5.6-terra`, medium, agent `90652774-9a9f-4d18-8d42-6a1a29730150`, with the approved
  four-file design and prohibited adjacent inspection stated explicitly.
- WP-2 completed `idle`; its sole output is
  `docs/reviews/2026-08-26-owner-user-feature-matrix.md` with 22 classified user actions.
- WP-1R completed `idle` with only its four owned files changed. Coordinator review reran
  `bash -n` for both shell files and `git diff --check` successfully.
- Coordinator applied the exact production metadata correction to
  `/var/lib/lagrange/data/artifacts`: group `982 -> 10001`; owner `995`, mode `0750`, device/inode
  `64512:2657247`, children, and bytes were unchanged. The updated provisioning `--preflight`
  passed. API UID/GID 10001 can now stat the dedicated leaf and its v2 control directory; both API
  and owner-beta runner remain `running|healthy`.
- Wave 2 WP-3 launched through `$paseo-delegate`: Codex `gpt-5.6-luna`, max, agent
  `c04a5c64-f478-4027-b176-8380e3cc0e25`. It was the only compile/DB-test worker active.
- WP-3 completed all file/test work but its final response was interrupted after an exhausted Mem0
  write stalled; coordinator stopped only that idle post-work call and directly reviewed the diff/log.
  Its scope is exactly three new test files. The disposable PostgreSQL 18.4 API test passed 1/1 for
  authenticated session, CSRF rotation, active config persistence, missing-CSRF rejection,
  fail-closed `OWNER_BETA_PRICE_INPUT_UNAVAILABLE`, zero run/job side effects, and same-key retry.
  Web typecheck, focused Vitest, Biome, rustfmt, and diff checks passed. Chromium E2E could not start
  because this host lacks `libasound.so.2`; no system package was installed and all loopback/Compose
  processes were cleaned.
- WP-3 cannot honestly prove the full `SUCCEEDED` queue/runner journey because the checked-in v2
  approval registry contains only immutable metadata/pins, not the approved 17,688-bar artifact, and
  the production approval type is deliberately nonconstructible from integration tests. This remains
  partial evidence, not a fabricated success.
- WP-3 found a release-blocking Web seam: `ownerBetaInstrumentSchema` contains seven instruments that
  differ from both the sole approved registry record and `kr-etf-core-v1`. The dedicated registry-bound
  test proves the current Web parser rejects items 4–10. WP-R1 was added before Wave 3.
- WP-R1 completed `idle`: Codex `gpt-5.6-terra`, medium, agent
  `0e34323e-f7c2-4f84-90df-5df003242b47`. It read the installed Next Vitest/TypeScript guidance,
  retained a strict `z.enum`, replaced only the seven obsolete symbols, updated the two synthetic
  owner-beta sources, converted the registry-bound test to successful parsing, and now asserts all
  eleven instruments render. Focused Vitest passed 11/11; Web typecheck, targeted Biome, and
  `git diff --check` passed. Playwright remained intentionally unrun because `libasound.so.2` is
  absent. Coordinator reviewed the exact five-file Web diff; no Rust/config/registry scope leaked.
- WP-4 was reclassified from Luna max to Terra medium before launch. The remaining surface audit is
  a broad high-recall comparison across routes, fixtures, usability states, and policy gates; a
  confident partial inventory from a lower-tier wide-context scan would be harder to detect than a
  local test failure. The mutable scope and resource limits are unchanged.
- WP-4 completed `idle`: Codex `gpt-5.6-terra`, medium, agent
  `ed2b9f4a-9120-4d6f-99c7-f87230a01109`. It added one dedicated test and a dated matrix section.
  Focused Vitest passed 33/33; typecheck, targeted Biome, and diff checks passed. Candidates,
  screener, stock, and backtest fixture contracts are sound but production Owner actions remain
  pending. Paper/Live and the Admin no-area placeholder are expected blocks. It found no P0/P1 and
  two P2 gaps: a dashboard Paper dead-end and the absence of a meaningful Admin UI.
- Coordinator decision: fix the bounded dashboard dead-end now as WP-R2. Do not invent or wire a
  partial Admin surface: a useful read-only datasets/jobs/workers/audit UI needs a separate approved
  product contract and remains `EXPECTED_BLOCK` for this fast Owner-only release.
- WP-R2 completed `idle`: Codex `gpt-5.6-luna`, max, agent
  `ff2d6081-ba6c-4128-bb5a-a68757c84839`. The async server page now resolves locale/session in
  parallel and filters all three beta workspace destinations through `permitsOwnerBetaProduct`.
  Its dedicated rendered-page Vitest passed 4/4 for Owner Paper disabled/enabled, normal mode, and
  Owner-beta Member behavior. Web typecheck, targeted Biome, and diff checks passed; coordinator
  reviewed the exact two-file scope.
- Coordinator production read-only gate: API, Web, owner-beta runner, both backtest workers,
  candidate/recommendation/research workers, PostgreSQL, and reverse proxy are healthy. API/Web/
  owner-beta runner report installed revision `037e686da1426260521b4c795bde47d7b5b0c5cf`.
  API UID 10001 can stat artifact parent/leaf/control/v2 with exact modes and GIDs. DB aggregates are
  `strategy_configs=1`, `active=1`, `owner_beta_runs=0`, migration max `52`. Public Funnel root is
  `307`, login handoff `303`, and anonymous session `401` JSON. No cookie, token, user ID, provider
  response, or mutation was used. Source branch/main/origin-main remain at `67fe14f` before the
  current uncommitted audit changes.
- The installed immutable-release approval check also passed after supplying its intentionally
  process-local installed commit: registry hash `4111f51d…5e3380`, status `APPROVED`, exact
  `OWNER_ONLY`/price-return/vendor-snapshot/non-strict-PIT envelope, and `11/1608/17688` coverage.
  The first worktree invocation and the first installed invocation without that required process
  value both failed closed before Docker/artifact validation; no file or artifact was changed.
- WP-5 independent review completed `idle`: Codex `gpt-5.6-terra`, high, agent
  `dfb60389-d8eb-4fe3-9237-45475f1fa229`, verdict `NEEDS_REMEDIATION`. It found no P0/P1 and
  confirmed the Owner can meaningfully retry recommendation, but identified P2 stale operator docs,
  P2 exact-pin parse/render evidence split, P2 empty Admin beta navigation, and P3 duplicate dashboard
  session fetch. WP-R3..R6 were added; the same reviewer must close them before final ACCEPT.
- WP-R3 completed `idle`: Codex `gpt-5.6-luna`, medium, agent
  `7c2be5e8-fc65-4b73-afd5-bf8a064eaff1`. Only the two operator documents changed; stale registry
  claims are gone and the installed process-local commit/approval contract is exact. Static scan and
  diff checks passed.
- WP-R4 completed `idle`: Codex `gpt-5.6-luna`, max, agent
  `ea2b59d9-26d5-4f49-90a8-0e80b3c2c8ed`. The registry-bound test now strict-parses and renders
  the exact five commitments/ETF11 model, reconciles 80% cash + 20% selected, and asserts factors,
  reasons, and beta limitations. Focused Vitest 1/1, typecheck, Biome, and diff checks passed.
- WP-R5 completed `idle`: Codex `gpt-5.6-luna`, max, agent
  `402fecdc-e5ad-4788-95f2-6c10718085c9`. Owner-only beta now hides empty Admin and Live while
  normal Owner retains both; Member and product policies are unchanged. Focused Vitest 6/6,
  typecheck, Biome, and diff checks passed.
- WP-R6 completed `idle`: Codex `gpt-5.6-terra`, medium, agent
  `6b637bb0-8b85-44c8-90bf-8dc9ace0b70e`. `getServerSession` uses React `cache` exactly as the
  installed Next 16.3 docs specify for same-request Server Component deduplication; cookies remain
  dynamic and the transport remains no-store with no persistent/shared cache. Focused Vitest,
  typecheck, Biome, and diff checks passed. Vitest honestly proves wiring rather than simulating a
  cross-request RSC dispatcher lifecycle.
- The same WP-5 reviewer completed the remediation closure `idle` with verdict `ACCEPT` for source
  readiness. It closed every prior P2/P3 functional finding and found no P0/P1/P2 regression,
  credential/account/order/trading enablement, provider/Raw leak, or unsupported readiness/PIT/return
  claim. Its low-cost rerun passed 5 Web files / 23 tests with two workers, both shell syntax checks,
  and tracked diff checks. One P3 editorial mismatch and one Markdown trailing-space in the feature
  matrix were corrected directly by the coordinator after review; neither affected runtime behavior.
- Final boundary: the Owner can meaningfully retry the installed production recommendation, but only
  the Owner may perform the Auth0 action. A durable production `SUCCEEDED` run/report, production
  candidate/screener/stock value, and the Owner backtest lifecycle remain user-acceptance evidence,
  not source defects. Current audit source is not yet deployed; installed production remains
  `037e686da1426260521b4c795bde47d7b5b0c5cf` until a later explicit release operation.
