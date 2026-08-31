Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# 웹 관리형 개인종목 유니버스 V2 실행 계약

## Goal and boundaries

### 목표와 완료 기준

Owner가 `/stock-beta`에서 코드나 배포 파일을 수정하지 않고 국내 6자리 종목코드를
추가·비활성화·재시도할 수 있는 `owner-managed-equity-universe-v2`를 만든다. 종목 추가는
동기식 장시간 요청이 아니라 아래 비동기 상태 전이로 처리한다.

`REQUESTED -> VALIDATING -> BACKFILLING -> MATERIALIZING -> READY`

데이터가 부족하면 `INSUFFICIENT_HISTORY`, 재시도 가능한 실패는 `FAILED`, 소유자가 제외하면
`DISABLED`로 전이한다. 모든 상태 전이는 owner, instrument, generation, code revision,
entitlement hash와 감사 이벤트에 결합한다.

완료 조건은 다음과 같다.

1. 인증된 Owner가 정확한 6자리 코드 하나를 제출하면 API는 202와 durable resource/status를
   반환한다. 동일 idempotency key 재전송은 같은 결과를 반환하고, 이미 활성인 종목의 중복
   추가는 새 백필을 만들지 않는다.
2. Member와 비인증 사용자는 목록·상태·데이터를 읽거나 변경할 수 없다. CSRF, owner RLS,
   요청 크기, 형식, 중복, 활성 종목 한도를 fail-closed 검증한다.
3. 설치된 worker는 KIS의 허용된 읽기 전용 reference quote와 daily-bars GET/TR만 사용해
   종목을 확인하고, 요청 범위·예상 GET 수를 먼저 계산한 뒤 기존 TokenManager와 endpoint/TR
   채널당 1 rps, 순차 실행, bounded retry를 지킨다.
4. 응답은 기존 Raw 불변 저장소에 검증 후 커밋하며, 종목별 materialized generation은 Raw
   batch/file hashes, entitlement, capture/materializer commit과 coverage를 보존한다. API나 Web은
   Raw 응답 본문과 provider 자유문자열을 받지 않는다.
5. 최소 121개의 유효한 관측이 확보된 종목만 20/60/120-session factor 대상이 된다. 추가한
   종목이 아직 준비되지 않았을 때 기존 READY 순위는 유지하고, READY 또는 DISABLED 전이 때
   exact active-ready universe hash로 새 cross-sectional snapshot을 원자 발행한다.
6. 새 종목이 READY가 되면 전체 순위와 Top 5에 반영되고, 기존 종목의 변경된 순위도 같은
   snapshot version으로 노출된다. 과거 snapshot은 당시 universe/hash와 함께 재현 가능하다.
7. 웹은 고정 30개 배열이나 `max(30)` 응답 가정을 사용하지 않고 추가 상태, coverage,
   실패 코드, 재시도, 비활성화를 표시한다. 종목 하나를 추가할 때 애플리케이션 재빌드나
   재배포가 필요하지 않다.
8. 기존 `kr-stock-price-beta-v1`의 30종목 Raw, artifact, approval registry, API read path와
   배포 회귀 테스트는 byte/behavior 호환으로 유지한다.
9. fixture 기반 전체 경로, tamper/idempotency/RLS/restart 테스트, Rust/Web/static 검증과
   provider-free 브라우저 QA가 모두 통과한다. 운영 활성화가 별도로 승인되면 exact-commit
   순차 빌드·설치 후 실제 Owner 세션에서 한 종목의 add-to-READY 흐름을 확인한다.

### 고정 설계 결정

- V1을 mutable하게 바꾸지 않는다. V2는 별도 DB tables, job type, collector/materializer
  contract, API와 Web mutation을 사용한다.
- 종목 목록의 단일 운영 원본은 owner-scoped DB membership이다. Rust 배열, 셸 목록,
  이미지에 포함된 universe JSON을 운영 원본으로 사용하지 않는다.
- 종목별 Raw와 materialized generation을 분리한다. 한 종목 추가 때문에 기존 종목을 다시
  수집하지 않는다. 전체 cross-sectional signal snapshot만 재계산한다.
