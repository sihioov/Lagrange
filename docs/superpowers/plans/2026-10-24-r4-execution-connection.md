Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R4-05 전용 실행·저장·조회 연결

## Goal and boundaries

작업 경로: `/data/worktrees/3puw275b/short-bobcat`, 기준 `79cbab6ad06733468b6408fdab97c56d4e40cb6d`.
Owner는 R4-04 계산 검증 종료와 R4-05 사용자 격리 의존 설명 뒤 “이어서 작업 진행해”라고 지시했다.
이번 범위는 기존 승인 price-only 시뮬레이션의 전용 작업 큐·owner-scoped 저장·인증 API·Web 연결과
사용자 격리 검증이다. 구현 방식은 기존 접점을 조사하여 G1에서 고정한다. 같은 범위의 승인을 다시 묻지 않는다.

완료 기준:

- 인증 actor가 소유하는 전용 작업 생성·조회와 기존 패턴에 맞는 취소/실패 상태를 제공한다.
  actor, 승인 입력, engine provenance를 클라이언트가 임의로 대체할 수 없어야 한다.
- 실제 계산 엔진과 큐 실행이 연결되고 두 비용 프로필이 모두 검증된 뒤에만 성공 결과를 원자적으로 저장한다.
  실행 중간·실패·부분 pair는 성공 비교 결과로 조회되지 않는다. lease/취소/재시도 경계는 기존 모델과 일치한다.
- 사용자 간 목록·ID/hash 조회/재사용을 거절하고 같은 내용 hash에도 소유권을 별도로 유지한다.
  DB RLS와 API 인증을 실제 PostgreSQL·HTTP 통합 검증으로 입증한다. UUID/hash/OWNER_ONLY label로 대신하지 않는다.
- Web은 전용 계약의 상태와 허용 지표·두 비용 프로필·입력 제한을 표시한다. 임의 새 지표나 전략을 추가하지 않는다.
- 독립 리뷰, 필요한 회귀, 최종 동일 소스 후보의 기존 CI/required와 Research smoke를 각1회 검증한다.
  구조 변화에 맞춰 두 아키텍처 도표의 해당 관계·file:line 근거와 로컬 PNG를 함께 갱신한다.
- R4-04 사용자 격리 차단은 해당 증거가 확인된 범위에서만 해소한다. 구현·격리 검증·운영 배포·Owner 인수는 구분한다.

고정 계약 SHA-256은 `8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd`다.
기존 승인 v3 입력, 첫 전략, OWNER_ONLY/PRICE_RETURN_ONLY/vendor_snapshot=true/strict_pit=false,
두 비용 프로필과 지표 규칙은 변경하지 않는다. 70998fd의 CI·smoke·실제 입력 재현 근거는 보존하되
변경 후보의 실행 검증으로 자동 재사용하지 않는다.

제외: R4-06 전략 확대·출시, 운영 migration 적용/배포/서비스 재시작·수리, provider 요청,
입력 업로드·복사·재수집, Daily 활성화, 계좌·거래, R2/R3 유예 작업. 기존 일반 백테스트와
price-only 거절 경계를 우회하지 않는다. 새 제품 연결은 개발·격리 검증이며 운영 활성화가 아니다.

적용 지침: 사용자 제공/저장소 AGENTS.md, Web AGENTS.md/CLAUDE.md, ROADMAP,
development-verification runbook, 고정 simulation 계약, $paseo-delegate-plan/$paseo-delegate.
Web 변경 전 설치된 Next.js의 관련 로컬 guide를 읽는다. 로컬 heavy Cargo/Web/Docker build는 하지 않는다.
현재 Paseo profile은 coordinator Astra/high만 제공하므로 leaf 모델은 AGENTS 규칙과 확인한 가용 모델에서 선택한다.
관련 memory context는 MCP_ERROR로 실패했으며 저장소 계약/근거로 계속한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | intermediate | 기존 DB/RLS·typed queue·runner 접점의 유한 소스 감사 | high | 기존 역할 모델과 요구 소유권 충돌, provenance 경로 부재 |
| WP-2 | intermediate | 기존 인증/API/OpenAPI/Web 전용 진입점의 유한 감사 | high | 고정 계약에 없는 사용자 선택이나 새로운 권한 의미 필요 |
| WP-3A | intermediate | 폐쇄형 입력·whole-pair 검증과 좁은 공유 helper | high | 기존 계산 계약 변경이 필요하거나 같은 원인2회 실패 |
| WP-3B | intermediate | 확정 schema의 RLS·원자 게시·복구와 실제 DB 회귀 | medium | DB 역할/잠금 모델과 충돌 또는 같은 원인2회 실패 |
| WP-3C | intermediate | 같은 실행 파일의 bounded child·provenance·lease lifecycle | medium | child 종료/출처 증명 불가 또는 같은 원인2회 실패 |
| WP-3D | intermediate | source-only 전달물과 실제 DB/HTTP test inventory 연결 | high | 기존 required 보존 불가 또는 전달물 범위 충돌 |
| WP-4 | intermediate | G1 인터페이스의 API/auth/OpenAPI 연결 | medium | 기존 actor/RLS transaction과 모순 또는 범위 밖 데이터 요구 |
| WP-5 | intermediate | 확정 API의 전용 Web 화면·상태·회귀 | medium | 기존 UI/Next contract로 해결 불가 또는 검증 반복 실패 |
| WP-6 | intermediate | owner isolation·atomic publication·회귀의 독립 리뷰 | high | 실제 보안 결함/모델 모순은 해당 분기만 재계획 |
| WP-7 | simple | 고정 후보 hosted run과 필수 증거 회수 | high | 환경 실패나 증거 누락은 원인별 판단; 맹목 재실행 금지 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | intermediate | 저장·큐·runner 접점과 구현 경계 감사 | private `target/r4-connection/wp-1/`만 | G0 | gpt-6.1-sol/high; 결론이 산출물인 소스 분석 | file:line 근거, 최소 변경 목록·타입·검증 제안 | coordinator 직접 소스 대조 |
| WP-2 | 1 | intermediate | API·Web 접점과 계약 감사 | private `target/r4-connection/wp-2/`만 | G0 | gpt-6.1-sol/high; 결론이 산출물인 소스 분석 | route/DTO/화면·권한·회귀 대응표 | coordinator 직접 소스 대조 |
| WP-3A | 2 | intermediate | 입력/whole-pair 경계 구현 | G1-2의 정확 Rust 파일만 | WP-1/2,G1 | gpt-6-luna/max | 검증된 타입·공개 signature·private 보고 | rust fast + hosted unit |
| WP-3B | 3 | intermediate | RLS·원자 게시·실패/복구 | G1-3의 migration/publish/recovery/DB test | WP-3A 수용 | gpt-6-luna/max | schema·publisher·실제 DB 회귀 | db/rust fast + hosted PG |
| WP-3C | 4 | intermediate | 전용 process runner·provenance | G1-4의 child/runner/binary | WP-3A/B 공개 signature 수용 | gpt-6-luna/max | bounded 실행기·lifecycle 회귀 | rust fast + hosted process |
| WP-3D | 5 | intermediate | hosted build/DB harness 연결 | G1-5의 정확 workflow/script/harness 파일 | WP-3C 및 WP-4 test target 수용 | gpt-6-luna/max | source-only 전달물·필수 named-test gate | script fast + hosted |
| WP-4 | 4 | intermediate | 전용 인증 API·OpenAPI 연결 | G1에서 api-server·OpenAPI 정확 파일 목록 확정; WP-3 쓰기와 비중복 | G1 및 WP-3 공개 인터페이스 동결 | gpt-6-luna/max | 구현·API 회귀·private 보고 | fast + hosted HTTP/DB/OpenAPI |
| WP-5 | 5 | intermediate | 전용 Web 결과 상태/비교 표시 | G1에서 apps/web·관련 generated client 소유 확정 | WP-4 wire 동결 | gpt-6-luna/max | UI·unit/E2E 회귀·private 보고 | Web cwd lint + hosted Web |
| WP-6 | 6 | intermediate | 독립 구현·격리 리뷰 | private `target/r4-connection/wp-6/`만 | 구현 통합,G2 | gpt-6.1-sol/high | 필수 수정/수용 근거 | coordinator diff/테스트 대조 |
| WP-7 | 7 | simple | 최종 동일 후보 CI·smoke 증거 회수 | private `target/r4-connection/wp-7/`만 | G3 후보 고정 | gpt-6-luna/medium | 정확 run/SHA/attempt/job/정리 증거 | coordinator 독립 검증 |

G1 전에는 WP-1/2 읽기 전용 감사만 실행한다. 구현 파일과 서버 내부 인터페이스는 보고서 및
코디네이터 판단으로 이 문서에 확정한 뒤 해당 worker를 시작한다. 저신뢰 미확정 설계를 worker가 추측하지 않는다.
코디네이터가 계획·로드맵·작은 통합 보정·도표 통합·최종 수용을 담당한다. 큰 추가 구현은 별도 scoped WP로 기록한다.

## Worker briefs

### WP-1 — 저장·실행 경계 감사

위 절대 작업 경로, intermediate/high. ROADMAP, 본 계획, 고정 계약의 P0-12와
R4-04 검증 기록을 읽는다. `crates/job-queue/src/owner_beta/simulation/`, 기존 owner_beta
runner/publish/recovery, typed queue/claim/worker filter, migration·DB role/RLS 및 integration harness를
실제 참조를 따라 조사한다. 기존 simulation은 순수 함수이며 저장·queue 경로가 없다는 기준을 검증한다.

전용 job type, 소유권 저장, pair 원자 저장과 terminal status, lease/cancel/retry, DB 역할 분리,
실제 engine provenance·approved artifact 접근의 재사용 가능한 기존 패턴과 공백을 file:line으로 제시한다.
기존 일반 backtest/추천/daily 작업에 새 type이 흘러가지 않는 경계도 확인한다. 최소 migration/타입/함수
인터페이스 및 실제 PostgreSQL 격리·부분 실패 회귀안을 제시하되 schema나 코드는 수정하지 않는다.
API/Web 쓰기는 WP-2/G1 이후이며 고정 계산 규칙·운영 구성·artifact를 바꾸거나 읽지 않는다.

출력은 `target/r4-connection/wp-1/report.md`와 필요한 소스 hash receipt(0700/0600)뿐이다.
추측·누락을 메꾸지 말고 충돌/미확인 경계를 명시한다. no Cargo/Docker/provider/DB/network 실행.

### WP-2 — API·Web 계약 감사

동일 절대 경로, intermediate/high. 동일 계획/고정 계약/R4-04 기록과 Web AGENTS/CLAUDE를 읽는다.
`crates/api-server/src/http/owner_beta*`, backtests, middleware/session/state, repos/owner_beta 및
관련 HTTP tests, contract/OpenAPI, `apps/web`의 Owner price-only/추천/백테스트 화면과 data layer를
실제 참조를 따라 조사한다. auth/CSRF/idempotency/actor transaction·기능 gate를 구분한다.

전용 enqueue/list/get/cancel 및 pair 조회의 최소 route/DTO·실패코드·Web 상태/표시 계약을 제안한다.
user-supplied owner/hash/provenance/경로를 권한으로 삼지 않는 경계, 다른 사용자 목록/ID/hash
조회·재사용 거절과 부분 결과 비노출 테스트를 제시한다. 기존 일반 backtest 거절을 우회하지 않는다.
새 지표·전략·범용 설정을 발명하지 않는다. 기존 설치 Next docs의 관련 경로를 찾아 구현 brief에 남긴다.

출력은 `target/r4-connection/wp-2/report.md`와 필요한 소스 hash receipt(0700/0600)뿐이다.
소스/문서/DB/운영 변경, npm/Cargo/Docker build, 브라우저/서비스 실행 및 provider 호출은 하지 않는다.

### WP-3~5 — G1 이후 확정 구현

위 표의 의존성이 충족되고 코디네이터가 정확 파일·타입·요청/응답 계약·회귀 기대값을 이 문서에
추가한 뒤 시작한다. 자기 소유 파일의 작은 누락/오류는 직접 고치고 관련 검사만 재개한다.
미확정 설계·명세 모순은 보고하며 scope 밖 리팩터링·운영 활성화는 하지 않는다.

### WP-6 — 독립 리뷰

완성 diff를 고정 계약·RLS·auth·pair publication·실제 worker provenance·테스트와 대조한다.
private 보고만 쓰고 코드를 고치지 않는다. 소스 리뷰와 hosted/runtime 검증을 구분하며
필수 수정이 없다는 판단에는 file:line 근거와 미검증 항목을 포함한다.

### WP-7 — hosted 증거 회수

coordinator가 확정한 candidate/branch/run만 읽기 전용으로 관찰한다. 기존 모든 required job과
집계, Research smoke의 기능·정리, 새 RLS/API/Web regression의 실제 실행 여부를 회수한다.
각 workflow dispatch는 coordinator가1회 수행한다. retry/rerun/cancel, provider 데이터 업로드,
cache 성공의 성능 주장 금지. 실제 runtime이 별도 필요하면 G3에서 정확 후보·입력·횟수를 제한한다.

모든 worker의 필수 보고: 변경 파일/행 범위, 명세와 다른 처리·이유, 검사 명령/결과,
미해결/후속 작업, 찾지 못하거나 검증하지 못한 것(없으면 명시적으로 없음).
Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

## Coordinator gates

1. G0: clean79cbab6, R4-04 종료/감시 삭제, 기존 계약·승인, 지침/profile/model 및 새 소유 ledger 확인.
2. G1: 두 감사의 실제 idle/pending0와 보고 회수, 직접 소스 대조. 파일 소유·DB/runner/API wire·검증 기대값을 확정한다.
3. G2: 구현 diff/fast/상호 인터페이스·도표 근거를 확인하고 독립 리뷰를 실행한다.
4. G3: 필수 수정 해소, source candidate/hash 고정, 기존 CI/required+smoke 각각1회 실행.
   실제 입력 runtime이 필요하면 기존 읽기 전용 격리 패턴과 자원 한계만 사용한다. local heavy compile 금지.
5. G4: 같은 후보의 실제 DB RLS/HTTP/Web·전체 CI/smoke와 필요한 실행/정리 증거 확인.
   R4-05 구현/검증과 R4-04 격리 상태·남은 운영/Owner 인수를 정확히 기록한다.
   모든 소유 turn·pending 요청·실행이 끝나고 보고 회수된 뒤 현재 감시만 삭제한다.

## Progress

2026-10-24: 새 R4-05 범위 착수. G0 기준은79cbab6이며 R4-04 계산 검증과 R4-05 격리 의존을 보존한다.
WP-1/2 감사부터 시작한다. 구현·새 hosted/runtime 검증은 아직 실행하지 않았다.

WP-1 `e441c98a-0e64-409e-89a1-3d331b764b8c`, WP-2 `d804689c-98d5-4b91-a2bf-91b665c3473e`를
각각 codex/gpt-6.1-sol/high로 시작했다. 두 worker의 workspace·parent를 확인했고 종료/권한 알림을
등록했다. WP-1은 mode를 지정하지 않아 daemon의 full-access 기본값을 상속했고 별도 mode 변경은
요청하지 않았다. WP-2는 생성 시 auto를 지정했다. 양쪽 모두 보고서만 쓰는 범위는 동일하다.
현재 작업 전용 감시 `5d393a6c`는 5분 주기·최대24회·2026-10-24 15:39 UTC 만료로 등록했다.
소유 상태/정확 요청/보고 회수는 `target/r4-connection/owned-workers.json`으로 관리한다.

## G1 — 2026-10-24 감사 수용과 구현 계약

WP-1/2의 실제 idle·pending0·보고 회수를 확인했다. 보고서 해시는 각각
`24796dd87083ee186de5392d10468672558563a2d5c52a4a85f84304e072b66f`,
`265b004336e36f20c693310cc86da407b7c92211b3eb1b4ceb79f4fb41aff681`이다.
코디네이터가 저장소 입력59/71개와 WP-2 외부 참조5개 해시를 재검증했다.
이는 source-only 감사 수용이며 구현/실행/사용자 격리 수용이 아니다.

직접 확인한 결정 근거:

- `crates/api-server/src/http/state.rs:398`의 일반 backtest 조회는 admin pool이다.
  새 경로는 app pool과 `actor_tx.rs:34`의 transaction-local actor를 사용한다.
- `apps/web/lib/api/contracts.ts:17`의 기존 disabled gate는 일반 제품을 허용한다.
  새 페이지/handler는 Owner와 정확한 owner_only mode를 모두 요구한다.
- `crates/job-queue/src/owner_beta/runner.rs:214`의 시작된 blocking 계산 abort를 복제하지 않는다.
  순수 계산은 유지하고 별도 소유 child를 종료·회수한다.
- `crates/job-queue/src/queue.rs:267,802,826`의 untyped claim과 두 sweep 지점에는 새 type 제외가 필요하다.
- 같은 파일 `:561`의 취소는 state commit 뒤 별도 audit insert다. 오류가 무변경을 뜻하지 않는다.
- `migrations/0035_recommendation_entitlement_lock.up.sql:8`은 recommendation/단일 날짜이며 새 실행의 권리 증명이 아니다.
  `crates/auth/src/entitlement/use_registry.rs`의 기존 Backtest 사용과 현재 entitlement 정책을 적용한다.

### G1-1. 공통 결정

1. 새 job type은 `owner_beta_price_only_simulation`, payload schema는
   `owner-beta-price-only-simulation-job-v1`, enqueue key namespace는
   `owner-beta:price-only-simulation:v1:`로 고정한다. 기존 generic/추천/Daily 경로에는 연결하지 않는다.
2. 별도 `owner_beta_simulation_runs`와 `owner_beta_simulation_result_pairs`를 사용한다.
   pair는 한 행의 두 NOT NULL canonical bytea+hash이며, 모두 검증된 뒤 job/run/attempt 성공과 같은
   transaction에서 게시한다. owner는 계산 hash 밖에 있고 두 사용자에게 같은 hash가 생겨도 별도 소유권이다.
   global hash UNIQUE/dedup, hash-only lookup, 외부 결과 import/reuse endpoint는 만들지 않는다.
3. app은 자신의 metadata와 완전한 성공 pair만 조회한다. worker는 기존 trusted writer 역할을 사용하며
   이 역할 자체의 daemon 간 격리는 주장하지 않는다. admin에는 새 canonical bytes 권한을 주지 않는다.
   migration_owner/FORCE RLS, 다른 serving role/PUBLIC의 최소 권한과 직접 SQL 거절을 검증한다.
4. 계산 결과 wire는 기존 canonical SimulationResult 전체를 보존한다. 새 API envelope만 덧붙인다.
   원본 bytea·hash를 재검증한 뒤 직렬화하며, 새 지표/전략/소수 규칙/계산식은 만들지 않는다.
5. 취소는 인증된 소유권+CSRF+정확 intent key를 요구하되 entitlement 회수 후에도 자신의 작업을 중지할 수 있다.
   다른 payload 조회 권한은 이 예외로 확대하지 않는다. 기존 actor-specific queue pool의 audited cancel을
   run/job/type 재확인 후 사용한다. run의 terminal mirror는 전용 recovery가 처리하며 GET은 job의 CANCELED를
   즉시 반영한다. `{status:"CANCEL_REQUESTED"}`는 접수 응답뿐이고 새 저장 상태가 아니다.
   audit 실패/commit 불명은 정적 오류로 알리고 GET으로 실제 상태를 확인한다. 성공 pair를 삭제하지 않는다.
