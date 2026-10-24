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