- 정적 수동 approval registry를 임의 종목마다 생성하지 않는다. 대신 설치된 immutable
  release의 provider-free verifier가 통과한 generation만 DB admission record에 exact pins와
  함께 원자 기록하며, API는 admitted generation만 읽는다. 이 변경은 검증을 제거하는 것이
  아니라 승인 단위를 release-time fixed list에서 runtime per-instrument evidence로 바꾼다.
- 삭제는 물리 삭제가 아니라 `DISABLED`다. membership과 signal 노출은 중단하되 Raw,
  artifact, audit, 과거 snapshot은 보존한다.
- 초기 제품 입력은 정확한 6자리 코드다. 이름 자동완성·전 종목 검색은 공식 security master
  계약이 없으므로 이번 범위에 넣지 않는다. KIS가 확인한 이름은 검증 성공 후 표시한다.
- 현 승인 자료만으로 보통주/ETF/ETN 구분을 독립 증명하지 못하므로 V2는
  `owner-configured KRX research instrument`라고 표시한다. 보통주만 허용해야 한다면 공식
  instrument master의 별도 승인과 계약을 먼저 추가해야 한다.
- readiness 기본값은 최근 261 observed sessions 목표, 최소 121 observations다. 10년 백필과
  기업행사 조정은 후속 계약이며 READY를 막지 않는다.
- 활성 종목 한도는 코드 상수가 아닌 배포 정책으로 둔다. 초기 권장값은 Owner당 100개이며,
  API·worker·Web이 동일한 typed policy를 사용한다. 한도를 바꿔도 스키마나 재빌드가 필요하지
  않아야 한다.

### 대상 workspace와 주요 경로

- workspace: `/data/worktrees/3puw275b/hungry-zebra`
- 보존할 V1: `configs/universes/kr-stock-price-beta-v1.json`,
  `configs/evidence/kr-stock-price-beta-v1-approved-artifacts.json`,
  `crates/market-data/src/fixed_stock_price_beta*`, 기존 V1 collector/wrapper/API 계약
- V2 foundation: `migrations/0053_*`, `crates/domain`, migration contract tests
- V2 data path: `crates/market-data`, `data-pipelines/collectors`, `crates/factor-engine`
- V2 orchestration/API: `crates/job-queue`, `crates/api-server`, `apps/api-server/scripts/openapi-spec.mjs`
- V2 UX: `apps/web/app/(authenticated)/stock-beta`, `apps/web/components/stock-beta`,
  Web contracts/client/i18n/tests
- runtime/QA: `deploy/compose`, production Dockerfiles, `scripts/ops`, runbook/STATUS

### In scope

- Owner-only add/list/status/retry/disable API와 Web UX
- owner-scoped membership, per-instrument generation, immutable admission, signal snapshot DB 계약
- 허용된 KIS read-only validation, initial backfill와 활성 READY 종목의 once-daily incremental EOD
- per-instrument Raw/materialization, coverage와 typed failure evidence
- dynamic-universe price/volume factors, deterministic rank와 atomic snapshot publication
- queue idempotency, crash/retry recovery, concurrency·request-budget enforcement
- provider-free fixture/E2E, image/Compose/static wiring, 운영 runbook

### Out of scope

- 주문, 계좌, 잔고, Paper/Live, WebSocket 또는 KIS allowlist 밖 endpoint
- 목표가, 상승확률, 종목 비중, 매수·매도 표현
- formal six-source PIT candidate, ETF11 recommendation/artifact 변경
- 보통주/ETF/ETN을 증명하는 신규 master source, 종목명 자동검색, 전 시장 bulk import
- corporate-action adjusted return, 10년 개인종목 history, Raw 재배포
- 이 계획만으로 production provider 호출, 실제 데이터 수집, 이미지 push, release 설치 또는 배포

### 적용 지침과 미해결 요구

- 루트 `AGENTS.md`의 KIS deny-by-default allowlist, 계좌·주문 금지, TokenManager 재사용,
  daily-bars/reference quote single-page, 1 rps, bounded retry, typed error, secret/body 비노출,
  immutable Raw와 owner-only rights가 모든 package에 적용된다.