6. 입력/결과 접근에는 기존 `KrUse::Backtest`를 사용한다. DevBacktest 우회나 새 entitlement 생성은 하지 않는다.
   admission 및 읽기는 fresh service를 한 번 로드하여 해당 schedule의 모든 warmup/evaluation 세션을 검사한다.
   list는 현재 Backtest gate와 각 반환 row의 범위를 검사한다. foreign/missing ID는 actor header 조회 단계에서
   동일404이며 권리 검사로 존재를 노출하지 않는다. publication은 동일 session 배열의 권리를 DB에서 잠금 재검증한다.
   양 끝 날짜만 검사하여 중간 공백을 허용하지 않는다. 여러 ACTIVE 기록의 합집합이 모든 세션을 덮는 것은 허용한다.
   기존 권리 행을 수정하지 않고 isolated fixture만 seed한다. 승인 입력의 재수집/복사/업로드는 하지 않는다.
7. 전용 binary `owner-beta-simulation-runner`의 같은 실행 파일을 parent/child로 사용한다.
   child는 실제 executable hash와 compile-time source/toolchain/target/profile을 확인하고 env_clear로 실행한다.
   parent/child 출처는 coordinator가 exact CI run/attempt/SHA와 다운로드 digest를 확인한 source-only 전달물에 묶는다.
   API code_commit, 실행 시 선언한 문자열, 임의 JSON 영수증만으로 출처를 인정하지 않는다.
8. 운영 상한은 동시 child1, 계산 deadline1200초, TERM grace5초 뒤 KILL/reap, total stdout64MiB,
   request256KiB, stderr 비저장으로 고정한다. 종료 회수 전 slot을 반환하지 않는다. lifecycle/검증 중 heartbeat도 유지한다.
   격리 실제 실행의 compute 한계는 메모리512MiB·swap동일·CPU1·PID32·nonroot·network-none·read-only root다.
   이는 상한이며 처리 시간/성능 보장이 아니다. timeout/잘못된 결과/입력/provenance는 terminal;
   분류된 infrastructure 실패만 최대3 attempts의 기존 queue 정책으로 처리한다. workflow 재시도 허가는 아니다.

### G1-2. WP-3A — 공유 입력과 pair 경계 (먼저 실행)

정확 쓰기 소유:

- 새 `crates/job-queue/src/owner_beta/simulation_execution/{mod.rs,input.rs,pair.rs}`
- `crates/job-queue/src/owner_beta/mod.rs`의 새 module export만
- `crates/job-queue/src/owner_beta/simulation/{mod.rs,input.rs,contract.rs}`의 아래 좁은 helper/getter만
- 새 `crates/job-queue/tests/owner_beta_simulation_execution_contract.rs`
- private `target/r4-connection/wp-3a/` 보고서/소스 해시

공개 인터페이스:

```rust
prepare_approved_simulation_window(
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    start: Option<TradingDate>, end: Option<TradingDate>
) -> Result<SimulationSchedule, SimulationInputError>;
SimulationJobInput::from_approved_artifact(
    run_id: Uuid, strategy_config_id: Uuid,
    artifact: &ApprovedHistoricalPriceOnlyArtifact,
    start: Option<TradingDate>, end: Option<TradingDate>,
    resolved: &ResolvedConfig
) -> Result<SimulationJobInput, SimulationExecutionError>;
SimulationJobInput::from_json(bytes: &[u8]) -> Result<Self, SimulationExecutionError>;
CheckedSimulationPair::from_canonical(
    illustrative: &[u8], zero_cost: &[u8],
    input: &SimulationJobInput, expected_engine: &EngineIdentity
) -> Result<Self, SimulationExecutionError>;
```

입력의 private 필드는 schema/run_id/strategy_config_id/resolved evaluation_start/end/five pins/
contract_document_sha256/bars_sha256/기존 OwnerBetaStrategySnapshot이다. 소유권/engine/root는 payload에 없다.
Serialize 및 validated Deserialize를 제공하고 `from_json`은 기존 strict parser를 재사용하여 중복 key도 거절한다.
이를 위해 simulation/mod.rs에서 기존 canonical parser/serializer를 `pub(crate) use`하는 것만 허용한다.
모든 factory/deserialize에서 exact fixed pins/contract/bars, 고정 trend50/200+069500 config/hash,
승인 XKRX calendar의 warmup200·evaluation최소2·전체창을 검증한다. UUID는 hash에 넣지 않는다.
검증된 입력에도 실행 시 artifact 재승인이 필요함을 문서화한다.

window helper는 기존 approve_and_prepare의 metadata/bars/calendar/schedule 검사를 추출하여 양 경로가 공유한다.
파일을 다시 읽거나 계산 규칙을 바꾸지 않는다. result의 engine/input/strategy/conventions 및
strategy snapshot hash를 읽는 getter와 기존 승인 상수의 crate visibility만 필요 범위에서 추가할 수 있다.
canonical.rs, engine.rs, signals.rs, ledger.rs, metrics.rs의 로직은 수정하지 않는다.

CheckedSimulationPair는 private fields이고 외부 Deserialize/unchecked constructor가 없다.
각 body를 production SimulationResult parser로 검증하고 canonical 재직렬화와 원본 bytes가 같아야 한다.
두 cost slot, 동일 engine/contract/input/strategy/schedule/signals, queued dates/pins/strategy hash를 대조한다.
hash는 원본 canonical bytes에서 재계산한다. bytes/result/hash의 read-only accessor를 제공하되 이 타입만으로
실행 파일 attestation을 주장하지 않는다. expected_engine의 실제 증명은 WP-3C 책임이다.

실제 공개 getter 목록/타입과 static error enum을 보고서에 확정해 후속 worker가 그대로 소비하게 한다.
새 unit/integration 계약 회귀: 정상 pair와 원본 byte/hash 보존; unknown/duplicate input; 잘못된 pin/config/window;
한 profile 누락/중복/반대 slot; 다른 engine/schedule/signals/strategy; noncanonical/truncated/tampered bytes;
다른 run/config UUID의 같은 내용 hash. 테스트는 production capital과 실제 validator를 사용하고
합성 fixture임을 명시한다. 실제 approved root/과거 private 결과를 읽지 않는다.
로컬 fmt/rust fast만 수행하며 compile/test/Clippy는 hosted pending으로 보고한다.

### G1-3. WP-3B — 저장·게시·복구

WP-3A public signature를 수용한 뒤 시작한다. 정확 소유는 새 migration
`0063_owner_beta_price_only_simulations.{up,down}.sql`, 새 simulation_execution/{publish,recovery,storage}.rs,
`crates/job-queue/src/queue.rs`의 세 new-type 제외 지점, 새
`crates/job-queue/tests/owner_beta_simulation_execution.rs`, 기존 `tests/queue_contract.rs`,
`tests/integration/migration-contract/tests/migration_contract.rs`다. 공유 mod.rs export는 코디네이터가 순차 통합한다.

runs의 immutable columns는 id/owner_user_id/strategy_config_id/job_id(unique)/strategy_id/version/config_json/config_sha256/
evaluation_start/end/session_count/five pins/contract_document_sha256/bars_sha256/fixed labels다.
state/error_code/created_at/started_at/finished_at/updated_at 및 성공 시 engine identity를 별도 보관한다.
pair columns는 run_id(pk)/owner_user_id/composite FK(run_id,owner)/illustrative_canonical/illustrative_sha256/
zero_cost_canonical/zero_cost_sha256이며 original bytes를 유지한다. 실행 중 runs는 PENDING으로 남아도 되며
effective 상태는 jobs를 기준으로 한다. terminal failure/cancel은 pair가 없어야 한다.

app은 column-limited own PENDING INSERT와 own SELECT만, pair SELECT는 run/job 모두 SUCCEEDED·exacttype·owner일 때만.
worker는 immutable binding 수정 없이 상태 변경 및 whole-pair INSERT만 허용한다. config 잠금은0052의
trigger-only/hardened SECURITY DEFINER 패턴으로 구현하고 callable privilege/config UPDATE를 늘리지 않는다.
row-specific deferred guards는 success-without-pair, pair-under-failure, retagged/mixed-owner/job binding을 차단한다.
단, canceled job+PENDING run의 recovery 전 상태는 허용하며 pair는 계속 보이지 않는다. populated down은 거절한다.
app의 success-only pair 정책을 무결성 trigger의 조회 정책으로 재사용하지 않는다. guard가 미완성/실패 pair를
RLS 때문에 못 보고 없는 것으로 판단해서는 안 된다. migration_owner의 actor-bound 별도 정책과 row-derived
잠금/권한으로 원시 관련 행을 검사하고 direct EXECUTE를 차단한다. 실제 app 취소·worker 게시·migration 역할에서
deferred guard가 동작하는지 테스트한다. populated down도 FORCE RLS로 감춰진 타 owner 행을 놓치지 않아야 한다.
기존0049의 역할별 정책과0050의 잠금/NO FORCE bounded emptiness guard를 참고하되 기존 정책은 수정하지 않는다.

publication/failure/recovery는 queue.pool, job->run 잠금 순서, DB clock/current attempt fence를 공유한다.
권리 helper `lock_owner_beta_simulation_entitlement(owner uuid, required_dates date[])`는 worker-only,
고정 dataset/use, bounded nonempty approved session 배열, 기존 user/권리 row 잠금과 모든 세션 coverage를 확인한다.
PUBLIC/app/admin 직접 EXECUTE는 금지한다. 권리 변경 없이 행의 현재 상태만 확인한다.
queued input·claim·run exact binding과 active frozen config를 검증하고 pair+run+job+attempt를 한 번에 commit한다.
commit acknowledgement 불명은 CommitUnknown으로 보존하고 exact own binding을 재조회하여 중복 삽입을 막는다.
late/stale/canceled attempt는 게시하지 못한다. generic claim과 sweep 양쪽에서 새 type을 제외한다.

WP-1 보고서 §5의 실제 PostgreSQL 행렬을 사용한다. 최소 RLS/role grants/two-owner equal-hash/atomic success/
rollback partial-hidden/typed exclusion/cancel-vs-publish/lease-zombie/retry-exhaustion/read corruption/migration guard를
실제 app/worker/migration 역할로 증명한다. CommitUnknown을 실제 장애로 재현하지 못하면 그 항목은 미검증으로 남긴다.
실제 artifact는 읽지 않고 합성 production-valid pair fixture로 저장 경계를 검증한다.

### G1-4. WP-3C 및 WP-4/5 — 실행과 제품 경로

WP-3C 소유: 새 simulation_execution/{provenance,child,runner}.rs, 새
`crates/job-queue/src/bin/owner-beta-simulation-runner.rs`, 새 `crates/job-queue/tests/owner_beta_simulation_process.rs`.
Cargo.toml은 기존 의존성으로 불가능한 경우에만 coordinator가 근거 확인 후 해당 branch 소유를 확장한다.
소유 child process group·env_clear·bounded stdio·kill+wait/reap·lease/cancel/shutdown·동시1을 구현한다.
root와 trusted build receipt는 operator startup 설정이며 client/payload에 없다. 실제 engine을 한 번 호출하고
양 profile canonical bytes의 closed envelope를 parent에 반환한다. DB credentials는 child에 전달하지 않는다.
계산/결과 검증 중 독립 heartbeat가 유지되어야 하며 started blocking validation을 abort했다고 회수 완료로 보지 않는다.
실제 child fixture PID/cleanup와 malformed/oversize output, unknown/mismatched/replaced executable을 검증한다.
운영 daemon/Compose/Daily 활성화는 없다.

WP-4 정확 소유는 WP-2 보고서 §8의 API/API tests/contract owner 세 행으로 고정한다.
prefix `/api/v1/backtests/owner-beta/price-only`, five operations(runs POST/GET, run GET/cancel POST/pair GET),
폐쇄형 body/metadata/full pair/error schema는 WP-2 §3~4를 채택하며 G1-1의 rights/cancel 결정이 우선한다.
새 repo는 app_pool+actor_tx로 모든 조회에 명시적 owner predicate를 함께 사용한다.
enqueue durable replay/capacity와 jobs+run 원자 삽입을 기존 Owner 패턴으로 구현한다.
cancel cache는 method+route+run_id+body에 묶고 매번 소유권을 다시 확인한다. 공유 admin 조회는 사용하지 않는다.
metadata 성공도 whole pair integrity를 확인하며 여러 list rows는 한 번에 모든 large bytes를 적재하지 않는다.
두 Owner-role 사용자의 actual RLS/HTTP tests가 필수이며 Member gate만으로 격리 증명을 대신하지 않는다.
HTTP503/500은 static message만 반환한다. generator+두 generated outputs는 WP-4 단독 소유다.

WP-5 정확 소유는 WP-2 §8의 Web/Web contracts-data/Web tests 세 행으로 고정한다.
child page `/backtests/owner-beta/price-only`, strict Owner+owner_only gate before ProductApi,
고정 config와 optional dates form, metadata-only bounded polling, both-cost+same-cost benchmark를 표시한다.
full SimulationResult schema를 strict Zod로 검증하고 이전 run/한 profile/권리 상실 후 결과를 보여주지 않는다.
새 metrics/월별/연율/robustness/시간-bearing execution date는 추가하지 않는다. decimal BigInt half-even,
percent/percentage-points 구분과 negative-zero 정규화는 전용 display helper에서 처리한다.
Next16.3.0의 검증된 `/data/workspace/lagrange/node_modules/next/dist/docs/` 참조를 읽기 전용으로 사용할 수 있다.
source 파일을 다른 checkout으로 복사하거나 설치된 환경을 cache에 넣지 않는다.
WP-4 wire를 수용한 뒤 UI 구현을 시작한다. UI synthetic fixture는 DB RLS/실제 입력 실행 증거가 아니다.

### G1-5. WP-3D — CI 전달물과 필수 통합 harness

정확 소유: `.github/workflows/ci.yml`, `scripts/ci/build-r4-verification.sh`, `scripts/ci/test_ci_contract.py`,
`deploy/db/integration-validation/{validate.sh,static-check.sh,EVIDENCE_TEMPLATE.md}`.
기존 opt-in/default-false `r4_verification_build` job 안에서 verifier 검증·build를 보존하고,
같은 pinned builder/Cargo target에서 runner strict Clippy/test/release build를 **순차** 추가한다.
job45분 제한은 유지한다. 기존 verifier delivery 두 파일을 보존하고 정확히
`owner-beta-simulation-runner`, `simulation-runner-provenance.json` 두 source-only 파일만 같은 delivery/artifact에 추가한다.
기존 contents:read/success-only upload/SHA-pinned actions/retention1일/no input artifact를 유지한다.
허용 path 수를 정확히4로 갱신하고 다른 경로·wildcard·provider data/log/receipt/cache 업로드는 회귀로 차단한다.
compile vars는 `R4_SIMULATION_BUILD_GIT_COMMIT`, `TOOLCHAIN`, `TARGET`, `PROFILE`, `WORKFLOW_RUN_ID`,
`WORKFLOW_RUN_ATTEMPT`에 같은 prefix를 붙인 이름을 사용한다. provenance JSON에는 source/run/attempt,
실제 rustc identity/target/profile/binary sha256와 해당 source hash를 기록한다.

PostgreSQL harness에 `-p job-queue --test owner_beta_simulation_execution`과
`-p api-server --test http_owner_beta_simulations`를 명시 실행하며 실제 최종 test 이름 inventory로
zero/ignored/SKIP/누락을 거절한다. 기존 모든 required CI/PG/Web/Research smoke 검사는 보존한다.
local shell/Python/static fast만 수행한다. hosted dispatch·download·실제 실행은 coordinator G3 책임이다.

### G1-6. 의존 순서와 수용

WP-3A -> WP-3B -> (WP-3C와 WP-4 병렬) -> (WP-3D와 WP-5 병렬) -> coordinator통합/도표 -> WP-6 -> G3/WP-7.
모든 구현은 gpt-6-luna/max로 시작하고 직접 diff/후속 hosted 검증으로 확인한다. 실제 실패 신호 전 선제 승급은 하지 않는다.
각 단계의 public Rust signature/SQL column/wire가 후속 시작 전에 확정되어야 하며 겹치는 파일은 순차 통합한다.
기존 WP-3의 넓은 저장/실행 범위를 네 bounded package로 나눈 이유는 취소·출처·DB 권한·CI의 서로 다른 경계다.
새 제품 범위나 승인 단계를 추가한 것이 아니다. 우선 WP-3A만 시작하며 이후 원래 의존 순서대로 이어간다.

실제 승인 입력 proof는 최종 source-only runner 출처를 확인한 뒤 G3에서 정확 date window/실행 횟수/정리 조건을
별도 기록하여 기존 read-only isolated procedure로 수행한다. hosted synthetic DB/API 검사를 실제 입력으로 부르지 않는다.
최종 코드가 구조를 바꾸므로 coordinator가 두 기존 .puml과 현재 file:line 근거·로컬 PNG를 갱신한다.
아직 존재하지 않는 배포 service/edge를 그리지 않는다. R4-04 isolation/R4-05 완료 체크는 G4 전까지 유지한다.

2026-10-24 14:01UTC — G1 수용 뒤 WP-3A `9a6da209-b08f-44dd-b159-1a29d84c3ed4`를 codex/gpt-6-luna/max, initial auto mode로 시작했다. parent/workspace와 알림 등록을 확인했다. 문서 fast receipt `target/dev-check/20261024T140034Z-9baa6a108f3e/run.json` PASS. 구현 완료나 hosted 검사 통과를 뜻하지 않는다.


### WP-3A source review and WP-3B interface freeze (2026-10-24)

WP-3A actual idle/pending0를 확인하고 8개 source hash, 0700/0600 report/receipt,
Rust fast receipt `target/dev-check/20261024T140941Z-b1ff5c6aa009/run.json` 및 diff를 대조했다.
shared window helper는 기존 검사를 추출했으며 계산 알고리즘은 바꾸지 않았다. 두 profile 원본 canonical bytes,
queued pins/window/strategy와 expected engine의 content binding을 확인했다. source-ready이며 compile/test/Clippy는
hosted pending이다. report hash는 `c80266284664e250d099249450a29932ebab98aea85e5dccbc98b95321ef2080`이다.
WP-3A report에 나열된 공개 signature/accessor를 후속 단계의 입력 계약으로 채택한다.

WP-3B public interface는 다음으로 고정한다. 모두 새 simulation_execution 경계이며 기존 Owner API를 바꾸지 않는다.

```rust
pub async fn publish_simulation_success(
    queue: &JobQueue, claim: &ClaimedJob, input: &SimulationJobInput,
    pair: &CheckedSimulationPair, expected_engine: &EngineIdentity,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError>;
pub async fn settle_simulation_failure(
    queue: &JobQueue, claim: &ClaimedJob, input: &SimulationJobInput,
    failure: SimulationPublicationFailure,
) -> Result<SimulationPublicationOutcome, SimulationPublicationError>;
pub async fn reconcile_simulation_commit(
    queue: &JobQueue, claim: &ClaimedJob, input: &SimulationJobInput,
    pair: &CheckedSimulationPair, expected_engine: &EngineIdentity,
) -> Result<SimulationCommitState, SimulationPublicationError>;
pub async fn recover_simulation_claims(
    queue: &JobQueue,
) -> Result<SimulationRecoveryReport, SimulationPublicationError>;
pub fn check_stored_simulation_pair(
    illustrative: &[u8], illustrative_sha256: &str,
    zero_cost: &[u8], zero_cost_sha256: &str,
    input: &SimulationJobInput, expected_engine: &EngineIdentity,
) -> Result<CheckedSimulationPair, SimulationPublicationError>;
```

`SimulationPublicationOutcome`: Published/Retrying/Failed/Canceled.
`SimulationCommitState`: Published/NotPublished/Unresolved. Published는 같은 owner/run/job/attempt/input/engine와
정확 원본 bytes/hash를 확인한 경우만, NotPublished는 일관된 non-success와 pair 부재를 확인한 경우만이다.
모순/불명은 Unresolved 또는 static integrity error이며 자동 재게시하지 않는다.
`SimulationPublicationFailure`: InputInvalid/EntitlementDenied/ProvenanceInvalid/ResultInvalid/
ComputationTimedOut/ComputationFailed/InfrastructureUnavailable/Canceled. InfrastructureUnavailable만 transient
재시도 대상이며 queue max_attempts=3를 넘기지 않는다. 나머지는 terminal 또는 취소이며 오류는 value-free다.
`SimulationPublicationError`: DatabaseTransient/DatabaseIntegrity/QueueTransient/QueueClaimLost/QueueIntegrity/
Integrity/EntitlementDenied/CommitUnknown; static Debug/Display 및 ErrorClass 대응을 제공한다.
`SimulationRecoveryReport`는 jobs_checked/attempts_orphaned/jobs_requeued/jobs_failed/runs_canceled/
runs_failed_healed/jobs_skipped의 usize counts만 반환한다. actual executable attestation은 WP-3C가 제공하며
expected_engine만으로 증명됐다고 보지 않는다.

G1-3 SQL 이름을 명확히 한다: `strategy_config_json`, `strategy_config_sha256`, `status`를 기존 패턴대로 사용한다.
engine은 nullable `engine_json jsonb`, 성공 attempt는 nullable `published_attempt_id uuid REFERENCES job_attempts(id)`로
저장하고 SUCCEEDED에서만 둘 다 필수다. 전체 성공 결합은 현재 claim attempt 및 job 관계와 함께 검증한다.
나머지 G1-3 column/label/pin 이름과 pair bytea/hash 이름은 유지한다. app INSERT 허용 목록에는 engine/attempt나
terminal fields를 넣지 않는다. DB가 canonical ledger 계산기를 대체하지 않으며 읽을 때 양쪽을 다시 검증한다.
API의 owner-app query는 WP-4 소유이고 storage helper는 DB lookup 없이 전달된 양 profile을 검증한다.
공유 simulation_execution/mod.rs export는 worker 완료 후 coordinator가 순차 연결한다.

WP-3B tests는 WP-1 §5 중 저장/역할/원자성/typed claim/recovery/migration 항목을 구현한다.
actual HTTP enqueue/replay/capacity/cross-owner cancel은 WP-4에서 실제 API로 검증한다. WP-3B에서는 SQL transaction
rollback과 actor queue cancellation을 검증하되 API가 존재하는 것처럼 보고하지 않는다.
CommitUnknown 실제 장애 주입을 못 하면 미검증으로 남기고 별도 저장 상태 reconciliation branch만 테스트한다.

2026-10-24 — WP-3B `91e596d4-1d0c-4b5b-8397-4e26f92d1bf4`를 codex/gpt-6-luna/max, initial auto로 시작했다. 정확 assignment와 쓰기 범위는 새 ledger에 기록했다. 저장/RLS/게시/복구 source와 회귀 작성이며 실제 DB·compile 검증은 hosted pending이다.


### WP-3B coordinator integration and next-worker contracts (2026-10-24)

WP-3B idle/pending0와 원 보고서9개 source hash를 검증했다. 원 report hash
`7adeb658d23fb628cc903ccc525c0a05ae02bfbac21bed02f11d1ea930d24851`와 원 receipt는 보존한다.
코디네이터가 app pair SELECT grant 누락, 두 test의 constant import, 실제 Owner user_roles fixture를 보정하고
simulation_execution module exports를 연결했다. 이후 Rust fast
`target/dev-check/20261024T144836Z-cdca2f17c355/run.json` 및 DB static/self-test
`target/dev-check/20261024T144836Z-3de8bd9d92e6/run.json` PASS. SQL 실행/compile/test/Clippy는 미실행이다.
기존 source-only 완료 주장과 이후 보정은 구분하며 최종 독립 리뷰/hosted PG가 필요하다.
새 SQL session_count는 warmup+evaluation의 전체 소비 session 수이며 API evaluation_session_count와 구분한다.
WP-3B public signature/column은 후속 구현의 입력으로 수용한다. 실제 CommitUnknown 전송 장애 proof는 OPEN이다.

WP-3C 추가 freeze:
- startup trusted receipt는 closed JSON의 schema=`owner-beta-simulation-build-v1`, git_commit, toolchain,
  target, profile, workflow_run_id, workflow_run_attempt, binary_sha256, source_hashes를 갖는다.
  source_hashes는 repo-relative file path->sha256 문자열 map이다. 모든 SHA 필드는 소문자64hex(no sha256 prefix),
  git_commit은40hex, run/attempt는 positive ASCII decimal strings다. EngineIdentity로 옮길 때 기존 hash 타입을 사용한다.
  CLI expected receipt digest는 coordinator가 exact same-SHA CI에서 검증한 전달물 pin이고 HTTP/payload에서 받지 않는다.
  receipt 자체가 authentic CI를 증명하는 것은 아니며 G3에서 origin/run/attempt/digest를 따로 대조한다.
- 전용 parent binary의 `--once` 또는 `--daemon` 중 정확히 하나, `--artifact-root`, `--build-receipt`,
  `--expected-receipt-sha256`, `--worker-id`를 operator-only 인수로 둔다. 부모 DB 연결은 기존 env 설정을 사용하며
  비밀번호를 argv/로그에 출력하지 않는다. service/Compose 등록은 없다.
- 같은 executable child는 `--child`, 동일 trusted root/receipt/digest를 operator args로 받고 stdin256KiB 이내의
  SimulationJobInput JSON 한 개만 처리한다. env_clear, 절대 executable path, shell/PATH 검색 없음.
- stdout은 폐쇄형 `{schema:"owner-beta-simulation-child-pair-v1",illustrative_canonical:string,
  zero_cost_canonical:string}` 한 개다. 두 문자열은 각 원본 canonical JSON bytes의 UTF-8 표현이며
  parent는 문자열 bytes로 CheckedSimulationPair를 검증한다. 전체 stdout64MiB 한도를 읽는 도중 적용한다.
  추가 필드/중복 key/trailing data를 거절하고 결과/자유 stderr를 로그에 남기지 않는다.
- 공개 facade 이름은 `TrustedSimulationRuntime`, `SimulationWorkerError`, `SimulationWorkerOutcome`,
  `run_simulation_once`다. runtime의 validate/load constructor는 root/receipt path+expected digest를 받고
  private fields에 실제 executable/engine binding을 고정한다. once는 queue, worker_id, runtime, shutdown signal을 받아
  정확 새 type만 claim한다. 상세 internal process types는 외부 API가 소비하지 않으며 보고서에 확정한다.
  enum outcome은 Idle/Published/Retrying/Failed/Canceled/ClaimLost/CommitUnresolved로 제한한다.
- 독립 heartbeat는 child 계산 및 비용 큰 parent pair 검증/게시 준비 전체에 유지한다. 동일 process-group의
  child/descendants 종료·wait/reap 전에 once/slot을 반환하지 않는다. shutdown은 사용자 cancel을 위조하지 않고
  owned child 정리 후 유효 lease에 한해 InfrastructureUnavailable로 정산하거나 loss 시 recovery에 맡긴다.
  unknown commit는 frozen reconciliation 결과만 반영하고 새 pair 삽입/재실행하지 않는다.
- cfg(test)의 실제 fixture process tests는 source-only CI에서 가능하되 real artifact/real provenance 성공과 구분한다.
  누락 compile vars로 정상 execution claim을 허용하지 않는다. local execution/DB/container는 여전히 금지한다.

WP-4 추가 freeze:
- WP-2 §3~4 full pair/metadata/wire, §8의 API/API test/contract-owner 세 행을 정확 소유로 사용한다.
  G1-1 전체 소비 session의 Backtest 권리 검사와 entitlement와 별개인 own cancel 결정이 우선한다.
  저장 session_count와 API evaluation_session_count를 혼동하지 않는다.
- 성공 enqueue의 실제 artifact approval은 production constructor만 통과한다. hosted 환경에 고정 v3 원본을
  복사하지 않으며 입력 승인 우회 flag/feature/환경변수/HTTP 필드는 만들지 않는다.
- synthetic admission을 쓰는 enqueue/replay/capacity HTTP+실제 PG 테스트는 `cfg(test)` library unit 경계에서만
  허용한다. private validated-input seam은 release library/binary에 존재하지 않아야 한다. 기존 app/router/auth/
  actor transaction/DB 코드는 실제로 거치되 artifact 승인 성공 proof라고 부르지 않는다. state/http owned 파일의
  test-only helpers와 최소 isolated harness를 사용하며 production sealed artifact 타입을 위조하지 않는다.
  별도 integration target은 production admission 거절 및 seeded actual PG read/cancel/isolation/integrity를 검증한다.
  WP-3D는 필요 시 `-p api-server --lib http::owner_beta_simulations::tests::`의 실제 DB named cases도 명시 실행한다.
- OPEN artifact-to-actual-enqueue/runner chain은 G3 coordinator의 existing read-only approved-input 검증에서 별도로
  증명한다. fixture나 test-only seam으로 이를 완료 표시하지 않는다. compile-time cfg(test) 경계로 구현할 수 없으면
  정확 문제를 보고하고 production bypass를 추가하지 않는다.

WP-3C와 WP-4는 서로 비중복 파일로 병렬 실행한다. 두 worker는 shared simulation_execution/mod.rs,
계획/ROADMAP/diagram/CI를 수정하지 않는다. worker 완료 뒤 coordinator가 필요한 module exports만 순차 연결한다.

2026-10-24 14:54UTC — WP-3C `c1a6a18a-89f1-43f1-aedd-72f76f66cb1e`와 WP-4 `d22970d0-4430-46b2-8963-61882dc6ef00`를 codex/gpt-6-luna/max, initial auto로 병렬 시작했다. 비중복 assignment·parent/workspace·완료/승인 알림을 확인하고 ledger에 기록했다. 앞선4명은 actual idle/pending0·보고 회수 상태다. 문서 fast receipt `target/dev-check/20261024T145247Z-d792b5f5f637/run.json` PASS. 실제 compile/DB/runner/HTTP 인수는 미완료다.

2026-10-24 — 후속 소스 대조에서 WP-3B publish/recovery의 job-type constant import 두 곳을 실제 simulation_execution 경계로 보정했다. 두 파일의 rustfmt/check 및 diff check PASS. 원 영수증을 덮어쓰지 않고 `target/r4-connection/coordinator-wp3b-imports.json`에 보정 해시를 기록했다. 컴파일/DB 검증은 hosted pending이다.

### WP-3C recovery and bounded corrections (2026-10-25 KST / 2026-10-24 UTC)

WP-3C actual idle/pending0, 원 보고서 hash `fd51e520e5bc50b8c3ed2bd6f5a5d885d10f83fede6dc24ca18942030761bfdf`,
소스5개 hash와0700/0600을 확인했다. source-only이며 compile/DB/runtime은 미검증이다. 공유 module exports는
코디네이터가 연결했다. 코디네이터 소스 검토에서 async 결과검증 await 누락, 기존 Sha256Digest에 필요한
sha256: 접두사 변환 누락, 부모 SIGTERM 처리 부재와 자식 정리 확인 경계를 확인해 WP-3C에 동일 범위 보완을 맡긴다.
원 formatter 실패는 당시 동시 API 파일의 포맷 상태 때문이며 최종 실패/성공과 구분해 보존한다.

WP-3B reported malformed claimed payload의 안전한 종료 경로를 다음 additive API로 고정한다:
`settle_invalid_simulation_claim(queue: &JobQueue, claim: &ClaimedJob) -> Result<SimulationPublicationOutcome, SimulationPublicationError>`.
입력이 실제로 SimulationJobInput 파싱에 실패할 때만 호출 가능하며 valid payload에는 Integrity를 반환한다.
DB의 exact typed job/current claim/owner/attempt를 먼저 잠그고 run은 payload의 임의 ID 대신 job_id+owner로 찾는다.
일치하는 PENDING run/no pair/engine absent를 확인한 뒤 job+attempt+run을 하나의 transaction으로 terminal InputInvalid 처리한다.
취소는 실제 job cancellation을 보존하고 stale/newer claim/다른 type/owner에는 수정하지 않는다. identity 불명은 fail closed이다.
파손 payload를 고쳐 쓰거나 재계산/재시도하지 않으며 실제 commit 불명은 기존 CommitUnknown으로 남긴다.
전용 recovery도 동일하게 invalid schedule/input의 pending run을 원자 terminal로 정리하고 뒤 valid job을 막지 않는다.
real PostgreSQL 회귀로 nil/noncalendar/warmup-invalid input, stale claim/cancel, no pair/다른 owner 비변경을 검증한다.

현재 mutable strategy config는 enqueue 때 active+fixed로 승인하고 그 snapshot을 run/job에 고정한다.
후속 config 비활성화·편집 때문에 실패/취소/복구 정산이 불가능해져서는 안 된다. 실행은 원본 snapshot을 사용한다.
새 migration0063의 UPDATE/success lock은 존재·owner 및 immutable stored snapshot을 검증하되 현재 config 값/active로
이미 승인된 run을 재해석하지 않는다. 기존 다른 제품 migration/rights는 바꾸지 않는다. 권리는 publication 때 계속 재검증한다.

G3 실제 실행 대상은 Linux pinned container이다. 다른 OS의 불완전한 termination을 정상 지원으로 간주하지 않는다.
추가 플랫폼 API/dependency를 도입하지 않고 non-Linux execution은 claim/spawn 전 fail closed로 두며 제한을 명시한다.
WP-3C와 WP-3B 보완은 각 기존 owned source 범위, WP-4는 기존 API 범위로 계속 분리한다.

### WP-4 recovery and read-snapshot correction (2026-10-25)

WP-4 actual idle/pending0, report `e3ce4078ea3cb26fb9930a522f76a417f223d12a1a57a7ccce5f684febf43078` and13 source hashes were recovered. Five-route HTTP wire remains frozen; compiler/real PostgreSQL checks were not run. The coordinator found app-role reads using FOR SHARE on run/pair tables without UPDATE privilege. Preserve those narrow grants. Use app-pool repeatable-read read-only transactions initialized before actor GUC queries for metadata/pair reads, with local actor/timeouts and full pair validation. Enqueue retains read-committed capacity serialization; no new role, grant or public route. Add real-role and deterministic same-connection GUC reset assertions. Original reports and failed dependency-backed OpenAPI check remain preserved. Coordinator separately generates TypeScript from the authored spec using the exact locked existing installed generator, without installing dependencies or reading approved inputs.

Coordinator generated `apps/api-server/generated/openapi.ts` using the existing installed openapi-typescript7.13.0 and the authored generator call (`openapiTS(spec,{prettier:false})` + `astToString`). Candidate and existing installation checkout package-lock hashes match; generated SHA-256 `36639ae194aad6cc4ef5ea8992a374812f798e9f11fe9117b98307d892399852`. Receipt: `target/r4-connection/coordinator-openapi-generation.json`. No dependency install or manual generated-output editing. Full OpenAPI metadata lint and TypeScript typecheck remain required in hosted Web; the worker's original missing-dependency failure is preserved.


### WP-3B correction recovery (2026-10-25)

Actual idle and no pending permissions were confirmed. Five correction source hashes, the correction report
(`2894d1dcede2e865c7a422db43f6817b388058ed64de73aa515cd0c0429b62d5`), private modes, and both preserved original receipts matched.
The coordinator traced exact claim fencing, canceled-job lock retention, invalid-payload recovery, and the insertion-only
live strategy snapshot check. The additive `settle_invalid_simulation_claim` export is integrated. Source is ready for
its dependent implementation; authored PostgreSQL tests, compilation and runtime acceptance remain pending.

Correction to the earlier test expectation: the frozen WP-3A parser accepts nil UUIDs when the remaining payload is valid.
Nil alone is therefore not malformed input under this contract. The new settlement API correctly refuses such a valid
payload without mutation. Keep that contract and test both valid-input refusal and actual calendar/warmup parser failures;
no new UUID restriction is introduced. The worker report's “WP-3C correction” opening is a label typo; its five-file scope
and receipt are WP-3B. The original report remains preserved.


### Dependent implementation freeze (2026-10-25)

WP-3C and WP-4 corrections reached actual idle/pending0; respectively five and three source hashes matched.
Original reports and receipts remain unchanged. Coordinator integrated the shared invalid-claim export,
cloned the child cancellation Arc before its move into the blocking supervisor, released the watch borrow
before awaiting cleanup, and removed a redundant trait import. Targeted rustfmt/check and diff check passed.
Receipt: `target/r4-connection/coordinator-correction-integration.json`. Compilation/process/PG acceptance is pending.
WP-3C correction report hash: `5872eb8ac57afe12fb69872d07b3b1d9e6015009ef929f7015226bfd04974876`.
WP-4 correction report hash: `942da85c5a4d28268e964aadf02fe3e7e47655ef22d2300415589c02f81ce45c`.

The app read paths now use repeatable-read read-only snapshots and preserve narrow role grants. Enqueue
keeps read-committed capacity serialization. Real app/no-UPDATE and same-connection actor reset assertions
are authored, not executed. The public wire and all seven HTTP integration names are unchanged. These
interfaces are accepted for the original WP-3D and WP-5 dependent implementations, not as isolation proof.

Frozen assignments: `target/r4-connection/wp-3d/assignment.md` (six existing CI/harness files) and
`target/r4-connection/wp-5/assignment.md` (the G1 Web paths). They have disjoint write scopes. WP-3D's
explicit inventory is fifteen new PG storage tests, one API library admission/replay test and seven API
integration tests, plus the source-only child/provenance/runner/binary cases; none may silently skip.
The optional source artifact remains exactly four explicit delivery files. WP-5 uses the generated wire,
full strict paired result validation, and metadata polling every two seconds up to sixty attempts per
selection, with manual refresh after exhaustion and no overlapping requests. No local heavy checks or
actual approved input access are authorized for either worker.


### Architecture draft integration (2026-10-25)

The coordinator added the dedicated typed runner, same-executable attested child, app-actor read
snapshots and migration0063 whole-pair table contract to both evidence-bound diagrams. Existing API
route line references shifted by this change were refreshed. The R4 execution box explicitly has no
Compose service, installed activation or production migration claim. WP-5 Web edges are pending its
completed source, followed by the final diagram refresh and independent review.

Local rendering used the existing pinned PlantUML image, a shared user-wide advisory slot, network
none, read-only container root, non-root UID, 768MiB memory/no swap and one CPU. Both renders exited0,
OOMKilled=false and each owned container's removal/absence was checked. The initial render exposed
right-edge clipping at PlantUML's default4096px width; the source scale and explicit renderer size limit now preserve the full canvas.
Receipts are `target/r4-connection/diagrams-1553/receipt.json` and
`target/r4-connection/diagrams-full-canvas/receipt.json`. The final natural-resolution render uses
`PLANTUML_LIMIT_SIZE=16384` with source scale capped at 16000px; it produces complete 6199 × 1623 and
14091 × 2108 PNGs, preserving label resolution. Its receipt is
`target/r4-connection/diagrams-20261024T155209Z/receipt.json`. The final Web edge refresh remains pending. This is diagram verification only, not product/runtime isolation proof.