- `apps/web/AGENTS.md`와 `apps/web/CLAUDE.md`에 따라 Web 작업자는 코드 수정 전에 설치된
  `node_modules/next/dist/docs/`의 해당 Next.js 문서를 읽어야 한다.
- repository routing은 단순 작업에 luna low/medium, 명세 확정 구현에 luna max, 반복 실패 시
  terra, 아키텍처·고위험 열린 문제에 sol high 이상, 독립 코드리뷰에 terra medium/high를
  사용한다. 낮은 tier 결과는 기계적 검증이나 coordinator 직접 확인 없이는 채택하지 않는다.
- 초기 100종목 한도와 261-session 목표는 권장 기본값이다. 실행 전 Owner가 다른 값을
  명시하면 WP-1 typed policy만 수정하고 고정 cardinality를 다른 계층에 복제하지 않는다.
- production activation의 하루 최대 신규 add 수와 실제 Owner QA 종목은 실행 직전 운영
  preflight에서 확정한다. 미확정 상태에서는 fixture/provider-free 검증까지만 진행한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 domain·schema foundation | hard | owner RLS, 상태 전이, generation/admission/snapshot 불변성과 rollback이 이후 전 계층 계약을 결정한다 | high | 기존 jobs/RLS와 원자 전이를 증명하지 못하거나 migration 검증이 두 번 실패하면 sol xhigh로 상향하고 schema를 재검토 |
| WP-2 per-instrument data pipeline | hard | 실제 provider 안전 경계와 immutable Raw/artifact, 동적 request budget, crash recovery가 결합된다 | high | 새 endpoint/TR/provider 수정이 필요하면 작업을 중단하고 사용자 승인 전 재계획; 동일 수정이 두 번 실패하면 sol xhigh |
| WP-3 dynamic factor snapshot | intermediate | 기존 factor 수식은 재사용 가능하고 입력·출력 계약과 결정적 검증 방법이 명확하다 | high | 서로 다른 as-of/coverage 때문에 비교 가능성을 증명하지 못하거나 corporate-action/PIT 요구가 생기면 hard로 재분류 |
| WP-4 queue·API orchestration | hard | owner auth/RLS, idempotent enqueue, 여러 worker 상태, atomic snapshot publication과 장애 복구가 교차한다 | high | API transaction과 worker claim 사이 중복 generation 가능성, stale write, replay mismatch가 발견되면 sol xhigh와 계약 수정 |
| WP-5 Web management UX | intermediate | 확정 API 위 add/status/retry/disable UI이며 unit/E2E로 검증 가능하다 | high | 현재 Next.js mutation/session recovery 패턴으로 구현할 수 없거나 두 번 검증 실패하면 terra medium으로 상향 |
| WP-6 runtime·ops integration | intermediate | 기존 immutable-release/Compose/static-check 패턴을 새 worker와 정책에 적용하는 범위다 | medium | secret mount, provider concurrency, release rollback을 정적으로 증명할 수 없거나 실제 운영 변경이 필요하면 hard로 재분류하고 중단 |
| WP-7 independent review·QA | intermediate | 구현 diff와 명시된 불변성을 독립 검사하고 결정적 테스트로 판정할 수 있다 | high | 보안·증거·RLS 위반이나 재현 불가 장애가 나오면 해당 branch를 반려하고 hard remediation package를 새로 계획 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | hard | V2 상태·DB·불변성 계약 고정 | 새 ADR, `migrations/0053_*`, domain V2 module, migration-contract tests | 없음 | `$paseo-delegate`: gpt-5.6-sol high; 반복 실패 시 xhigh | typed policy/state/generation 모델, reversible migration, RLS와 DB invariants | domain tests, `cargo test -p migration-contract`, fmt/check |
| WP-2 | 2 | hard | 임의 단일 종목의 안전한 validation/backfill/materialization | 새 market-data/collectors V2 modules·bins·fixtures/tests; 기존 KIS generic API는 read-only | WP-1 | `$paseo-delegate`: gpt-5.6-sol high | per-instrument immutable generation과 provider-free verifier | market-data/collectors focused+all-target tests, tamper/no-secret/request-budget tests |
| WP-3 | 3 | intermediate | READY 집합으로 결정적 dynamic snapshot 생성 | 새 factor-engine V2 module/tests만 | WP-1, WP-2 | `$paseo-delegate`: gpt-5.6-luna max; 두 번 실패 시 terra medium | eligibility, score/rank, universe hash, snapshot candidate | factor focused/all-target tests, permutation/idempotency/as-of tests |
| WP-4 | 4 | hard | DB·queue·worker·Owner API를 하나의 원자 lifecycle로 연결 | job-queue V2, api-server repo/http/state/router/DTO/OpenAPI와 tests | WP-1~3 | `$paseo-delegate`: gpt-5.6-sol high | add/list/status/retry/disable, worker runner/recovery, atomic publish | API/job focused+all-target, RLS/idempotency/crash/replay/OpenAPI tests |
| WP-5 | 5 | intermediate | 웹에서 종목 lifecycle을 관리 | `apps/web` stock-beta route/components/contracts/client/i18n/unit/E2E | WP-4 | `$paseo-delegate`: gpt-5.6-luna max; 검증 반복 실패 시 terra medium | add form, status/coverage, retry/disable, dynamic rows | Web typecheck/lint/unit/build/provider-free Playwright |
| WP-6 | 5 | intermediate | worker와 정책을 immutable runtime에 안전하게 연결 | Dockerfiles, Compose, ops/static/self-tests, 새 runbook | WP-2, WP-4 | `$paseo-delegate`: gpt-5.6-luna max; runtime 증명 실패 시 terra medium | installed-release-only runner, mounts/env/concurrency/preflight | ops static/self-tests, Compose config, image static checks, network-none verifier |
| WP-7 | 6 | intermediate | 전체 구현 독립 감사와 QA 판정 | read-only 전체 diff·tests·fixture runtime; 파일 수정 금지 | WP-1~6 | `$paseo-delegate`: gpt-5.6-terra high | severity별 code/security/data review와 acceptance report | full Rust/Web/static/E2E, V1 regression, secret/order-surface scans |

Wave 5의 WP-5와 WP-6만 병렬 실행할 수 있다. 두 package는 mutable scope가 겹치지 않으며
WP-4 API 계약이 고정된 뒤 시작한다. 그 외 package는 데이터·타입·상태 전이 의존성이 있어
순차 실행한다. 모든 worker는 위 workspace에서 직접 작업하되 추가 delegation을 하지 않는다.

## Worker briefs

### WP-1 — domain·schema foundation

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** hard; schema가 잘못되면 Raw admission과 사용자 상태가 서로 다른
  진실을 갖게 된다. confidence high. jobs/RLS와 원자성을 증명하지 못하면 sol xhigh로 올리고
  다음 wave를 시작하지 않는다.
- **Objective:** `owner-managed-equity-universe-v2`의 typed policy, 상태 전이, membership,
  per-instrument generation, signal snapshot과 row lineage를 DB/domain 계약으로 고정한다.
- **Known facts:** migration은 0052까지 존재한다. V1은 filesystem sealed snapshot이며 새 V2
  DB에 이식하거나 수정하지 않는다. owner role/RLS와 기존 `jobs` idempotency 패턴을 재사용한다.
- **Owned scope:** 새 `docs/decisions/0008-owner-managed-equity-universe-v2.md`,
  `migrations/0053_owner_managed_equity_universe_v2.{up,down}.sql`, domain V2 module/export,
  `tests/integration/migration-contract/tests/migration_contract.rs`의 0053 전용 테스트.
- **Required contract:** owner+instrument unique active membership; generation monotonicity;
  legal state-transition check; immutable content pins after admission; exact snapshot-universe hash;
  snapshot rows가 admitted generation만 참조; soft disable; actor-scoped RLS; rollback fail-closed;
  runtime policy에는 max-active와 history target/minimum을 둔다.
- **Prohibited adjacent work:** collector/factor/API/Web/Compose 수정, V1 table/artifact/registry 수정,
  provider 호출, account/order schema 추가.