### WP-3D recovery and integration (2026-10-25)

WP-3D reached actual idle/pending0. Six implementation hashes, 33 build-source hashes, the preserved
assignment/inventory, report and private modes matched. Report SHA-256:
`a6b4b0f11ab6bedb3b12ec164b81b4ecc5951ad45fce614205fbbe7e35c8cfda`.
The coordinator checked the four-file upload allowlist, sequential source build, closed provenance,
named-case parser and three PostgreSQL lanes. The authored gates cover 19 new source-only Rust cases
and 23 PostgreSQL cases; these Rust/DB cases have not run yet.

The root builder created private metadata/logs which the host runner could not read/remove under the
original ownership. An EXIT trap now hands only those named outputs to the actual host UID/GID,
retains their 0600/0700 modes and preserves an original failing exit status. The verifier command's line
continuation was also aligned with the CI contract without changing its arguments. Shell syntax,
four exit/handoff fixtures, private mode checks, the 11-command contract and diff check passed.
Receipt: `target/r4-connection/coordinator-wp3d-integration.json`. Actual root-to-host handoff is still
part of the hosted build proof.

The worker's original missing-PyYAML result remains preserved. The coordinator found existing system
Python 3.14.4/PyYAML 6.0.3 and ran `python -m unittest scripts.ci.test_ci_contract -q`: 43 tests PASS.
Hosted uses PyYAML 6.0.2, so this local check does not replace the final hosted gate. No dependency was
installed. Source wiring is ready for independent review; Web implementation, real compilation,
PostgreSQL, artifact origin and approved-input runtime acceptance remain pending.


### G2 integrated Web and diagram freeze (2026-10-25)

All eight implementation/audit workers reached actual idle with no pending requests. WP-5's sixteen
source hashes and frozen generated OpenAPI hash matched; report SHA-256
`0cd2a4769f17a5b0e057e1888224c2c0c802b90b41010b5ce82d1274cac0de1f`.
The coordinator checked the role/mode gate before ProductApi, current-rights gate, dedicated mutation
and read paths, bounded metadata polling, selected-run/pair binding, stale-response epoch checks,
logout purge and BigInt rounding. This is source review, not executed Web acceptance.

Correction to WP-5 assignment prose: optional enqueue dates are `start_date` and `end_date`, as the
frozen generated OpenAPI and server DTO already specify. The worker correctly followed that wire;
no API or calculation contract changes are required. Preserve the original assignment/report.

Both diagrams now include the dedicated Web-to-API calls with current file:line evidence. Local
render receipt `target/r4-connection/diagrams-20261024T163458Z/receipt.json` records exit0,
OOMKilled=false and owned-container absence. Complete PNG canvases were inspected: component
6199×1623 and runtime14307×2170. No product service, migration or runtime was started by rendering.

The integrated source will be frozen in WP-6's private review manifest before launch. Independent
review must distinguish mandatory defects from unexecuted acceptance, including the production
approved-artifact-to-enqueue and actual attested runner/publication path. Existing synthetic PG/API
fixtures and source-only negative process tests do not establish that complete positive path.
G3 must bound the exact verification method and source-only delivery requirements before dispatch;
no unverified step may be represented as acceptance or silently omitted. Final hosted checks have
not been dispatched; the existing four-file artifact contract remains unchanged at this gate.


### WP-6 findings and bounded corrections (2026-10-25)

Independent review reached actual idle/pending0 with all64 source and22 protected receipt hashes
unchanged. Verdict CHANGES_REQUIRED; report SHA-256
`ffd4f50228f07e56a3d797632ce371d912ccd4f6053225af2082bd0f00a9c6b9`.
The coordinator confirmed all five findings against source. F1 is a mixed borrowed/value TradingDate
comparison in the API cfg(test) seam; F2 is a missing exported Web metrics type. Both were corrected
in their two existing files, with targeted rustfmt/Biome and diff checks PASS. No compiler was run.
Receipt: `target/r4-connection/coordinator-review-f1-f2.json`. Preserve the original review/manifest.

F3 is the substantive Web defect: checking POST status before typed error parsing bypasses access-loss
purge/login recovery. F4 is invalid submitted-fixture timestamp ordering; F5 is an ambiguous cancellation
status locator. The original WP-5 worker will fix only the six paths and meaningful mutation-boundary
regressions frozen in `target/r4-connection/wp-5/correction-assignment.md`. Runtime/schema/wire and
financial rules remain unchanged. No repeated same-cause failure has occurred, so its Luna/max model
is retained. After correction, refresh shifted diagram citations/PNGs and independently review fixes.

Review also confirms the positive sealed admission/authenticated enqueue/attested runner/publication/
authenticated read chain lacks an executable API proof harness. The current four-file delivery remains
frozen while the coordinator specifies a minimal opt-in source-only acceptance driver and separately
bounded provenance delivery. This is required verification infrastructure within R4-05, not a new product
API, provider allowance or deployment. G3 must fix its files, interfaces, private scratch-DB transport,
short approved window, resource/cleanup checks and delivery allowlist before implementation or dispatch.
No current CI, smoke, actual approved-input execution or R4-04 isolation completion is claimed.

### G3 bounded positive connection proof — frozen source addition (2026-10-25)

WP-3E is an intermediate/high-confidence, bounded verification implementation assigned to
gpt-6-luna/max, with direct coordinator source review and a follow-up independent review. Its only
source writes are the new `crates/api-server/examples/verify_approved_simulation_connection.rs`,
`examples/simulation_connection_proof/mod.rs`, and narrow Unix-socket query preservation in the
existing `tests/common/mod.rs` role/database URL helper. Exact interfaces, six source tests and
private-output limits are frozen in `target/r4-connection/wp-3e/assignment.md`. No production
route, migration, role grant, dependency or calculation rule is added. Contradictions or two failures
with the same cause reopen only this branch; they do not permit guessed requirements.

The opt-in driver uses the real release API library/router, stored sessions, CSRF and actor pools.
Its fixed approved window is 2017-06-22 through 2017-06-23, with exactly200 warmup and2 evaluation
sessions required from the actual calendar. It enqueues once, records a private readiness receipt,
and waits for the separately delivered attested runner's single `--once` execution. It checks pending
pair nonvisibility, complete canonical pair/input/engine binding, one successful attempt and atomic
stored state, owner API read/list, foreign/missing404 equivalence, direct RLS absence, durable replay,
and scratch rights revocation. It emits only hashes, counts and boolean checks. The earlier synthetic
PG suite remains responsible for failed/partial/cancel/recovery branches; it is not replaced.

Before any DB mutation the driver requires the exact Unix-socket URL
`postgres://postgres@localhost/postgres?host=/r4-proof-db`, a nonce-bound cluster_name, empty
listen_addresses, a Unix connection and an otherwise empty dedicated cluster. The production
approved root is mounted read-only in an isolated API proof container; its parent is the API's
artifact configuration without changing the harness-owned temporary directory or teardown target.
The operator owns the scratch PostgreSQL container/socket and always-run removal/absence checks.
Driver, runner and DB are separate processes; the runner compute boundary remains512MiB/no added
swap,CPU1,PID32,nonroot,network-none,read-only root. API proof and scratch DB are bounded to512MiB
each,CPU1,PID128/64 respectively, and no TCP listeners or production network/volume are exposed.
The driver deadline is1500s; the existing child deadline is1200s. The shared heavy advisory slot is
held across this isolated validation and cleanup. No local compilation is permitted.

WP-3D will add the proof example's strict source-only Clippy, exact six test rows and release build
sequentially in the existing opt-in hosted build, then deliver a separately named artifact containing
exactly `verify_approved_simulation_connection` and `connection-proof-provenance.json`. The existing
four-file artifact allowlist remains unchanged. The proof receipt uses schema
`owner-beta-simulation-connection-proof-build-v1` and binds the executable, source hashes, compile
identity and exact same CI SHA/run/attempt as the runner. Both receipts are locally hash checked;
authenticated hosted origin and artifact digests are independently verified by the coordinator.
No approved inputs, results, DBs, session material or logs enter either artifact or hosted runner.

This addition is required verification infrastructure within R4-05. It does not authorize deployment
or production migration. Actual Unix transport, process resource/cleanup evidence and positive
connection acceptance remain unexecuted until G3. A short-window success would not establish
full-default-window performance, real commit-acknowledgement-loss injection or cleanup-fault behavior.
After WP-3E/WP-3D and Web corrections settle: integrate, refresh diagram evidence/PNGs, independently
review corrections, freeze the final candidate, then run existing same-SHA CI/required and Research
smoke each once and perform the bounded approved-input proof only from authenticated delivery.


### WP-5 correction recovery (2026-10-25)

WP-5 reached actual idle/pending0; all10 owned/report/protected/reference hashes and0700/0600
modes matched. Correction report SHA-256 `08f7e36df1a4e30f56da2821e891a959f590c921f3be7b978271233fc603f4d8`.
Coordinator confirmed F3 parser ordering and shared access epoch/cache purge, F4 creation-relative
fixture timestamps and actual submitted-route schema regression, and F5 unique acknowledgement
locators. Five new mutation-boundary browser cases and one fixture-schema case are authored but
unexecuted. Original reports/receipts and frozen generated wire remain preserved. Receipt:
`target/r4-connection/coordinator-wp5-correction-review.json`. Final independent correction review,
diagram evidence/PNG refresh and hosted TypeScript/unit/build/E2E remain pending alongside G3.


### WP-3D G3 delivery recovery (2026-10-25)

The proof-delivery worker reached actual idle/pending0. All15 changed/report/protected/reference
hashes and0700/0600 modes matched. Report SHA-256
`39553963bde0a7713fb381df94ef88fd0c30262a8b462bfbf1a9a7c5a4319e14`.
Coordinator reviewed the unchanged original four-file upload, separate two-file final success
upload, sequential source-only proof commands, six compile identity fields, static executable
checks, closed receipt binding and private ownership handoff. Worker CI-contract suite47 PASS;
no Cargo/hosted/runtime validation was run. Original artifacts and interpreter limitation remain.
Receipt: `target/r4-connection/coordinator-wp3d-proof-delivery-review.json`.

The854-path tracked inventory matched at recovery but is only a concurrent-worktree observation.
WP-3E's untracked main file changed after the worker snapshot; its separate observed hash is not
final candidate evidence. After WP-3E settles, include all new sources in the final clean revision,
verify the resulting compile-input inventory, update diagrams/PNGs and perform independent
correction review before the one same-SHA hosted CI/required and smoke dispatch.


### G3 driver recovery and correction review freeze (2026-10-25)

WP-3E reached actual idle/pending0. Its two owned files, narrow conn_url helper range and report
hashes matched; only coordinator-owned roadmap/plan references had changed. Original manifest
SHA-256 `17cd4051466c34b162c0f50cdea36db8c7fda35a3a357309d5338af2a6fd0a58` is preserved.
The existing strict parser is crate-private, so its local duplicate-key-rejecting receipt decoder
is accepted without widening product exports; typed jobs/results retain the public contract parsers.
The coordinator removed an unused import and documented the bounded request helper's narrow
argument-count lint allowance, matching the existing Harness request-helper convention. Targeted
rustfmt PASS; all six frozen test names are present but unexecuted. Integration receipt:
`target/r4-connection/coordinator-wp3e-integration.json`.

A source-only rehearsal of the hosted inventory with current tracked plus intended new source
paths covers872 inputs, including the proof main/support and shared harness. It is recorded in
`target/r4-connection/coordinator-g3-source-inventory.json`; final committed-checkout inventory and
hosted provenance must still be verified. Both diagrams now refresh Web citations and fold the
QA-only proof's in-process API, typed pair, private scratch DB and readiness dependencies with
concrete source evidence. Local render `diagrams-20261024T172153Z/receipt.json` records exit0,
OOMKilled=false and owned-container absence; complete6199x1623/14307x2170 canvases were inspected.

Freeze a new correction manifest without overwriting the original WP-6 review. Reuse the idle
independent gpt-6.1-sol/high reviewer for F1-F5, integrated G3 driver/build boundaries, and source
compatibility/validation risks. It may write only its new private correction report and receipt.
No source/document changes while that review runs; ledger/monitor bookkeeping remains separate.
Final hosted CI/required+smoke, authenticated artifact delivery, exact isolated runtime preparation,
and real approved-input acceptance remain pending. The short proof does not establish full-window
performance or real commit-ack-loss/cleanup-fault injection.


### G3 final bounded source correction (2026-10-25)

WP-6 correction review reached actual idle/pending0; all66 changed and38 protected hashes
and private modes matched. Report SHA-256
`235f73ff66f4de7631e2a34027b5d3de4c72fe600afd593c14e85e4707593d91`;
receipt `024fe337413f34ee424166b9efeb897c018f92dd0528805a77254aecb6a4eca6`.
Original F1-F5 are resolved in source. New C1/C2 are confined to the proof support module:
its private BuildReceipt lacked Clone used by receipt tests, and valid_metadata rejected the
production API's RUNNING/null-start shape (the job is claimed before the run is settled).

The coordinator derived Clone and allowed a nullable RUNNING start while retaining absent
finished_at and all identity/window/label/success checks. The existing pair validation test now
parses that actual response shape and rejects finished RUNNING, incomplete SUCCEEDED, wrong
identity and FAILED states; all six frozen case names and hosted gates remain unchanged.
Targeted rustfmt and git diff --check passed; no Rust compilation/test/Clippy was run. Receipt:
`target/r4-connection/coordinator-g3-c1-c2.json`. No structural edge or pre-test evidence line
moved, so current diagrams and PNGs remain valid without another render.

The review's optional readiness observation is assigned to runtime orchestration: wait for the
fixed stdout completion marker emitted after ready.json is written/synced, then validate the
closed private receipt. File existence alone is not readiness. This does not change the driver's
wire or authorize a product change. The private preparation note records the requirement.
Existing reviewer may confirm only C1/C2 and their regression delta against a fresh manifest;
no new implementation/validation worker is needed. Once no mandatory source issue remains,
freeze the clean candidate and follow the existing hosted/artifact/isolated proof gates.
Actual running interval, atomicity, cross-user denial, cleanup and full-window limitations remain
execution evidence dependencies; source corrections alone do not close R4-04 isolation.


### G3 source acceptance and hosted candidate (2026-10-25)

The final reviewer reached actual idle with no pending requests. READY_FOR_HOSTED resolves C1/C2
and preserves F1-F5 source resolution. All66 changed and45 protected hashes match. Final report
SHA-256 `5c9b349b13889ee687fe744ca22afb9a0e37a96966d30072cf78228def543f42`; receipt
`8c4b939ff73300842304b84d16ff3279edd2c6e49b408033660ac60eb7b8cfd9`. Original failed reviews remain preserved.

This documentation-only acceptance update is the sole delta after the reviewed source freeze.
The candidate is the commit containing this record. Verify the complete872-path compile inventory
against its committed tree, push the existing review branch without force, then dispatch existing
CI (including optional source-only build and required) and Research smoke once each with the same
SHA and cache epoch. Record exact run/attempt identities in the private WP-7 receipt. All source,
PostgreSQL and Web named gates remain mandatory; a failure requires diagnosis, not a blind rerun.
Authenticated static build artifacts and the separately frozen scratch/Unix-socket runtime proof
follow hosted success. No compile/test/runtime result, user isolation or R4-05 completion is claimed
by this source acceptance. R4-04 isolation remains blocked on actual connection evidence.


### G3 first hosted result and bounded correction (2026-10-25)

Candidate `ccb9d8f596f6333821c8842e697edb6ee78314ce` ran CI `40117548175` and
Research smoke `40117550387`, each attempt1 and exactly once. CI failed six jobs plus
`required`; format and Python tests passed. Smoke completed all20 steps with functional PASS.
Its cleanup summary observed no project containers, volumes, networks or context image.
Private-builder removal has no independent absence check, and successful cache-save steps do
not establish persistence or performance. Both artifact listings were empty. The19 source,
23 PostgreSQL and6 proof named results remain unverified; runtime isolation is not accepted.

WP-7 reached actual idle/pending0. The coordinator independently verified66 changed,
52 protected and872 compile-input hashes against the original candidate, then preserved the
worker report and receipts. The worker's narrow log filter lacked detailed CI diagnostics;
coordinator reads of the same exact job logs recovered the following bounded corrections:

- Web request parsing omits absent optional date properties; the run URL helper returns its
  exact route template type. Existing contract tests cover both absent and supplied dates.
- The non-Linux isolation error variant and match arm are platform-gated; two child-reaping
  conditions use let chains. Fail-closed behavior and cleanup remain unchanged.
- Two list queries wrap fixed SQL composition in `AssertSqlSafe`; all owner/cursor/limit
  values remain bound parameters, with no grant or transaction changes.
- WP-3D aligns the full-release migration guard and two backup/grant fixture inventories with
  migration0063 in exactly four assigned files. Historical staged guards remain unchanged.

The original candidate, failed reviews, run identities and private WP-7 evidence remain
preserved. The coordinator owns the five Web/Rust/API source files and documentation/diagram
citation refresh; WP-3D owns only the four migration-fixture files. Targeted Rust formatting
passed. Local Web lint initially could not find Biome; verification uses the existing pinned
2.5.7 installation through the Web workspace and passed (305 files, no fixes,6 warnings/69 infos).
Rust and docs fast checks passed; refreshed citation PNGs rendered locally with exit0/OOM false
and confirmed owned-container absence. Compile, typecheck, DB and browser checks remain
hosted obligations. After combined source/hash review and independent correction review,
freeze a new candidate and run existing CI/required plus Research smoke once each on that SHA.
Authenticated artifacts and a separately frozen approved-input scratch runtime follow hosted
success. No production or provider work, input copy/upload/recollection, or blind rerun is allowed.

WP-3D subsequently reached actual idle/pending0. The coordinator verified all four changed
source hashes against the worker receipt and original candidate, plus19 preserved report/receipt
hashes and private modes. Bash syntax, Python AST (no imports), DB static check including its
no-Docker self-test, diff whitespace, and the exact0001..0063 up/down inventory passed. The only
additional diagnostic edit changes the grant fixture's printed count to63. This is source/static
readiness for independent correction review; actual PostgreSQL setup and named cases remain
unverified. Freeze this combined delta before dispatching the existing independent reviewer.


### G3 corrected source accepted for new hosted candidate (2026-10-25)

The independent reviewer reached actual idle/activeTurn null/pending0 with READY_FOR_HOSTED.
Report SHA-256 `158b3044bdc9d938687d1c3d9ed55879463a648fa31f49590e52bc23cf686424`;
receipt `df2c0c8fc75a5516ae8157d67e214c1ba87ebf9ccc8eb49a7fd581b586a70c75`. The coordinator
independently matched all68 source,85 protected evidence and872 compile inputs against start/finish
review receipts. All four observed failure classes are resolved in source; no mandatory source
issue remains. Earlier failure/READY reports and exact first-run evidence remain preserved.

This ROADMAP/plan acceptance record is the only post-review delta. Commit the corrected candidate
on the existing review branch, verify all source and compile inputs against committed blobs, push
without force, then dispatch existing CI/required with optional source-only build and Research smoke
exactly once each on that new SHA. The prior failed candidate must not be rerun. Round2 receipts
use a separate private directory and exclusive intent files. Source readiness is not hosted or
runtime acceptance: exact19/23/6 named gates, Web checks, authentic artifacts and the separately
bounded approved-input connection/owner-isolation proof remain required.

### G3 second hosted result and scoped correction (2026-10-25)