- **Inputs/dependencies:** 루트 지침, 본 계획의 상태·정책 결정, 기존 jobs/owner-beta migration 패턴.
- **Deliverable/verification:** ADR와 reversible schema/domain types; unit/property tests와 disposable
  DB migration contract. `cargo fmt --all -- --check`, `cargo test -p domain`,
  `cargo test -p migration-contract`, `git diff --check`.
- **Required report:** 변경 파일/라인; brief 이탈과 이유; 실행한 검사/결과; 미해결·후속;
  찾지 못하거나 검증하지 못한 것. 빈 항목은 반드시 `none`.

### WP-2 — per-instrument data pipeline

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** hard; KIS network safety, immutable evidence, retry/recovery가 결합된다.
  confidence high. 새 endpoint/TR 또는 기존 provider semantics 변경이 필요하면 구현하지 말고
  blocker를 보고한다.
- **Objective:** DB/job에서 받은 단일 canonical instrument ID와 requested range로 validation,
  initial/incremental capture, provider-free materialization/check를 수행하는 V2 engine을 만든다.
- **Known facts:** 허용 surface는 reference quote `FHKST01010100`와 daily bars
  `FHKST03010100`; 둘 다 single-page/blank continuation이다. 기존 TokenManager, rate limiter,
  retries, Raw store와 generic KIS constructor/range fetch seam을 재사용한다.
- **Owned scope:** 새 `crates/market-data/src/owner_equity_v2*`, market-data export와 focused tests;
  새 `data-pipelines/collectors/src/owner_equity_v2*`, collector export/bin/fixtures/tests. 기존
  `providers/kis.rs`, `kis-client`, V1 files는 원칙적으로 read-only이며 변경 필요 시 중단한다.
- **Required contract:** exact 6-digit input canonicalization; no arbitrary host/path/TR; precomputed
  windows/GET ceiling; one active capture per endpoint/TR; response/schema/rt_cd/target/continuation
  fail-closed; raw bytes commit after validation; identity binds owner-neutral instrument, range,
  entitlement, contract and commit; materializer accepts only exact Raw evidence; coverage와 typed
  failure only; no provider prose/secret; idempotent same-generation replay.
- **Prohibited adjacent work:** DB/API/job queue, factor score, Web, Compose, static approval registry,
  production network call, account/order identifiers.
- **Inputs/dependencies:** WP-1 typed generation/policy contract, existing V1 fixtures and generic KIS
  fetcher. DB access는 adapter trait 밖에서 금지한다.
- **Deliverable/verification:** fixture-driven single-instrument Raw/artifact engine and CLI/library seam;
  malformed/tampered/missing/duplicate/stale/continuation/request-budget/retry tests; same input same
  pins. `cargo test -p market-data --all-targets --no-fail-fast`,
  `cargo test -p collectors --all-targets --no-fail-fast`, fmt/clippy/diff check.
- **Required report:** 공통 형식 전부와 실행하지 않은 provider 검증을 명시한다. 없으면 `none`.

### WP-3 — dynamic factor snapshot

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** intermediate; 기존 factor가 일반 벡터 계산을 제공하고 수치 계약이
  알려져 있다. confidence high. 비교 가능한 as-of를 만들 수 없으면 hard로 재분류한다.
- **Objective:** active+admitted generation 집합을 받아 eligibility와 결정적 cross-sectional
  signal snapshot을 생성한다.
- **Known facts:** 120-session return에는 121 observations가 필요하다. unadjusted vendor snapshot,
  strict PIT 아님, condition은 확률이나 매수·매도 신호가 아니다. 추가 준비 중인 종목은 현재
  snapshot을 바꾸지 않는다.
- **Owned scope:** 새 `crates/factor-engine/src/owner_equity_v2.rs`, lib export, focused tests만.
- **Required contract:** snapshot `as_of`에 관측이 있고 121개 이상인 admitted active instrument만
  eligible; stale/insufficient reason은 typed; input order와 무관한 universe hash; 동점은 canonical
  ID; exact active-ready set으로 ranks 1..N; same inputs same canonical bytes/hash; snapshot rows는
  generation pins를 포함한다.
- **Prohibited adjacent work:** V1 factor 변경, market-data artifact writer, DB/API/Web, corporate
  action adjustment, 목표가/확률/비중.
- **Inputs/dependencies:** WP-1 domain/policy, WP-2 verified generation reader DTO.
- **Deliverable/verification:** pure factor/snapshot candidate. 1/2/31/100 instruments, shuffled input,
  tie, stale, insufficient history, add/remove rerank, numeric bounds와 golden hash tests;
  factor-engine all-targets/fmt/clippy.
- **Required report:** 공통 형식 전부. 수치 비교 가능성 또는 미검증 fixture가 없으면 `none`.

### WP-4 — queue·API orchestration

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** hard; HTTP transaction, jobs claim, filesystem evidence와 DB publication이
  하나의 exactly-once 사용자 경험을 만들어야 한다. confidence high. stale/replay mismatch를
  fail-closed하지 못하면 다음 wave를 막는다.
- **Objective:** Owner mutation을 durable membership/generation/job에 원자 연결하고 WP-2/3를
  실행해 상태와 snapshot을 publication하는 V2 worker/API를 만든다.
- **Known facts:** 기존 owner-beta API는 sealed V1 read path다. 기존 jobs table, idempotency,
  claim/recovery/audit/RLS patterns를 재사용하되 새 job type과 runner를 별도로 둔다.
- **Owned scope:** 새 `crates/job-queue/src/owner_equity_v2/**`와 exports/runner tests;
  새 API repo/http module, 필요한 `http/mod.rs`, `state.rs`, `runtime.rs`, `contract.rs`, DTO/repo
  exports, `apps/api-server/scripts/openapi-spec.mjs`와 focused tests.
- **Required API:** Owner-only list/status GET; add POST 202; retry POST 202; disable DELETE 또는
  POST transition; typed lifecycle/coverage/error DTO. mutation은 CSRF와 idempotency를 요구한다.
  detail/screen/latest read는 V2 latest admitted snapshot을 사용하되 V1 endpoint behavior는
  유지한다. Web max-count와 무관한 server policy metadata를 반환한다.
- **Required worker behavior:** one generation/job per idempotent request; validation/backfill/
  materialize state CAS; crash 후 bounded retry; permanent vs retryable typed failure; admission과
  snapshot pointer를 transactionally publish; disable 중인 stale worker는 publish 금지; add/remove
  concurrency에서 last valid snapshot 보존; daily incremental job도 동일 generation discipline.
- **Prohibited adjacent work:** Web/Compose/ops, V1 approval parser/registry, new provider endpoint,
  production call, account/order/live routes.
- **Inputs/dependencies:** WP-1 schema/domain, WP-2 engine, WP-3 snapshot generator.
- **Deliverable/verification:** API/job lifecycle with fake provider/artifact adapters. auth/Member 403,
  CSRF, RLS, duplicate/idempotency mismatch, capacity, retry, crash-before/after-admission,
  disable-vs-publish race, snapshot atomicity, V1 regression, OpenAPI parity tests; api-server/job-queue
  all-targets/fmt/clippy.
- **Required report:** 공통 형식 전부와 DB integration 실행 여부. 누락은 `none` 또는 명시적 blocker.

### WP-5 — Web management UX

- **Target:** `/data/worktrees/3puw275b/hungry-zebra/apps/web`
- **Initial classification:** intermediate; 안정된 API 위 UI이며 기계적 검증이 가능하다.
  confidence high. Next.js local docs와 기존 auth recovery가 충돌하면 terra로 상향한다.
- **Objective:** `/stock-beta`에서 정확한 코드 추가, 상태/coverage 확인, 실패 재시도, soft disable,
  READY 동적 rank/detail을 한 화면 흐름으로 제공한다.
- **Known facts:** 기존 table rendering은 동적이지만 Zod `max(30)`, rank max 30, 고정 30 copy와
  fixtures가 있다. 세션 만료는 로그인 redirect로 복구해야 한다.