Candidate `1fadf3b176757516e9475308d79d2c2915166e62` ran CI `40122686601` and
Research smoke `40122689146`, each attempt1 and once. CI completed six failed jobs including
required, three successful jobs and172 recorded steps. Format, Python and policy passed.
Smoke completed successfully with functional PASS; reported project container/volume/network
and context-image absence is distinct from unverified private-builder absence. Cache saves do
not establish persistence or performance. Both artifact listings were empty. Required19 source,
23 PostgreSQL and6 proof outcomes remain unevidenced; no approved-input runtime was attempted.

WP-7 reached actual idle/activeTurn null/pending0. Coordinator recovery matched68 candidate,
97 protected and872 compile inputs and preserved126 evidence files. Report SHA-256
`d5c079891941f6d1b67718357bc6bea3de97ec421cce69812e31c9a50c0ec870` is CHANGES_REQUIRED.
The new private freeze is `target/r4-connection/hosted-correction-round2-preparation/recovered-freeze.json`
(SHA-256 `32cf82b80cdc317f72ba97b2c519b22e34e6ac4c7e56a0e8df753bf80c23f608`).
Original failed reviews, prior candidates and all receipts remain intact.

Direct diagnostics and source inspection constrain the correction to five files:
- WP-3B owns the two test files: use the existing boxed TestResult error for tx_a, test generic
  claim absence with is_none, and match the existing0063 rollback runs-table OR EXISTS token.
  Actor/RLS/atomicity/lease checks, both SQL guards, migration SQL and all test names stay fixed.
- WP-5 owns one expectation: normalized negative-zero KRW is `₩0.00`, matching the formatter's
  existing prefix. Product formatting and the other assertions remain unchanged.
- Coordinator owns four function-local result_large_err allowances using the repository's
  Response-helper convention and the runner's equivalent is_multiple_of parity predicate.

These are new exposed failure causes after the first correction, not blind retries. No local
Rust compilation/Clippy/tests, DB, Web typecheck/unit/build/browser or runtime proof is allowed.
Use source inspection, permitted formatting/lint and whitespace checks, recover the worker
reports/hashes, then freeze the integrated delta for the existing independent reviewer. No
structural edge or currently cited line is changed by these five-file corrections; diagram
sources and PNGs stay intact. After source acceptance, a new clean candidate must pass the
existing same-SHA CI/required/source build and smoke before authenticated artifacts and the
separately bounded approved-input connection proof. R4-04 isolation and R4-05 remain incomplete.

The bounded correction is integrated. WP-3B and WP-5 are idle with no active turn or pending
permission. Coordinator inspection confirmed the exact assigned edits, unchanged test names
and all 126 preserved evidence hashes. Rust formatting, Web workspace lint and documentation
fast checks passed. The existing independent reviewer will review this frozen seven-path delta
(five code/test files plus roadmap and plan); hosted and runtime acceptance remain pending.

### G3 second correction source acceptance (2026-10-25)

The independent reviewer returned READY_FOR_HOSTED with no mandatory source item remaining.
Coordinator recovery verified actual idle, activeTurn null and pending0, all 68 source,
139 protected-evidence and 872 compile-input hashes, unchanged HEAD/index and private modes.
The report SHA-256 is `15f095b09fca47c7c8c9cc052969da10713679a8af39a2f47131d3cc9b233210`;
receipt SHA-256 is `8a69708e6cefbeed2fef050adb67e4b94e18db3258b5adf39664779ae360dab8`.
Both are under `target/r4-connection/wp-6/hosted-round2-correction/`; coordinator recovery is
`target/r4-connection/coordinator-hosted-round2-correction-acceptance.json`.

Only roadmap/plan acceptance records change after that review; all five corrected code/test
files, unchanged diagrams and compile inputs remain exactly reviewed. The target is the new
commit containing this acceptance record, not the failed prior candidate. Run existing
CI/required with source-only build and Research smoke once each at that new SHA, recovering
the exact named results and artifact identities. No hosted or runtime acceptance is implied.
Original failure evidence, approved-input boundaries and actual owner isolation requirements
remain intact. Do not rerun either failed candidate.


### G3 third hosted failure and bounded correction (2026-10-25)

Candidate `5c20f5b4cd9aa2c8256b15ddb252c4ea5d97bc3f` was dispatched once per workflow:
CI40128291578 attempt1 and its required aggregate failed; Research smoke40128294217 attempt1
succeeded. Both terminal artifact listings are empty. The pinned build parser confirmed the19
source-only cases; the workspace harness directly reported all23 frozen PostgreSQL names once,
22FAILED/1OK. The separate PostgreSQL job contained none of those23 names. Six proof cases were
not reached because of a duplicated attribute. Web OpenAPI/lint/typecheck,539Vitest cases and
build passed; the separate browser harness failed with coverage_incomplete. Smoke confirms
functional PASS, four restore/save markers and two serialized exports; independent private-builder
absence and cache persistence remain unverified.

WP7's immutable terminal report is `target/r4-connection/wp-7/round-3/observer-continuation/report.md`
(SHA256 `2a4f6b87d0e077a8b6ef6df994da1e812aadf82bbb963892e27feee2102dbfcf`).
The separately recovered cause report identifies14 storage SQLSTATE42501 and6 API InputInvalid;
two R4 causes remain unknown due to interleaved log blocks. These categories do not establish
which database object caused42501. WP1's source report (`42ed091c5b7b2db41b724d264376f0602a7a44466570c6152f3d19b4f9c168df`)
confirms four malformed contract pins and two standalone scratch builders using the wrong migration
role. Eleven other failing targets already use canonical migration_owner preparation. Do not infer
that these two fixes resolve all failures. Original reports and227 protected files remain frozen.

Bounded ownership for this correction branch:

| Owner | Exact write scope | Work / permitted checks |
| --- | --- | --- |
| WP3B, Luna/max | 0063 up SQL; API tests/common; queue_contract; result-model robustness_harness; owner_beta_simulation_execution test; scripts/ci/test_ci_contract.py | Two SQL literals, two positive fixture constants, two standalone migration-only owner pools, unused import/local tuple alias, exact contract parity regression. Owned rustfmt/Python contracts only; no DB/compile. |
| WP5, Luna/max | scripts/qa/candidate-web-e2e-runner.py and its test file | Include existing22 simulation browser cases in strict off/full inventories and regression fixtures. Bounded mocked Python tests; no browser execution. |
| WP1, 6.1-sol/high | New private hosted-round3-cause-audit report/receipt only | Analyze committed candidate's42501 and generic22P02/actor paths against recovered safe causes; no speculative production patch. |
| Coordinator | Proof example's duplicate outer dead_code attribute; this plan and ROADMAP | Small integration and source/protected-evidence verification; review remaining hypotheses before assigning any trigger correction. |

Assignments with starting hashes and non-overlapping scopes are under each owner's
`hosted-round3-correction/assignment.md` (WP1 uses `hosted-round3-cause-audit`). No existing tests,
strict inventory validation, FORCE RLS, atomic pair constraints, or required hosted jobs may be
weakened. The approved contract/calculation, production, provider, input and Daily boundaries remain.
After source corrections and independent review, freeze a new candidate for existing hosted gates;
never rerun the failed candidate blindly. Authenticated artifacts and actual owner/API isolation
remain mandatory later gates. R4-05 stays DOING and R4-04 full isolation stays blocked.


#### Third correction recovery and fixture repair (2026-10-25)

WP3B's six-file correction and WP5's two-file inventory correction were recovered after
actual idle with no pending permissions. The coordinator verified all227 protected hashes,
HEAD/index, source hashes and private modes. Existing system Python (with PyYAML) passed the
new contract parity test; the approved stdlib Web supervisor suite passed36 tests in22.626s.
These are local contract/harness checks, not browser or PostgreSQL evidence.

WP1's follow-up source audit (report SHA256
`f8853b7630df585dd1f3c5872bcc1febd2fccf96e5b6d5690c1094dc9e378fb4`) found two concrete
fixture privilege contradictions. The coordinator owns the one-file repair in
`crates/job-queue/tests/owner_beta_simulation_execution.rs`: seed explicitly selected config
IDs through the existing privileged fixture pool, preserving the nil-ID case; assert the
real app role and owner plus config visibility before app jobs/run writes; retain denied
app config-ID INSERT and worker jobs INSERT privileges; exercise allowed app config creation
and explicit-ID denial. Seed the ordinary generic job through the fixture queue, retaining
worker claim/settle. No production grants or policy are changed by this fixture correction.
All15 existing storage test names remain. Rustfmt and whitespace checks pass; DB execution awaits
hosted validation.

The same audit proves0063 jobs lineage changes caller actor without restoring it. Its proposed
empty-GUC explanation for three generic22P02 errors remains a hypothesis. Existing WP6 owns
only `target/r4-connection/wp-6/actor-guard-design-review/` to assess a bounded restoration and
null-safe migration-owner jobs policy design, including down migration and behavioral regression
requirements. It may not modify source or run DB/hosted checks. Coordinator fixture/docs writes
and WP6 report writes do not overlap. Implementation of the actor repair follows this review;
no sentinel actor, widened grant, weakened RLS, skipped test, or production action is authorized.

### Actor repair implementation scope (2026-10-25)

WP2's bounded correction review found no additional defect; it excludes actor repair and does
not declare overall hosted readiness. WP6's design report SHA256
`55fcf1eb94c9794cc75cb65b37254b6291898fdcc77061b6893dc04fa33bcfca` was recovered after actual
idle with all five pins and227 protected hashes matching. Adopt scoped restoration in0063,
normalization of only the migration-owner jobs policy, and OLD/NEW attempt-parent discovery
before any generic early return. Keep all existing immutable binding and publication fences.

The coordinator confirmed the older0044 scheduler control is seeded even when inactive.
Its jobs/attempts deferred callback changes actor independently of0063, so it is part of the
standard R4 scratch path. In0063 up, replace only that callback's body to snapshot and restore
the caller actor on normal exits; preserve its settlement checks, ownership, settings and ACL.
Down restores the original0044 definition after the existing history refusal. Historical0044
bytes and inactive control remain unchanged. Do not remove the control to make tests pass.

Existing WP3B owns0063 up/down and the migration-contract assertions. Existing WP3C owns only
the storage suite's behavioral helpers/assertions: same-connection actor preservation,
attempt-first recovery, both directions of parent reassignment, error/savepoint restoration,
deferred callbacks and policy/function roundtrip. Existing15/69 names remain fixed. Their write
scopes do not overlap. The private shared specification and252-entry evidence freeze are in
`target/r4-connection/hosted-round3-correction-preparation/`; coordinator owns documentation and
diagram citations. No local heavy execution is added. Independent source review, a new candidate's
hosted checks and authenticated artifacts still precede the approved-input isolation proof.

### Actor repair implementation recovery (2026-10-25)

Both owned implementation workers are actually idle with no pending permissions. The three SQL/static-test outputs and the behavioral test report were recovered with all252 protected and six read-only source hashes intact; HEAD remains5c20f5b and the index is unchanged. Existing15 storage and69 migration-contract test names remain intact. The original worker reports remain immutable.

The coordinator corrected the behavioral test's default candidate service UUID to the exact0044 seed, removed its redundant inactive-state mutation, bound pool actor assertions to the same backend PID, and executed both attempt-parent reassignment denials as the real worker role. The callback's seeded inactive control remains present. The current patch covers scoped0063 restoration, the0044 callback replacement and exact down definition, null-safe migration-owner jobs policy, OLD/NEW parent discovery, deferred constraints and savepoint restoration. These are source changes awaiting independent review and hosted PostgreSQL execution, not established isolation outcomes.

Pinned Rust formatting and owned whitespace checks passed; no Rust compile, Clippy, SQL/database or browser check was run locally. Diagram evidence citations are updated to the settled SQL locations and rendered locally. Next: independent source review, a new frozen candidate and each existing required CI/build and smoke dispatch once; only successful hosted gates and authenticated exact4+2 artifacts unlock the separately bounded approved-input connection proof. Prior candidate runs must not be rerun.

### Third correction source acceptance (2026-10-25)

The independent reviewer returned READY_FOR_HOSTED with no mandatory source finding. Coordinator confirmed actual idle, activeTurn null, pending permissions empty, report SHA256 db7b5acc7bed9ced9445f28e3ccbb5d25a9367f1e31e1e609f1da6a1ca979b0c, receipt SHA256 f0b21c34629ff17a3e799214a0c786fb324bde7b5ad0a7e2311d6ac9feb499f5 and all71 source/265 protected/872 compile inputs. The original review and earlier failures remain immutable. Only this plan and roadmap acceptance text change after review.

The source verdict does not establish compilation or PostgreSQL behavior. Post-COMMIT actor checks alone cannot reveal transaction-local leakage; direct assertions inside transactions cover their specified paths, while other exit restoration conclusions remain source-based. Two unidentified old R4 failures and other unrecovered target causes remain unresolved. All existing hosted gates remain required.

Create a new candidate containing this record and the reviewed corrections, verify committed and working blobs, push the same branch, and dispatch existing CI/required with selected source-only build and Research smoke once each. Do not rerun any of the three failed candidate SHAs. Only successful same-candidate required gates permit authenticating exact4+2 artifacts before the separately frozen approved-input scratch connection proof. R4-04 full isolation and R4-05 completion remain unaccepted.

### G3 fourth hosted evidence and bounded diagnosis (2026-10-25)

Candidate `9c3af801b44357eea96a7ec6452aa9cade6a2b25` ran CI40137757186 and
Research smoke40137759181 once each, attempt1. CI and required failed; smoke passed.
WP7 actual idle/pending0, report `1355e5df5bf14fa0d96caf4deea63fb75fd5780b3d93d49f302e996a2ce45d50`
and receipt `0c558987ee69bf8d42d2ec53e7a7346147ebc62e4adf79d22ca3e6b2544fde9f`
were recovered with71 source/271 protected/872 compile/17 observer inputs intact.
The monitor is reaped and both terminal artifact lists are empty. Old candidates and all
round-four evidence remain immutable; no rerun or artifact authentication is authorized here.

Source19 passed both the pinned build parser and separate workspace direct rows.
Dedicated PG ran15 frozen storage cases:7 passed,8 failed, then8 API cases were not reached.
Workspace ran all23 PG-scope cases:14 passed,9 failed. These harnesses remain distinct.
Web unit539 passed; E2E coverage is complete with165 executions/164 unique identities and
5 unexpected off-lane cases. The optional build was cancelled after source markers;
proof6 has no execution proof. A45-minute source limit is not proof of the cancellation cause.
Smoke static/functional markers and project-resource absence checks passed. Two cache exports
completed, but their direct service identities and independent builder absence were not recovered.
Cache persistence/performance and host OOM/resource metrics remain unverified.

The coordinator corrects only the direct Clippy bool-assertion diagnostic at
`crates/api-server/tests/openapi_contract.rs:227`, preserving its condition and message.
Existing owned idle WP1 and WP2 separately trace the DB/actor failures and Web5 failures
read-only. Because22P02 and some actor-related assertions survived the preceding correction,
prior actor-restoration assumptions must be tested against the new exact source locations;
do not extend that theory without a complete causal chain. The build cancellation branch is
separate coordinator analysis. No local heavy compile, database, Docker or browser execution;
no new remote reads, input access, production changes or new workers. Source implementations
wait for verified bounded causes and retain all acceptance fences and existing test names.
Only after corrections and independent source review may a new candidate be frozen and the
existing required CI/optional build plus smoke dispatched once. Actual authenticated owner
isolation remains blocked on successful hosted gates and authenticated4+2 deliverables.


### G3 fourth hosted correction — implementation integrated, source review pending (2026-10-25)

The coordinator recovered both diagnoses only after actual idle, no pending requests,
matching report receipts and all current71 source/338 protected/872 compile pins.
WP1 report SHA256 is `d15a2259afd8b93d24b42a1b0b0bfe8c2d65d671ba79421b19886db00bcf4654`;
WP2 report SHA256 is `e110973b7afb74d0e6c875d7c6bfff03f0d0c8eba8be43a0f6a1159a818a210f`.
Their private paths are `target/r4-connection/wp-1/hosted-round4-diagnosis/` and
`target/r4-connection/wp-2/hosted-round4-web-diagnosis/`.

- Storage/API fixture corrections preserve the existing named tests and acceptance:
  valid synthetic hexadecimal identities, explicit inactive scratch candidate control
  with the seeded row/callback retained, lease-relative expiry with a live negative
  control, distinct malformed-caller versus stored-pair reconciliation expectations,
  and exact per-request correlation IDs. No production control or historical migration
  changes are authorized by these test arrangements.
- Web corrections target the full cancellation acknowledgement, the actual named
  combobox, and POST-only injected errors with observed response status/code/count.
  The prior five unexpected cases remain historical failures; the two enqueue cases'
  actual intercepted request/order was not retained. No product auth, abort, rights,
  eligibility or cancellation semantics are changed to accommodate the tests.