- **Owned scope:** stock-beta pages/components, equity-signals contracts/client, stock-beta i18n,
  related unit/surface/E2E fixtures/tests. 작업 전 설치된 Next.js docs의 form/mutation/cache/session
  관련 문서를 읽는다.
- **Required UX:** 6-digit validation; submit 중 중복 방지; 서버 policy capacity 표시; 상태 badge와
  observed/target coverage; polling/refetch while nonterminal; typed failure만 표시; retry; disable
  confirmation; READY 이후 자동 list/rank refresh; empty/100-row/responsive/keyboard/screen-reader;
  unadjusted/owner-only/non-PIT warning 유지.
- **Prohibited adjacent work:** backend/OpenAPI/DB/worker/ops, 종목명 자동검색, arbitrary URL,
  투자추천 표현, Member navigation 노출.
- **Inputs/dependencies:** WP-4 committed API/OpenAPI contract and test fixture shapes.
- **Deliverable/verification:** production UI and provider-free fixtures. `npm --prefix apps/web run
  typecheck`, `lint`, `test`, `build`, focused Playwright; expired-session redirect, Owner/Member role,
  31/100 rows, add-to-ready, retry/disable tests.
- **Required report:** 공통 형식 전부와 읽은 Next.js 문서 경로. 미확인은 `none` 또는 명시.

### WP-6 — runtime·ops integration

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** intermediate; 기존 installed-current-release 패턴을 적용하지만
  provider/secret/runtime 안전성 때문에 confidence medium이다. 정적 증명이 안 되면 hard로
  재분류하고 실제 실행하지 않는다.
- **Objective:** V2 worker와 provider-free verifier를 production images/Compose에 연결하고,
  request budget/concurrency/rollback을 운영자가 검증할 수 있게 한다.
- **Known facts:** 기존 V1 one-shot wrappers와 registry mounts는 보존한다. production image는
  exact revision, 순차 build, immutable installer를 사용한다. parallel production build는 과거
  OOM을 일으켰다.
- **Owned scope:** 필요한 Rust/API/collector Dockerfile additions, `deploy/compose/compose.yml`,
  release/static/self-test scripts, 새 `docs/runbooks/owner-managed-equity-universe-v2.md`.
- **Required runtime:** queue worker만 provider credentials와 Raw/artifact RW를 받고 API/Web은 받지
  않는다; materialize verifier는 network none; endpoint/TR concurrency 1; active/backfill/daily
  request ceilings와 disk estimate preflight; secrets/body 미출력; installed exact image revision;
  shutdown/restart 후 claimed job recovery; V1 profiles/mounts unchanged.
- **Prohibited adjacent work:** application domain/API/Web 수정, V1 wrapper 삭제, credentials 읽기나
  출력, production provider 호출, build/install/deploy without explicit coordinator authorization.
- **Inputs/dependencies:** WP-2 binaries, WP-4 runner/config contract, existing release installer/static
  patterns.
- **Deliverable/verification:** image/Compose wiring, fake runtime self-test, operator runbook.
  `docker compose config --quiet` where available, ops static/self-tests, build-image static check,
  network/mount/env scans, `git diff --check`.
- **Required report:** 공통 형식 전부; Docker daemon/production 검증 미실행을 숨기지 않는다.

### WP-7 — independent review·QA

- **Target:** `/data/worktrees/3puw275b/hungry-zebra`
- **Initial classification:** intermediate code review; confidence high. correctness conclusion is the
  deliverable, so repository rule에 따라 terra high를 사용한다.
- **Objective:** WP-1~6 결과가 본 계획의 사용자 흐름, 데이터 증거, 보안, V1 회귀와 운영 경계를
  실제로 충족하는지 독립 판정한다.
- **Owned scope:** read-only. 어떤 파일도 수정하지 않는다. 발견된 문제는 severity와 재현 절차로
  coordinator에게 반환한다.
- **Review checklist:** single source of truth; no fixed 30/list duplication in V2; no manual per-symbol
  rebuild; exact owner RLS/CSRF/idempotency; illegal state transition/stale publish 차단; per-symbol
  capture only; 1 rps/allowlist/no account/order; immutable pins/tamper; 121-observation eligibility;
  add/remove deterministic rerank; dynamic Web schemas; session recovery; V1 byte/behavior regression;
  secret/provider-body scans.
- **Prohibited adjacent work:** 자동 수정, production network/deploy, missing requirement 추측.
- **Inputs/dependencies:** 통합된 WP-1~6 diff와 worker reports.
- **Deliverable/verification:** severity순 findings, file/line evidence, 실행한 full/focused tests,
  unverified surfaces, `ACCEPT` 또는 `REJECT`. Rust workspace tests, migration contract with disposable
  DB when available, Web type/lint/unit/build/E2E, ops/static checks를 실행한다.
- **Required report:** 변경 파일은 `none`; deviations, tests/results, unresolved/follow-up,
  not-found/not-verified를 모두 명시한다.

## Coordinator gates

1. **Pre-launch:** `$paseo-delegate` availability를 확인하고 모든 WP를 그 스킬로만 실행한다.
   현재 worktree의 사용자 변경과 untracked 문서를 기록·보존한다. V1 exact artifact/registry/API
   regression baseline, migration next number 0053, KIS allowlist, Owner-only rights, 권장 100 active,
   261 target/121 minimum, broad KRX instrument labeling을 확인한다. 다른 instrument-type claim이
   필요하면 WP-1 전에 멈추고 source 계약을 별도 계획한다.
2. **Wave 1 gate:** coordinator가 ADR, 상태 전이표, DB constraints/RLS/down migration과 migration
   test를 직접 열어 확인한다. admission pins 또는 snapshot lineage가 application convention에만
   의존하면 반려한다.
3. **Wave 2 gate:** provider fixture에서 exact allowed method/path/TR, blank continuation, planned
   GET ceiling, 1 rps serialization, Raw visibility after validation, secret/body 비노출과 same-input
   idempotency를 직접 확인한다. provider 변경 요구가 나오면 실행 branch를 멈춘다.
4. **Wave 3 gate:** 1/31/100 instruments, add/remove, shuffled/tied inputs, insufficient/stale instrument
   tests에서 deterministic universe/snapshot hash와 ranks를 재검산한다.
5. **Wave 4 gate:** disposable DB에서 add transaction, duplicate/mismatched idempotency, worker crash,
   retry, disable race, RLS/Member denial, admission+snapshot atomic publication을 검증한다. V1 API
   regression과 OpenAPI parity가 통과해야 Wave 5로 간다.
6. **Wave 5 gate:** WP-5와 WP-6의 disjoint diff를 통합하고 Web 31/100-row parsing, login recovery,
   fake add-to-READY와 Compose credential/mount/network boundary를 확인한다. production 호출·배포는
   하지 않는다.
7. **Wave 6 gate:** WP-7이 `ACCEPT`하고 high/medium 미해결 finding이 없어야 한다. finding이 있으면
   해당 owner scope만 갖는 새 remediation WP를 같은 형식으로 계획하고 `$paseo-delegate`로
   실행한 뒤 독립 review를 반복한다.
8. **Final provider-free acceptance:** `cargo fmt`, affected Rust all-targets/workspace tests,
   migration contract, Web type/lint/unit/build/Playwright, OpenAPI/static/Compose checks,
   `git diff --check`, order/account/secret scans가 모두 통과하고 V1 fixtures/hash/behavior가
   유지되는지 coordinator가 직접 확인한다. `docs/STATUS.md`에는 실제로 검증된 상태만 기록한다.
9. **Optional production rollout — 별도 승인 필요:** clean commit과 원격 동기화 후 production
   images를 순차 빌드하고 immutable installer로 exact release를 설치한다. provider preflight가
   하루/종목/request/disk ceiling을 출력 없이 확인한 뒤, 사용자가 지정한 실제 종목 한 개를
   Owner Web에서 추가해 `REQUESTED -> READY`, coverage, dynamic rerank, restart 0/OOM false,
   severe log 0을 직접 QA한다. 실패하면 last-known-good snapshot/release를 유지하고 typed 상태만
   노출한다. Paper/Live/account/order는 끝까지 비활성이다.