- The prior22P02 inner statement is still unconfirmed. Source inspection finds a
  separate unsafe evaluation-order dependency: the new pair policy's AND expression
  joins jobs, whose historical app policy casts empty actor text to UUID. PostgreSQL
  does not guarantee AND ordering and documents CASE as an evaluation guard, subject
  to constant-folding/aggregate caveats. The R4 policy now returns false for missing
  or empty actor before entering the existing owner/run/jobs predicates. No predicate,
  role, grant or historical app policy is removed or broadened. Same-connection
  missing/empty/valid/malformed actor behavior and phase-labelled failures must be
  checked in hosted PostgreSQL; source reasoning is not a reproduction of the old
  failure. References: [evaluation rules](https://www.postgresql.org/docs/17/sql-expressions.html#SYNTAX-EXPRESS-EVAL)
  and [CASE](https://www.postgresql.org/docs/17/functions-conditional.html#FUNCTIONS-CASE).
- A separate bounded read of check-run121002800665 annotations now confirms timeout.
  The raw bytes were not retained; their digest and safe classification are in
  `target/r4-connection/hosted-round4-correction-preparation/build-annotation-evidence.json`.
  The original worker report's earlier uncertainty is preserved. Exact active compiler
  phase, OOM and resource metrics remain unknown. Only this source-build job's limit
  changes45→90 minutes. All serial release commands, CARGO_BUILD_JOBS=2, permission,
  source/proof parser gates, required aggregation and success-only exact4+2 uploads
  remain unchanged. No old candidate run is retried.

Coordinator checks so far: pinned rustfmt1.97.1/edition2024 on its Rust changes,
Python AST, TypeScript syntax via Node24 strip-types, and three focused existing
workflow contract tests passed. The existing Biome2.5.7 binary from the same repository
worktree family was reused with cwd=apps/web; one formatting correction was applied and
the focused check passed. No dependency install, Web build/browser, Rust compile or
database check was run. These checks do
not establish hosted success. WP1 changed only its two bounded test files; coordinator
owns Web/policy/budget and documentation. Independent source review and a new clean
candidate remain prerequisites for any later dispatch. G3 runtime/artifact gates and
all previous approval boundaries stay unchanged.

The final WP1 correction report was recovered after actual idle/pending0, with SHA256
`1226e0dc62771a1784a4644417d0e514cc3d6fb05d30837439c6d24f038bca36` and receipt
`76c6aafe4f6887a6b0b8a12fe815d0e8a8858424869703e1455b34915300e380` under
`target/r4-connection/wp-1/hosted-round4-correction/`. All57 readonly source,
867 readonly compile and345 protected evidence hashes matched. The worker retained
the original15 storage/23 aggregate PostgreSQL names and corrected the later live-cancel
expiry as well. Coordinator review then corrected two remaining expiry arrangements:
the30-second typed-claim generic-sweeper exclusion and60-second frozen-config
cancellation now use the same lease-derived helper and assert real eligibility before
their unchanged expected outcomes. The two10-second arrangements for1-second generic
leases remain valid. Pinned rustfmt, whitespace and original storage-name checks pass;
no compile/database/browser run has occurred.

Index preservation exception: baseline bytes hashed to
`811eb11b9729a09e29a7b9c0050de58c8418293250e893f8543443fc6836ae54`; observed bytes hash to
`ce82be142240f011190b880693726fd0add089838c5a9ea70db004d034fd43b3`. HEAD is unchanged and
all2254 stage0 path/mode/blob entries equal HEAD, with no staged diff. An automatic
stat refresh is plausible but was not observed conclusively. No index reset or
reconstruction was performed. The original baseline and worker report remain immutable;
`coordinator-index-byte-deviation.json` records the exception, and the independent review
must pin the current bytes without claiming the old byte-preservation check passed.

### G3 fourth correction review accepted — new hosted candidate next (2026-10-25)

The coordinator recovered WP6 at actual idle with no pending permissions and accepted
READY_FOR_HOSTED for source review only. The report SHA256 is
`d960f15f4eee67e549c3a50b8c354e46dfb5c0621f89c763e9927437d0f54823`; receipt SHA256 is
`0d754bc211344153f0c784d3c25917f9f5ed89d210cb7d7153d4ffda8d5d725c`, under
`target/r4-connection/wp-6/hosted-round4-correction/`. All71 source/document/image,
361 protected evidence and872 compile-input hashes matched independently. The review
found no mandatory source correction. HEAD/branch/current index remained pinned during
review; the earlier index-byte exception remains documented, with no repair or old-byte
preservation claim. The two recorded coordinator lease corrections were independently
checked against the saved worker-final source.

Only this plan and ROADMAP acceptance text change after review. The next step is a new
clean committed candidate and exactly one existing CI dispatch with selected source-only
build plus one Research smoke dispatch at the same SHA. No previous candidate run is
retried. The source verdict does not establish compilation, PostgreSQL/browser behavior,
proof6, authenticated delivery or approved-input owner isolation. Historical unknowns
remain unknown. Required hosted success and coordinator authentication of the exact4+2
archives still precede the isolated real authenticated API/runner proof and mandatory
cross-user denial/failed-partial-pair nonvisibility checks. Scope and input permissions
remain unchanged.


### G3 fifth candidate terminal evidence — remaining actor lookup failure (2026-10-25)

Candidate `7f80c4db6ef0f7de722302553b2c38768c074fe3` ran CI40144306388 and
Research smoke40144308241 once each, attempt1/workflow_dispatch. CI and required
failed; the selected R4 build and smoke succeeded. WP7 is actual idle/pending0.
The report SHA256 is `087f6907ccd069ecbf7a68e9fab6c7a01222c9f8e8e3eb41b7a6a173750a0700`;
receipt SHA256 is `25dfddcd764c4b73cf8f9c15ae91d6310634451e927d8719593439d892e999e6`.
The coordinator checked71 source,368 protected,872 compile,21 observer inputs,
907 committed blobs,40 output hashes and the unchanged candidate index. Recovery:
`target/r4-connection/hosted-round5-correction-preparation/wp7-recovery.json`.

Source19 and proof6 passed the pinned strict parser; source19 also passed direct
workspace rows. Web unit539 and all165 E2E executions/164 unique identities passed.
Dedicated PG ran15 storage cases:13 passed and2 failed, then8 API/replay cases were
not reached. Workspace ran all23:21 passed and2 failed. The fixed phase marker
identifies `empty_pair_select` with SQLSTATE22P02 on the same app backend, after
missing and empty actor classifications. The success-pair test also failed22P02,
but its exact inner statement is not retained. Do not infer successful isolation.

The previous R4 pair CASE guard did not remove the observed empty-actor failure.
Re-plan only that diagnosis branch: existing WP6 performs a read-only trace of
pair policy, referenced run/jobs RLS and test phases, identifying competing causes,
a minimal in-scope correction or the precise missing evidence. No speculative SQL
edit, historical migration change, global app-policy change, actor mutation, role
expansion or removal of owner/run/job/status/partial-pair fences is authorized by
this diagnosis assignment. The coordinator removes the independently confirmed
unused `mut` at API test line335; assertions and behavior stay unchanged.

Both exact artifact metadata records exist, but no archive was downloaded or
authenticated. Required CI success remains mandatory before exact4+2 archive
verification and the separately frozen approved-input G3 execution. Smoke cache
exports and project-resource absence are evidenced; independent builder absence,
cache persistence/performance and host resource metrics remain unavailable.
All old runs/reports remain immutable; never rerun the five consumed candidates.


### 2026-10-25 round5 diagnosis accepted; bounded pair-read correction

WP6 actual idle and report `dc95f2b4ccb9c190b93f39158bad3c5543f81dcfa9a660b3521d7f31027e9a4e`, receipt `0f52ec9a36f59f2992992bfa17428e5846f4821e64ba7c8e87ed7ef8ba610bbc`, and71 source/872 compile/478 protected hashes were verified. The same-backend empty-actor lookup failure is confirmed; nested jobs actor-cast evaluation stage and the second failing statement remain unconfirmed. The diagnosis is not runtime acceptance.

Within the authorized R4-05 implementation, add only `public.owner_beta_simulation_pair_read_allowed(uuid, uuid)` as a PL/pgSQL STABLE SECURITY INVOKER, CALLED ON NULL INPUT, PARALLEL UNSAFE function with `search_path=pg_catalog`. Return false for a missing/empty actor before reaching a separate nested query. Cast every nonempty actor before argument/owner shortcuts so malformed values retain22P02. Preserve run ID, both owners, successful run/job status, dedicated job type and existing RLS. Do not query the pair table recursively. The app pair policy calls this helper with its row's run/owner IDs.

This implementation scope explicitly includes app-only EXECUTE on the new function, with PUBLIC and other runtime roles denied. It adds no table grant, role membership, definer authority, actor mutation or generic jobs app-policy change. Preserve historical0010/0044/0060/0061 bytes,0063 writer/lease/retag/OLD+NEW-parent guards, candidate control and Daily OFF. Down retains its populated-history refusal, drops the dependent pair policy before the helper, and restores the existing callback/jobs policies unchanged.

Existing WP3B owns0063 up/down and migration-contract assertions. Existing WP3C owns only the storage behavior test. Preserve15 storage/69 migration/23 PG/8 Web/22 browser names. Extend same-backend cold/warm missing/empty/valid/malformed reads, direct-helper and ACL checks, and down/up definition/config/ACL/dependency snapshots. Add fixed phase labels to the success-pair test without weakening atomicity or absence assertions; do not guess a writer fix. Local verification is limited to pinned rustfmt, diff and source/static checks; compile/DB/browser remain hosted. Coordinator owns documentation/diagrams and independent final review before any new candidate. No consumed candidate rerun, archive download or G3 execution is authorized by these edits.

The procedural boundary follows PostgreSQL's documented first-reached SQL preparation behavior, not an observed explanation of the old failure: [PL/pgSQL plan caching](https://www.postgresql.org/docs/16/plpgsql-implementation.html#PLPGSQL-PLAN-CACHING). Function authority and null-input attributes follow [CREATE FUNCTION](https://www.postgresql.org/docs/16/sql-createfunction.html). Runtime regressions must still prove the new candidate.


### 2026-10-25 pair-read source correction integration

Both owned workers reached actual idle with no pending requests. WP3B report SHA256 `290867809c6fe8dbbda77731e452d2db6b9866324c84c459aa462f5d996c4388`, receipt `f2285e1858cfb8a248b76e2b75aac276bbebac8fa433ae975fb78ed0b99fc90a`; WP3C report `65251e478b617939940da06092ee4576148c0e607af9d939217cb846f8cc256f`, receipt `e1bfc1f797505c0e497bec6397051d6b4deac28e4844a4b074ef177f293273be` were recovered with67 readonly source/868 compile/485 protected hashes. Old reports and the WP3C final source snapshot are preserved under the private correction-preparation directory.

The coordinator checked the actual diffs and corrected two `SimulationPublicationError` results passed to the boxed-error phase wrapper, named the large catalog row type, and separated expected rejection markers (`R4_EXPECTED_SQLSTATE`) from failing phase markers (`R4_FIXTURE_PHASE`). These are source corrections, not compiler-observed outcomes. Initial local name extraction counted only19 synchronous migration tests; including tokio tests confirmed all69 unchanged, alongside15 storage tests. Pinned formatting and whitespace checks pass. No compile/SQL/DB/browser run occurred.

The function still uses caller RLS and full publication predicates, with no writer or historical policy change. Same-backend regression source now covers cold/warm missing/empty/valid/malformed states, direct helper null/owner cases, ACL/catalog dependency and empty down/up equivalence. Fixed success-pair phases preserve atomic/partial invisibility checks and distinguish the still-unlocalized second failure if it recurs. Expected malformed/immutable denials must not be counted as test failures by future evidence extraction. Current actor markers carry a fixed phase and classification; the future observer must support this format without printing raw values.

Diagram evidence is updated for the new app-callable invoker predicate and shifted0063 deferred-trigger lines. After local PNG rendering and documentation checks, existing WP6 independently reviews the combined exact source before a new candidate. Preserve all original required jobs,19source/23PG/6proof and Web inventories, same-candidate gates and the archive/G3 block until hosted success.


### 2026-10-25 pair-read independent source review accepted

WP6 reached actual idle with no pending request. Its report SHA256 `fe6ad26c1175f3a4071e6c784a89430d59228ef3ee127b1872770bdbb8e878e4` and receipt `b2961072908aa221fb3cc00f034e75a3823b273db6476eaee82e5a9be46514c1` were recovered and matched, alongside71 source,872 compile-input and506 protected hashes, unchanged HEAD/index and eleven authorized dirty paths. All eleven owned workers are idle. The verdict is **READY_FOR_HOSTED — SOURCE REVIEW ONLY**, with no mandatory source finding. Only this plan and ROADMAP acceptance text are added after that review.

The local diagram renderer exited0 without OOM and the owned container was independently absent; docs-fast `target/dev-check/20261025T012649Z-01b05cac509f/run.json` passed. The next observer's offline phase parser recognizes the four fixed actor classifications and separates the three exact expected-denial phase/code pairs from actual fixture-phase failures. Unknown phases, suffixes and secret sentinels are rejected. Those parser checks are not test outcomes or runtime isolation evidence.

Create a new clean committed/pushed candidate, then dispatch the existing CI with optional source-only build and Research smoke once each. Preserve all prior reports and never rerun consumed candidate SHAs. Compilation/SQLx/PLpgSQL/DB behavior and whether both old failures are fixed remain unverified; exact early-evaluation and second-statement causes remain unknown. All required hosted gates must succeed before coordinator authentication of exact4+2 archives and the separately frozen approved-input scratchDB/authenticated API/runner proof. Owner isolation and failed/partial-pair nonvisibility remain unaccepted.

### 2026-10-25 round-six terminal evidence and bounded diagnosis

Candidate `c75700e57bb39e0803a4f8fc801f9dac4546b9d2` was dispatched once to CI40151933028 and smoke40151934711, attempt1. WP7 reached actual idle without pending requests; report SHA256 `8a3bb03f8f08817c832638e0e2deb713c060ee5cfb70a965e45297bb2f12b135` and receipt `1d704a64ba5bbebdf6df383592427bc47c0ede2db5939151dd900bec0d84d4bf` match. Coordinator independently verified71 source,516 protected,872 compile and29 observer inputs,907 committed blobs, unchanged index and clean candidate before corrections.

CI and required failed; the R4 build and smoke passed. Source19/proof6 strict-parser markers and Web539 unit/165 E2E executions passed. Dedicated PostgreSQL ran14 passing and1 failing R4 storage cases, then did not reach8 API/replay cases. Workspace ran all23 with22 passing and1 failing, plus a separate `background_health_heartbeat_advances_during_long_work` failure. The remaining R4 failure is `assert_raw_actor_in_transaction` at storage test line130; its exact invocation and comparison values were suppressed by the safe collector. Expected malformed-actor22P02 denials are separate successful regression observations, not failure causes. Two CI artifact metadata records exist; no archive was downloaded or authenticated.

The coordinator removes only the unused second tuple binding at `owner_beta_simulation_execution_contract.rs:359`, retaining every assertion. Existing idle WP1 diagnoses the remaining actor assertion path and existing idle WP6 diagnoses the heartbeat test's scheduling/completion assumptions. Both tasks are read-only, preserve the frozen source/evidence and write separate private reports. No SQL, runtime timing behavior or assertion is changed on a guessed cause. No new worker, mode change, network recollection, local compile/DB/browser, production action or consumed-candidate rerun is allowed. Further bounded corrections require source evidence and independent review before a new candidate. Required hosted success remains a prerequisite for exact4+2 archive authentication and the approved-input G3 proof.

### 2026-10-25 round-six diagnoses accepted and bounded repairs underway

WP1 actor report SHA256 eb06f58cbb1ee2f4edfcf0d27367a6b2ebbb6a2f041c02fe7a44892d5d001e32 and WP6 heartbeat report661a9879e7b9e98fed96c93e2e3edd3e1220d773ae5a93ec8415d9542eee3bc8 were recovered after actual idle, no pending requests, and independent71 source/872 compile/587 protected hash checks. Both are source diagnoses with runtime attribution limits: the preserved actor failure does not identify its exact caller, and heartbeat scheduling/write/clock timing is unobserved.

The existing0035 terminal recommendation callback writes the actor unconditionally before recommendation-only branches. The bounded0063 repair adds only an early return for owner_beta_price_only_simulation; non-R4 statements, function owner/ACL/security/search_path and trigger attachment remain unchanged. Down restores the exact0035 definition after the nonempty-history refusal. Historical0035/0044/0060 files and existing pair-read/lineage/lease/cancellation/publication fences remain frozen. Existing WP3B owns the two0063 files and migration assertions; WP3C owns only storage behavioral/catalog regressions. Their shared specification is target/r4-connection/hosted-round6-correction-preparation/terminal-actor-repair-spec.md and the stage freeze excludes both workers' and coordinator's mutable paths from read-only groups.

The coordinator changed only the existing heartbeat test: a sentinel observes a completed periodic write while an on-stack work future stays pending, with a five-second progress deadline. Closing the watch sender allows the serial awaited blocking write to drain; assertions run after cleanup and preserve strict timestamp advance and one-second freshness. Cleanup cannot convert an observation timeout to success. If completion is indeterminate, the private fixture is retained rather than deleted under a possible live writer. This does not claim aborting the parent cancels blocking I/O. Production cadence, health reader/writer code and dependencies are unchanged. Pinned rustfmt and diff checks pass; compile/runtime remain unverified.

New safe actor equality markers use six closed phases and match/mismatch only; the next observer must validate them offline and keep expected SQLSTATE denials separate from actual failed test rows. Preserve all15 storage/69 migration/23 PG/8 Web/source19/proof6 names. Integrate and independently review actual source before freezing a new candidate. No consumed rerun, archive authentication, approved-input/G3, production or provider action is part of this correction.

WP3B subsequently reached actual idle with no pending requests. Its report73a34a9cd2d7e738a4fc018b8870e9dff8c44cc3a0cbc35736f72d4227429321 and receipt305298bb9c977abbc6a7c48633ffc0a3fa85d7dd342545c4ca0c9d55d4e356d7 match. Coordinator independently compared the actual SQL definitions: removing exactly one early R4 guard recovers0035 byte-for-byte, and down restores0035 after the history refusal. All69 migration names and61 readonly source/867 compile/599 protected hashes match; HEAD/index/staging are preserved. This is bounded source recovery, with integrated review and runtime acceptance pending.

The inserted callback shifts0063 citations by39 lines. Both existing diagram sources now cite the current ranges; local PNG rendering recorded exit0, no OOM and independent owned-container absence in target/r4-connection/diagrams-20261025T031541Z/receipt.json. Diagram edges and deployment claims are unchanged. The prospective phase parser's offline receipt recognizes twelve actor equality cases, rejects five invalid/suffixed records and keeps three expected SQLSTATE denials separate; it is not a hosted test result. WP3C remains the only active implementation worker.

### 2026-10-25 round-six corrections integrated for independent source review

WP3C reached actual idle with no pending permission. Its original report SHA256 `82e73a11a15b8132173f8f718e674a81b00574e5e0fdf1c4d9bec7b4dad87a24`, receipt `9a767072c04d90350829a21209bf6d09215aad489cae36c3a26001d8d4ddf9b0`, and source snapshot `5a7fffb724b815d2d452b5ce6d5ca8548a1abb7b3a7c4881e94753c10cdc4797` are preserved. The coordinator verified all61 read-only source,867 compile and599 protected hashes before integration, with unchanged HEAD/index/staging.

Direct review found five bounded test corrections: remove an extra newline from the expected callback body; compare the optional pair-helper refusal snapshot with an optional reference; use the pinned Rust optional-PID predicate without an unnecessary map; preserve logical absence only for a freshly missing custom actor setting restored as empty by the existing BEFORE lineage guard; and explicitly cast catalog booleans to text for the intended ACL strings. Explicit empty and valid actor settings still require exact equality on the same backend. The original missing-state assertion, visibility/atomicity assertions, SQL authority and production behavior remain intact. This resolves the shared spec's raw-absence wording conflict without treating any nonempty value as absent.

The exact worker-to-coordinator patch and source checks are recorded in `target/r4-connection/hosted-round6-correction-preparation/coordinator-terminal-actor-integration.json`, SHA256 `726ec77c7dc7579aac8741c73d0c3640c0a9170ea60a160943bce3894373f246`. Current storage source SHA256 is `2e02141becb9ea07a73631878b5071f524ef6d483289f4fad767e08ad1be0fa7`. Pinned rustfmt and diff checks passed; all30 existing R4 async helper/test names,15 storage tests and69 migration tests match. The constructed callback body equals the actual up migration, and down equals the historical body. Six closed actor phases each appear once; twelve offline equality markers parse without inferring a test outcome. These are source checks, not compilation or database execution.

The next independent WP6 review covers all twelve changed paths, including the coordinator heartbeat test, lint binding and documentation. The source manifest also includes recommendation-runner, bringing reviewed source inputs to72; compile inputs remain872. No source edits are allowed while that review runs. A new clean committed candidate and one existing CI/build/smoke dispatch follow only after the actual-idle review is recovered and accepted. All required hosted gates must succeed before exact4+2 archive authentication and the separately frozen approved-input G3 proof. The consumed six candidates, historical evidence, v3/calculation/first strategy, Daily OFF and R2/R3 deferrals remain preserved.

### 2026-10-25 round-six independent source review accepted

Existing WP6 reached actual idle with activeTurn null and no pending permission. Report SHA256 `e7589007f47da92a2e8029ca405941f4f897010f275e4448a242fa096f0fe816` and receipt `b5f04210cec719e3d8af0831bc718e8a1bd3e33a0ab82763900836db0bb03117` were independently matched. All72 source,872 compile and621 protected hashes, modes, HEAD/branch/index, empty staging and twelve dirty paths match the review manifest. The verdict is READY_FOR_HOSTED — SOURCE REVIEW ONLY, with no mandatory source findings. Coordinator recovery is `target/r4-connection/hosted-round6-correction-preparation/independent-review-recovery.json`, SHA256 `d2c3b93dd7dc9676634b97b7c0d6d76597b1f1804bdc2967b22475d60ca1c0ef`.

After review, only this acceptance record and ROADMAP status are changed. Integrate the pinned prospective actor parser into the new observer and verify its closed marker/expected-denial protocol offline before dispatch. Freeze a new clean committed/pushed candidate, then run the existing CI with selected R4 source build and Research smoke once each. Do not rerun any consumed candidate. Actual PostgreSQL/ACL/actor behavior, heartbeat timing and exceptional cleanup, new hosted checks and authenticated archives remain unverified. A successful source review does not replace every required hosted gate or the subsequent separately frozen approved-input owner-isolation proof.


### 2026-10-25 round-seven hosted acceptance and authenticated delivery

Candidate `d100c81f31dcd4495271e4318a82d28f6cb74aff` was committed and pushed clean on
`review-handoff-next-task`. Existing CI40159023552 and Research smoke40159025262 each ran
once, attempt1, and succeeded. All ten job/check ID, SHA and details-URL joins match; required
aggregation succeeded. The coordinator recovered the actual-idle WP7 report
`b787fbed20b427c0c33f32dd53978855068ae61b2e1a67d5aed95d251114cf89` and receipt
`1bb45af80a954e7f0d5c4b94da58487104c0e3da1599d7c9cce1cebb9ecef8f0`, independently
verified72 source/631 protected/872 compile/34 observer hashes and907 committed blobs, and
confirmed clean candidate HEAD/index. Recovery receipt is
`target/r4-connection/wp-7/round-7/coordinator-final-recovery.json`, SHA256
`62689e5ca89460ed45af9e23e3c4fb120d223e5ac232ca3b58a7aba2c886178c`.

Source19 has both direct workspace rows and strict build-parser PASS; proof6 has strict-parser
PASS. All23 PostgreSQL cases passed separately in dedicated validation and workspace. Web539
unit tests and165 E2E executions over164 unique identities passed, with no skips/retries/flakes
and complete lane coverage. Successful PG harness output suppresses the private actor/denial
markers; their absence is an observation limit, not independent proof of those values. Smoke
static/functional markers and queried project container/volume/network/context-image absence
passed. Two completed BuildKit exports remain unidentified by service. Cache persistence,
performance, builder independent absence and host resource peaks are not inferred.

After every required hosted gate passed, the coordinator fetched current authenticated GitHub
artifact metadata and downloaded exactly two archives. Artifact12586754471 is150432743 bytes,
SHA256 `f1f18a29d786fc8b720143087edcee8023cd04783ed51bb6a5d50dea99909941`;
artifact12586639463 is35300530 bytes, SHA256
`53ec1b900700d3ef01eb81818d73ae725199d2950636846000f4837e1bb6102c`.
The exact original four members and separate two proof members are regular files with expected
modes, no extra members, duplicate paths, symlinks or encrypted entries. Closed receipts bind
all six origin fields to the same SHA/run/attempt and each binary digest. Runner33, proof872 and
verifier3 source maps match committed blobs. All three executable files are static x86-64 ELF
with no interpreter or needed dynamic libraries. Thirteen offline negative/format cases passed.
Authentication receipt `target/r4-connection/artifact-authentication-round7/authentication.json`,
SHA256 `d14f849d4f013aebbb6e49f6778b0ddade120f850d21399bcc8f1eddc0a19d20`, records
this source-only delivery acceptance. No delivered binary or approved input has been executed.

The existing pinned PostgreSQL image was inspected in bounded network-none, nonroot,
read-only containers with no data/input mounts. The first check rejected an overly narrow
command-path expectation; the corrected check confirmed UID/GID999, initdb/postgres under
`/usr/lib/postgresql/18/bin` and pg_isready/psql wrappers under `/usr/bin`. Both containers exited
and were independently confirmed absent; no database was initialized. G3 will use DB999:1000,
API/runner1000:1000 and a private0770 socket directory, suppressing the image's default data
volume with a bounded tmpfs over its entire parent.

The private runtime specification fixes nonce `6e7171c37f024256aa00a10b10b864e3`, exact owned
names/images/resources/deadlines and readonly mounts. Its SHA256 is
`cc786a4f489b3e3e5f5893617fb9e819011a7671936f571189df738d378e0190` at
`target/r4-connection/g3-round7/runtime-spec.json`. Existing WP3E implements only the private
operator and offline stdlib tests; existing WP6 independently reviews authenticated delivery and
this contract. Actual operator review and an immutable execution freeze are still required before
the single approved-input proof. Earlier failed candidates and reports remain immutable; no
reruns, production changes, input copying, provider calls, deployment or R4-06 are authorized.
R4-04 isolation and R4-05 completion remain unaccepted until the actual API/RLS/publication
proof and owned-resource cleanup are recovered.

### 2026-10-25 G3 operator corrections awaiting independent review

The authenticated-delivery/contract review was recovered at actual idle: report
`e2f50250d607301f64bc0fad8878914d43d66849e0251ce81da94d28b194e2aa` and receipt
`37c9e06d924dbae4ef8b5c1b8e935c8b0eb6a64a8062eba8c8f36d4fd67b2dd6` match.
Its acceptance did not cover the later operator implementation. WP6 then reviewed that
implementation and returned CHANGES_REQUIRED with eight mandatory findings; report
`415b2beab96e6f972af4cd8b1c7f6190a1e316b5997a2e938ba0d2d256176bf6`, receipt
`f8d25779fdd46417ee9ff026b10698db020f47006d0519a48d5832a59212e69f` and all907 source/
747 protected hashes were recovered with unchanged candidate/index and two dirty progress docs.

The coordinator corrected only the existing private orchestrate.py and test_orchestrate.py.
The original four worker outputs and review remain preserved. Corrections cover selected JSON
inspect fields and a minimal preexisting snapshot; native boolean SQL guard output; readiness
reads before EOF; cleanup RPCs after signals; registered subprocess ownership and bounded reader
completion; the1260-second outer runner deadline; observed OOM fields; and final receipt/PASS
only after actual slot-context exit. Additional integration fixes distinguish bind mounts from
HostConfig tmpfs, reconcile uncertain creation by role, and wait within the existing startup
deadline. Runtime spec, resource limits, authority, input contract and delivered bytes are unchanged.

All38 offline stdlib tests passed, preserving the original21 names. New cases include a live
pipe held open after readiness, spawn/thread/read failures, repeated signals, uncertain ownership,
cleanup ordering and live readers, slot exit failure, and the full fake controller completing
after1200 but before1260 seconds or failing safely on signal/deadline. They run no Docker/DB/
delivered binary or approved input. A separate read-only selected-field Docker probe accepted
all10 existing containers, including nullable PID fields and absent tmpfs fields. This proves
serialization of those observations, not actual G3 boundaries or cleanup. The correction report
and hash receipt are under `target/r4-connection/g3-round7/coordinator-correction-*`.

Existing idle WP6 must independently review the corrected bytes and the new failure-path
coverage before the coordinator creates the immutable execution freeze. Actual approved-input
API/runner/publication/RLS behavior and complete cleanup remain unverified. Preserve all earlier
hosted attempts and authenticated archives; no redispatch, production action or input copying.

### 2026-10-25 G3 final-drain and deadline acceptance corrections

WP6's first correction review reached actual idle/pending0. Report
`06b9c8b50e7873747205cff20c29cd4d29932ddd3cc5f99d52c31405ea76108e` and receipt
`ade70fd3dddc07fe707b8d367fb8ef9ac417d2674ecf6aa130f30f4ff07f819d` match, with all907
source and765 protected hashes preserved. Its independent38-test run passed, and OP1–OP8 were
resolved. It found two remaining acceptance paths: a DB reader can fail only during final cleanup,
and blocking inspections/joins can cross a deadline after the loop's earlier clock sample.

The coordinator preserved those reviewed Python bytes in correction1-reviewed and corrected only
the same operator/test files. Final stream validity now affects the verdict after every reader has
finished, independently of successful physical cleanup. Readiness and proof acceptance recheck
the monotonic deadline and signal after blocking observations and before accepting success;
cleanup remains supervised beyond deadlines until ownership/absence/reap is resolved.

The suite now passes41 tests. Three added regressions cover late DB read-error/oversized/
unterminated output, same-iteration inspection/join/terminal-inspection/report-read expiry, and
late readiness success. Applying only these new cases to the preserved prior operator produces
eight expected assertion failures and no harness errors. The full fake controller confirms cleanup
and normal slot release remain successful even when the proof verdict correctly fails.
No real G3, Docker mutation, SQL, approved-input or delivered-binary operation occurred. Final
independent source review and immutable execution freeze still precede the single actual proof.

### 2026-10-25 final operator accepted; G3 preflight blocked by host permissions

WP6 reached actual idle with no pending request and returned READY_FOR_EXECUTION_FREEZE_SOURCE_ONLY.
Report `ce02854f1d125c01efd5025d6f0282c4f84f6ea4c616abdce1a08a6629e0f2d1`, receipt
`f6e55665e9594997c89396456c474866d4f1e1f1d4b283d6e75331a0a6aac839` and all907 source/
776 protected hashes matched. Its one permitted suite run passed41 tests; no mandatory source
finding remained. Operator SHA256 is `decc6507c82f0c74c98851ab9760a7a6acd192f4a62899344af365d3671793a6`.
The coordinator copied the accepted report to the fixed review path and created the immutable
execution freeze, SHA256 `2f0a7a02300eff0c99fb9a1229ff1d8c93d52dfd5c084f93b35456c16552fdf4`.

The sole operator invocation exited2 with `path-component-stat` before creating the execution
directory or intent. Exact metadata-only diagnosis identified `/var/lib/lagrange/data` as
root:982/mode0750 and EACCES while lstat attempted its artifacts child under effective UID/GID1000.
The approved root's leaf metadata and content remain unobserved. An exact read-only `sudo -n stat`
probe was denied because interactive authentication is required. No permission or group change,
input copy, alternate path, DB/API/runner start or delivered-binary execution occurred.

Independent read-only Docker queries found no exact owned name or nonce containers. The existing
shared heavy lock was acquired nonblocking and released with its inode preserved, confirming
availability after the exited operator. All905 original readonly-source and732 protected hashes
and the candidate index still match. Receipt `target/r4-connection/g3-round7/coordinator-preflight-block.json`
SHA256 `613f9015ffa5685127fdb23c5078daef1c9b44415daab32412219bb92875cf78` records one operator
invocation and zero actual runtime attempts. No final runtime receipt exists because no runtime
attempt began. Do not misclassify this environment preflight failure as a failed calculation or RLS test.

R4-05 is now BLOCKED on an external host-access prerequisite. An authorized operator must provide
the fixed UID1000 execution context metadata/traversal access to the existing approved root before
an explicit continuation. Host permission repair is outside the current scope. Preserve the exact
freeze, report and failed-invocation receipt; no automatic second invocation, reset, input copying,
workflow rerun or new candidate is authorized by this handoff. After access is resolved, first
reconcile owned agents, exact pins and the still-unstarted runtime state before deciding how to
resume the existing single-proof contract. Actual API/runner/publication/RLS/cleanup acceptance
and R4-04 full isolation remain unverified; prior hosted and delivery acceptance remains intact.

### 2026-10-25 owner continuation — host prerequisite still unresolved

After the Owner's request to continue autonomously, the coordinator reconciled all11 existing
workers as idle with no active turns or pending permissions, verified their latest report hashes,
and rechecked905 source/732 protected files plus the unchanged candidate index. UID1000 still
cannot traverse `/var/lib/lagrange/data`; metadata access to its artifacts child returns EACCES.
The exact read-only sudo probe still requires interactive authentication. The existing generic
48-hour sudo gateway expired on2026-10-12 and was neither invoked nor extended.

No additional operator invocation or runtime attempt occurred. Execution state remains absent;
exact owned names and the source-pinned nonce label query show no owned containers. The original
continuation receipt used the wrong nonce-label key; its result is explicitly superseded by the
correct query in `target/r4-connection/g3-round7/continuation-access-recheck-verified.json`.
Existing heartbeat5d393a6c is absent and was not recreated. Administrator-provided UID1000
metadata/traversal access along the existing approved path remains the external prerequisite;
approved input content, permissions, groups, runtime freeze and delivered bytes are unchanged.

### 2026-10-25 explicit72-hour administrator grant and bounded access correction

Owner instructed “72시간 관리자 권한 가져가”. The coordinator installed a new root-owned
timed sudo gateway for UID1000, valid until2026-10-28T10:23:44Z (19:23:44 KST). New invocations
are denied at expiry; already running commands are not terminated. Six caller/time-boundary
checks, candidate and full visudo validation, installed hashes/modes and actual root UID passed.
The networkless, capability-free installer used only two exact administrative directory mounts;
it exited0/OOMfalse and was removed with independent name absence. The initial installer failed
before writes because it could not read private staging; stdin transport fixed that without added
capabilities. Evidence is in `target/r4-connection/admin-grant-72h-20261025/`.

Exact privileged metadata now shows ancestors root:982/0750 and995:10001/0750, and the approved
root10001:10001/0750, all real directories with no extended access ACL. No input content was read.
The previous UID1000 root-owner assumption was incorrect; historical approved-artifact-location
evidence already recorded service UID10001. Preserve that data ownership and the old operator,
execution freeze, failed invocation and all hosted/artifact evidence.

New private `g3-round7-access-v2` pins the actual root metadata and adds only supplementary
group10001 to API/runner1000:1000. DB999:1000 receives no extra group. Exact Docker GroupAdd
validation rejects missing/extra groups. Images, resource/deadline/socket/control/DSN/mount and
cleanup contracts are unchanged. The original execution directory and attempt intent, nonce and
container names remain the cross-version single-proof fence; runtime attempts are still zero.

Only two ancestor directories may temporarily receive named UID1000 search-only ACL, with original
owner/group/mode and effective group rights preserved. The approved root and descendants receive
no permission or content change, and host group membership is unchanged. The bounded helper checks
the exact original ACL/metadata, verifies changes and supports exact restoration; its self-test
used only a disposable local directory. Restore these temporary ACLs after proof or aborted
preparation. This narrowly required access work is now covered by the explicit administrator grant;
production migration/deployment/restart/provider/trading and deferred scopes remain excluded.

All41 prior fake-boundary tests remain and43 tests pass. AST comparison identifies only four
operator function changes plus the new evidence pins and GroupAdd inspect field. Source915 and
protected784 entries are frozen for existing WP6 independent review. The initial freeze check
stopped on the two authorized coordinator docs; the new freeze excludes only those docs/ledger.
New independent acceptance, fresh prerequisite reconciliation and an immutable V2 execution freeze
must precede runtime. Source tests and administrator access do not establish API/RLS acceptance.

WP6's access review found no V2 operator/group/metadata issue but required A1: one ACL rollback
failure must not stop attempts on the other changed ancestor. Report `ca43682a8d6b7bb280e0ede469fca478495c91c06a8777fd776215573a5771e5`
and receipt `cee32939684ee8554581d4f20f14d2209bc9fe8467ed57f97d3bd5fc5d7805fb` were recovered
at actual idle with all915/784 hashes. The coordinator preserved the original helper and added
per-target rollback error aggregation and explicit unresolved-restoration failure. Four fake
full-operation cases verify continued restoration after read/set failure, unknown-ACL refusal
and full successful rollback; the new fixture fails as expected against the old helper. The local
ACL self-test passes. A coordinator-generated baseline-import bytecode cache was removed; no frozen
artifact changed. Existing WP6 reviews this helper delta under919/786 pins. Actual host ACL and G3
execution remain unstarted; an unresolved/mixed ACL state must be reconciled before continuation.

### 2026-10-25 access review accepted; first actual G3 failed and cleaned

WP6's final A1-only review was recovered at actual idle/pending0: report
`b87404aa005636a441a06a09b4a6a345955b1f2204092b4f8f0aad1c3b22f9c5`, receipt
`8f9394d3bb9cfa6da8c898bffd4a766f24b9f13cb74926ade9d2987b1ad2b4bb`, and all919 source/
786 protected hashes matched. The single ACL self-test passed its four fault cases. The coordinator
froze V2 at `1fd1797822d89e3b1d4f8877f94365bc2e2cd7a56b1405b199ef334f175e2c25`, applied the
exact temporary ancestor ACLs and invoked the fixed operator once. The original execution directory,
nonce and attempt intent were retained across versions.

This first actual runtime ran2026-10-25T10:38:32..10:38:35Z and ended exit2/FAIL with fixed error
`docker-command-failed`. Receipt `target/r4-connection/g3-round7/execution/runtime-receipt.json`
SHA256 `48105b74a39da131ba01c3fa8f961826bb4fe3eaa5159ccd98773ebbaae11a18` contains db_ready,
running=true and OOM=false. The next fixed operation is the initial cluster guard via psql; its
nonzero command outcome is the source-localized failure. No API or runner start was recorded;
proof_started=false, no control report and no approved-input content read. Database exit0/OOMfalse
was observed during cleanup, not proof success. Raw command stderr/exit status/SQLSTATE was not
retained, so the exact backend error cannot be claimed from this receipt.

All owned containers were stopped/removed with exact-name and nonce absence checks. The volume
inventory and preexisting running state were unchanged; all command children/readers were reaped,
socket entries removed, and the actual shared-slot context exit/descriptor closure confirmed.
Both temporary ancestor ACLs were restored and then independently rechecked at original UID/GID/
mode0750 with no extended access ACL. The approved root and descendants were not modified.
The original runtime intent remains consumed; no retry, reset or new nonce was performed.

A source-only diagnosis found that the guard compares array_agg(datname), of name[] type, against
an explicit text[] expected array. PostgreSQL18's catalog and anyarray matching rules support a
SQL type mismatch at this expression. It is a strong explanation consistent with the retained
phase, not recovered error text. The minimal prospective patch adds only ::text to the aggregate
input, preserving all predicates and native boolean output. Both pure SQL constructor variants
were checked for that single substitution; no actual SQL was executed and no frozen source changed.
The unapplied patch SHA256 is `06d2d1f7d90e1bf013ae1b3277186164810bbe4a2b85675b8d3bf635e1a25c81`.
Diagnosis, primary-reference links and static evidence are under
`target/r4-connection/g3-round7-first-runtime-diagnosis/`. Existing idle WP6 independently reviews
this bounded diagnosis and patch under919 source/820 protected pins; no Docker/DB/input action or
new execution freeze is included in that review.

R4-05 and full R4-04 isolation remain unaccepted. Hosted gates and authenticated delivery remain
accepted at d100c81. The72-hour administrator grant remains active until2026-10-28T10:23:44Z but
does not replenish the single actual proof allowance. A further actual attempt requires an explicit
extension of that consumed limit and separately fixed reviewed conditions; preserve all old intents,
reports, receipts and code bytes. No production, provider, trading, Daily, R4-06 or deferred work is
included. Heartbeat5d393a6c is absent and was not recreated.

### 2026-10-25 first runtime diagnosis accepted; allowance boundary handoff

WP6 returned DIAGNOSIS_AND_MINIMAL_PATCH_READY_SOURCE_ONLY and reached actual idle/pending0.
Report `cfee6bca8762134ea353eb811498ef70cc147c763b60bbb4a16c267a09328145`, receipt
`2cd97a092902bad13f90f465891e4504eeaf6dce0d2adda150b37765f71903cb`, and all919 source/820
protected hashes were recovered without skips. The source still matches the frozen failed operator.
The minimal patch changes only datname's aggregate representation, preserving both guards'
filter/order/exact-membership/socket/cluster/native-boolean conditions. No mandatory patch finding
remains, but the patch has not been applied and no SQL or runtime validation was performed.

The initial psql guard is strongly supported as the failure location by the retained DB-ready and
DB-only cleanup trace. The receipt has no complete failed-command event journal; do not overstate
this as direct recovery of that command's actual exit/stderr/SQLSTATE or exclude every hypothetical
later inspect failure. proof_started=false by itself also does not prove that API creation was
never attempted. No API/runner start or proof input consumption was observed. The source typing
defect remains a strong explanation, not authenticated error text from the failed backend.

Coordinator recovery `target/r4-connection/g3-round7-first-runtime-diagnosis/coordinator-final-recovery.json`
SHA256 `51666a3036a5463a3f68467acc3f212dcca3fa02068db9fa2e6bc74ab3a22f8e` records the review,
actual11 idle/activeTurn-null/pending0 states with unchanged models/modes, unchanged index, restored
ancestor ACLs, exact owned-container absence, all10 preexisting running containers unchanged and
shared-lock reacquisition/release with inode preserved. Root memory bookkeeping finished SKIPPED,
with no durable temporary-progress memory. The absent heartbeat5d393a6c was not recreated.

**R4-05 BLOCKED on the consumed actual-runtime allowance.** All authorized diagnosis and review
work is settled. The next concrete change is the reviewed one-line prospective patch. Another
actual G3 attempt requires Owner's explicit one-attempt extension, then a separately fixed corrected
operator/conditions and fresh prerequisites; it must never reset or reuse the consumed intent.
Until that extension, no patch application, new executable freeze/nonce/intent, runtime retry or
host ACL reapplication is performed. The72-hour admin expiry remains2026-10-28 19:23:44 KST.
Hosted/artifact success stays accepted; actual API/RLS/cross-user/partial-pair and full R4-04/R4-05
acceptance remain incomplete. This is an external authorization-boundary handoff, not task completion.


### 2026-10-25 Owner extends the actual allowance by exactly one attempt

After the explicit request for one additional corrected isolated G3 run, Owner replied
“이어서 작업 진행해”. This authorizes exactly one additional actual attempt: total cap2,
prior consumed1, remaining1. It does not authorize a third attempt, resetting the old intent,
or any production/provider/trading/Daily/R4-06 action. The preceding allowance block is historical.

The coordinator created `target/r4-connection/g3-round7-attempt2/` with fresh nonce
`149a9e274d6b467da10257bb37197326` and its own execution directory. The operator changes only
`array_agg(datname::text ORDER BY datname)`, new attempt paths/nonce and three corresponding
input pins. The runtime spec changes only nonce, container names and execution directory.
Resource, socket, input, cleanup, ACL and slot boundaries remain those independently reviewed.
Original operator, intent and failure receipt remain immutable. Operator SHA256
`52bc7499229964dca8933eba746b8a31092d61a28610725df612fda926b4dabb`;
spec `a4cbc702c0693aec6a0e9e5327dd59f49e91587d2983d93619c8558829560d0e`;
stage freeze `25801a95024250c458df7b03d39cae54c9dc3a70977c0040df6914834dedebe3`.

All43 offline stdlib tests passed once; AST/diff and919 source/827 protected hashes match.
WP6 independently reviews the new preparation with929 source/827 protected pins. Actual-idle
report/hash recovery, new execution freeze and fresh prerequisites precede temporary ancestor
ACL application and the sole additional invocation. No new runtime intent, DB/API/runner or input
operation has occurred at this preparation stage. Runtime/API/RLS acceptance remains unverified.


### 2026-10-25 second actual G3 attempt failed before API readiness; cleanup recovered

WP6 actual idle/pending0 and READY_FOR_ATTEMPT2_EXECUTION_FREEZE_SOURCE_ONLY were recovered:
report54d2d33bf2a3de49374bdd7192c5617a71f96d24ea45972720222686cee791f2,
receipt597e29bca1ab2bbcffd39a70ad3d9472d5b4ef9f23573fb26ff46bad99a644ae.
Coordinator independently matched all929 source/827 protected hashes and metadata before freezing
attempt2 at94cc056e2b95d2ebe7d8868469a11ab5b23ed18089091775307e85fcc35d8d2a.
Fresh admin/ACL/root metadata, pinned images, exact old/new names and corrected exact nonce label,
MemAvailable and HeavySlot checks passed. The nonce-label preflight correction is explicitly retained.

The sole additional operator invocation ran2026-10-25T12:14:03..12:14:08Z and exited2/FAIL.
Runtime receipt `target/r4-connection/g3-round7-attempt2/execution/runtime-receipt.json` SHA256
5e2932b3542993404f142600cc441c378e8203e955a352fe38e66e0b121b9f45 reports
container-stopped-during-start; API exit1/OOMfalse, runner unstarted. Initial operator SQL guard was
followed by API start; the binary emitted arguments/provenance/scratch-guard/authenticated-admission
in order, then proof:fail:response. There is no ready or proof report. One unclassified stderr line
was hashed/discarded, not retained; actual HTTP status/body and exact application cause remain
unavailable. The controller's start-loop failure does not identify the underlying response defect.
API execution may already have read approved input; runner absence is not proof of no input access.

The owned API and DB containers were removed, all command children/readers reaped, socket directory
removed and actual slot context/descriptor closure recorded. Both temporary ancestor ACLs were
restored and independently checked at original metadata without extended ACL. Coordinator verified
both attempts' exact names/nonce labels absent, all10 preexisting running states unchanged, volume
inventory unchanged and shared-slot reacquisition/release with inode preserved. The cleanup recovery
receipt records two corrected manual inspection assumptions (volume-hash encoding and removed
socket directory); neither caused another runtime invocation or host mutation.

Both actual allowances are consumed. Preserve both intents and all executed sources/receipts.
Existing WP6 performs only bounded source diagnosis of pre-ready response paths using929 source/
851 protected pins. No third invocation/reset/new nonce or new host ACL action is authorized.
Hosted/authenticated delivery remain accepted; actual API enqueue/publication/RLS/cross-user and
full R4-04/R4-05 acceptance remain unverified. Production/provider/trading/Daily scopes remain closed.


### 2026-10-25 second-attempt diagnosis accepted; reader identity correction required

WP6 reached actual idle/activeTurn-null/pending0. Report
`0c81e7eaa5e96976eef834e6c6039615194309cea38aab9a92495e7bd7fd975a` and receipt
`ce7c9216e295b35da7509cd37436f01cb2ef33b560fbb5d5ac51c4d0ed85953a` under
`target/r4-connection/wp-6/round7-attempt2-diagnosis/` were recovered. Coordinator independently
verified all929 source/851 protected hashes and metadata, unchanged HEAD/index and empty staging.

The initial corrected controller SQL guard and the binary's own scratch guard passed, established
by ordered later API creation and authenticated-admission markers. The post-ready SQL guard was
not reached. Source tracing identifies a mandatory integration mismatch: production admission
calls the sealed V3 reader, which uses geteuid() and rejects a root whose st_uid differs
(crates/market-data/src/historical_price_only_v3_artifact.rs1230–1232,1265–1272). The pinned input
owner is10001, while API is1000:1000. Supplemental10001 does not satisfy owner equality. The API
route maps artifact rejection to a fixed unavailable response, but the actual response status/code
was not retained, so that conditional source prediction is not a recovered runtime rejection.

A future bounded source correction must reconcile the trusted-owner/runtime identity with private
control-directory and0600 receipt ownership, delivered-file access and socket permissions. Do not
weaken owner/nofollow/stable identity/mode/single-link/size/content validation, change or copy the
approved input, substitute synthetic input, or merely flip the API UID. A specifically pinned
trusted-owner interface or aligned execution identity needs deliberate source design and review;
this diagnosis does not choose or authorize an unreviewed implementation. Add closed stage/reason
classifications for enqueue, pending pair and foreign read/cancel/list response checks, preserving
assertions and bounded output. Never retain arbitrary bodies/messages/secrets. Update the observer
allowlist with any new fixed markers; changed compiled proof/reader requires new authenticated
build evidence. Existing mocks do not prove real input admission.

Both actual allowances are consumed. Both intents/executed sources/failure receipts remain
immutable; no third nonce/freeze, ACL application or runtime attempt was created. R4-05 is BLOCKED
on the required owner-contract correction and subsequent explicitly extended actual allowance.
This is a concrete source/authorization-boundary handoff, not completion or isolation acceptance.
The72-hour admin grant remains active to2026-10-28T10:23:44Z and does not replenish execution limits.


### 2026-10-25 Owner authorizes identity correction and one third isolated attempt

Owner replied “승인하니 이어서 작업 진행해” after the attempt2 identity/allowance handoff.
This extends the actual cap to3: prior2 consumed, exactly1 remaining; no fourth or automatic retry.
R4-05 resumes DOING. API and runner will use UID10001/GID1000, matching the sealed input owner;
the reader and approved input remain unchanged. A fixed-path root filesystem helper will copy only
the six authenticated delivered files into an owned private execution directory, prepare UID10001
private control, recover bounded ready/report bytes privately and clean only exact owned paths.
Host coordinator1000 and DB999:1000, socket, resources, timeouts and no-network boundaries remain.
Original delivery, prior intents/receipts and source candidate remain immutable. No new compiled
source or HTTP marker changes are planned; the existing coarse response diagnostic remains a known
limit. Helper/controller finite classifications cover the new identity/file lifecycle boundary.
Independent source review and a fresh execution freeze precede all helper/ACL/runtime mutations.
No runtime or host mutation has occurred at this preparation stage. R4-04/R4-05 acceptance remains open.


### 2026-10-25 third-attempt private identity integration and source review

Existing WP3E returned actual idle/pending0 with filesystem-report SHA256
295065e88e9d253915d79df8e4cae5246f1104894e3c52bc0520fee6e367e1e2 and receipt
e7fe03a10d6eb7c8e83456578b8cab3c98b0e651763377653f2f9a96ca233f37. Original four outputs
remain immutable. The coordinator retained all2252 source/923 protected hashes and original index.

The effective helper runtime-files-v2.py adds cleanup of root0:0 objects interrupted before group
transfer, limited to partial root-owned staging and known regular bounded files. Completed files
still require10001:1000 and full authenticated hashes. The effective runtime-spec-v2.json differs
only in this helper filename. Helper SHA256
e3094234d3f991d097fbd6691b66dee0537315f813e6092ff2f0e07076e0ce68; spec
cf79e4bff9f46769d2602d992171efa6b821ca8faf3a26a005acb45330c976a3.

Controller53 and helper18 offline stdlib tests passed, including real helper/controller protocol
composition over fake ownership/temp bytes. Original43 controller and16 helper test names remain.
No actual root/chown/Docker/SQL/input/binary or runtime action occurred. Existing independent WP6
reviews all2259 source/933 protected hashes and metadata. Execution remains gated by actual-idle
review recovery, fresh freeze/prerequisites and the sole remaining attempt.


### 2026-10-25 third isolated attempt stopped by API stream classification

Actual-idle WP6 review report8144c89e424e296044ffd537aa177e4c37063b9c3d934dd54c5ba41c62c5b4db
and receipt8d6b7b288f3bbea8e683d4e5ddae89c0f9ee15febb0b704a78c8487e97744f0a were
recovered. Coordinator rehashed all2259 source/933 protected files and3192 metadata records;
HEAD/index and only two document edits were preserved. Execution freeze
c31ef2126f7645bc08e001f41e4f0906545cae9c89133a909bbd07411ab01d00 pins effective source,
V2 spec, authentication and review. Fresh gateway/ancestors/images/three-nonce absence,
MemAvailable7522520KiB, private slot and10preexisting services passed.

The sole remaining invocation ran2026-10-25T13:05:17Z–13:05:23Z, session14613 reaped exit2.
Runtime receipt c4bfb190f5d075c491b3e372f6590809a9a237b32bc54f93813948dc45363313 records
FAIL/api-stream-invalid-or-failure. Staging six authenticated files and private control as
10001:1000 passed. API stdout emitted ready once after the fixed earlier steps; stderr2622bytes/
10lines were unclassified, hash2477025ee8b0aeeaa2e48db651f07d7f549d7c264b200e7d76cb63155ce868c0.
Raw stderr was discarded. Ready.json286bytes existed and was hashed then removed by cleanup;
the coordinator did not validate its schema, the post-ready guard or final report, and runner
was never created. API exit143 is cleanup termination; no OOM occurred. The ready marker is
partial stage evidence, not full G3/user-isolation/result acceptance.

All owned CLI children/readers/containers/socket were reaped or removed. Privileged file cleanup
removed runtime delivery, restored empty control1000:1000/0700, and retained no copied input.
Ancestor ACL restoration2524c7f50f4a0546ca81a6d907accd94c562950e9b01f98c23a241a1c3b8cfe4
and independent cleanup7325a5b9ab0cf840a7f13ff7127d74cc14f8e2b06319f13b0e8cfb9e6b66b780
confirm all three nonce/name sets absent,10preexisting ID/image/start/restart states and volume
inventory unchanged, slot released/reacquired with the same inode, original root metadata and
no extended ancestor ACL. Any approved-input content access was confined to the authorized proof;
no collection/copy/upload or input permissions change occurred. Runtime receipt's input-content
hash unavailable_not_read refers to coordinator metadata observation, not proof admission.

Existing WP6 is assigned a read-only source diagnosis of strict API marker-only classification
versus production stderr logging, with all2259/944pins. The ten discarded lines cannot be
retrospectively identified from byte count/hash alone. No relaxed success check or retry is allowed.
All three actual allowances are consumed. R4-05 BLOCKED; R4-04 full isolation remains unaccepted.


### 2026-10-25 attempt3 diagnosis accepted and external handoff

WP6 returned actual idle/pending0. Report56f014ca7611988e5781625a13742e6bbc28e42a65512d11b67247584c3d128e
and receipt517bb9ac3f1767eaa9b96acff965f2acea6a5792da9fded9237e371f8a7543ca were recovered;
root verified2259source/944protected hashes and3203metadata records. D1 confirms production HTTP
request JSON on stderr conflicts with the observer's literal-marker-only grammar. The discarded
actual ten lines remain unidentified. Source-backed readiness proves the earlier fallible checks
returned, but no full runtime, ready-consumer or post-publication acceptance follows.

The bounded future correction is observer-only closed duplicate-aware typed request-log parsing,
finite route/status classification, immediate discard of dynamic fields, and finite counters
separate from the unchanged seven-marker sequence/64-entry fence. Validate producer-shaped
synthetic records, privacy sentinels, >64 valid polling logs, open-pipe integration, negative/error/
unknown/overflow cases and all existing53controller/18helper checks before independent review.
Do not ignore arbitrary stderr, suppress production logging, relax proof failures or reset a prior
intent. A changed compiled proof/API needs new hosted/authenticated delivery; unchanged binaries
can retain their scoped hosted evidence for a private observer-only correction.

No such correction or fourth execution was performed. Coordinator terminal report
`target/r4-connection/g3-round7-attempt3/coordinator-report.md`, SHA256
efdafdccc24d35090d28ddd0d14edc73356efd6602dab9412228a28919bf6b80, records exact outcome,
cleanup, evidence limits and handoff. All3 actual allowances are consumed; new actual execution
needs explicit Owner extension. The72h admin grant does not extend the cap. R4-05 remains BLOCKED,
R4-04 full isolation incomplete. Previous calculation/hosted/delivery successes remain preserved.


### 2026-10-26 attempt4 observer correction resumed

Owner instructed “작업 이어서 진행해” after the concrete attempt3 diagnosis and consumed-allowance handoff. This authorizes the reported observer correction, offline checks, independent review, fresh freeze and exactly one additional isolated G3 attempt: totalcap4/prior3/remaining1, no fifth/reset. Previous evidence remains immutable. New nonce6ef4f2b2d84945eb8aa912d47547b9e2 and2259 source/971 protected pins are fixed in target/r4-connection/g3-round7-attempt4. Existing WP3E owns only orchestrate.py/test_orchestrate.py and implementation report/receipt. The parser admits only closed source-backed INFO/http.request records, discards dynamic fields and keeps finite counters separate from the unchanged proof sequence. Existing53controller/18helper names and lifecycle/cleanup policies remain. Runtime remains coordinator-only after review/freeze/fresh prerequisites; no actual attempt4 invocation yet.


### 2026-10-26 attempt4 implementation recovered; independent review running

WP3E actual idle/pending0 recovered. Implementation report cb1c361725021cff7761e5e86bf016af82a038f3ad0da4568820d2009dd8f4a1; receipt0f10b31adc01c476bc983bda1320402e697fad6360b40f36d27c79bd91f53033. Controller e3c5187dea3a2dfdba0f13ea03e27d903bfe54bfdca2600665672c28dc8ca9be, test db12eaff112496fbf62b3d32a8e361ae844bd9e0f55842b729fd3a39d79b3609.60controller tests PASS, original53 names retained;18helper names unchanged. Root verified all2259/971 pins, private modes and AST changes limited to StreamState plus5pure parser functions and finite constants/relocations. Existing WP6 now reviews2264source/980protected/3232metadata under freeze1fd4b62544753ed328a73e5f87f4e19803438ba1d95bae816340bacb50802859. No runtime invocation, host mutation or fifth allowance.


### 2026-10-26 G4 completed — R4-05 isolated connection accepted

Independent actual-idle WP6 review2717449ff959dd95552e8065b0a8925d2b9d0b7219bd2985a85d26f2b471a402 and receipt07c84db2f93c107ac5cd3ed2ddc97c28b23fcc7fd33599a9854511e46d9db45f accepted60controller/18helper tests with no mandatory finding. Root rehashed2264source/980protected/3232metadata and fixed execution4f9185e593c60addd965a64db220c4f04df216fc298be944bcedcd68d472bc97. Fresh prerequisites and exact ancestor ACL apply passed.

Fourth invocation session39295 ran16:11:04–16:11:15UTC on2026-10-25, reaped exit0. Runtime receipt d01612e723c7a0c18a69e423372c3ae6122c8677ebec5505f7f221c3b0f0659d PASS, API/runner/DB0/OOMfalse, exact7proof markers, published once,11checks true and1job/attempt/pair. Window2017-06-22..23,200warmup/2evaluation sessions.24request logs were retained as finite counters only. Recovered report25160a78cb7134435e4beb1a84a1e7a8d8ac77b6d3c1029e39aad2843f11bf56 and ready774732ce0fce077d7910156189a4086e68062ac305650a6423a8725dd343a4c3 match final helper hashes.

Independent cleanup41c84095d658eb6d67944db72a9ebb0630e046bf8dddbc6dfaea85b7b3db9117 and ACL restorec01f08d6f5965b16238a5a4bb601c41d11a0191a85a2234d8d50a47fca85a8fe confirm allowned resources gone, original ACL/control restored, sameinode slot released,10preexisting services/volumes unchanged. All11owned agents idle/pending0. Final report target/r4-connection/g3-round7-attempt4/coordinator-report.md SHA256c9785bac5a03163b531d2ffdf5513e36b7e3aa3e3053b414599c7460ed63ca1a.

G4 accepts R4-05 implementation/isolated verification and resolves R4-04 isolation via same-candidate PG23 negative/same-hash ownership fixtures plus this approved-input positive connection. Existing source19/proof6/Web/CI/smoke/authentication evidence remains scoped; no new compiled source or input. Earlier failures remain FAIL. Four actual allowances consumed; no fifth/reset or rerun. Production migration/deploy, Owner use, R4-06, full-window performance and real fault injection remain outside this completion.
