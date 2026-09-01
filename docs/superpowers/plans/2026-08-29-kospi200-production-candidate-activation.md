Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# KOSPI200 개별종목 후보 정식 활성화와 KOSDAQ150 단계 확장 계획

## Goal and boundaries

### 목표

기존 candidate 도메인을 실제 point-in-time 데이터로 연결하여 인증 사용자가
KOSPI200의 장 마감 후 Top 5 연구 후보, 저장형 스크리너, 종목 상세 분석을 사용할 수
있게 한다. KOSPI200 운영 안정성이 확인된 뒤 같은 계약으로 KOSDAQ150을 별도
유니버스로 활성화한다.

완료 조건은 다음과 같다.

1. 가격, 투자자 수급, 재무, 지수 구성, 업종, 시장 상태의 공급자·권리·시점 계약이
   소유자 승인과 immutable entitlement hash로 고정된다.
2. KOSPI200 run은 cutoff 이전에 이용 가능했던 구성 종목과 데이터 revision만 사용하고,
   누락·미래·무권리·오래된 입력에서는 publication을 fail-closed 한다.
3. EOD 가격과 수급은 해당 거래일에 fresh하고, 재무·구성·업종은 exact as-of pin으로
   해석되며 모든 Raw/Curated manifest와 dataset version이 run에 결속된다.
4. 인증 사용자가 KOSPI200 Top 5, screener, 종목 분석과 provenance/freshness를 조회한다.
   결과는 연구 후보이며 목표 비중, 주문 수량, 매수·매도, 기대수익률을 만들지 않는다.
5. exact commit으로 배포하고 인증·RLS·stale/blocker·replay·재시작·복구 QA가 통과한다.
6. KOSPI200이 3개 연속 eligible trading session에서 무인 발행 또는 정당한 typed
   fail-closed 상태를 보인 뒤 KOSDAQ150을 별도 run/rank로 활성화한다.

### 대상 작업공간

- 저장소: /data/worktrees/3puw275b/hungry-zebra
- 계획 작성 기준 HEAD: 559c6dab40cdf19301a2e1cd41c62943140d2eb3
- 이전 설치 source revision: c958c6d277aa5baecdf40577ccc10bb63780a8ca
- 주요 범위:
  - crates/market-data
  - data-pipelines/collectors
  - crates/job-queue
  - crates/factor-engine
  - crates/api-server
  - apps/web
  - migrations
  - deploy
  - scripts/ops 및 scripts/qa
  - configs/data-rights 및 configs/universes
  - docs/decisions, docs/runbooks, docs/STATUS.md

실행 시 최신 main과 설치 revision의 차이를 다시 확인하고 새 실행 기준 commit을
고정한다. 이 계획의 기준 hash를 배포 대상으로 자동 간주하지 않는다.

### 포함 범위

- KOSPI200 우선 활성화와 KOSDAQ150 후속 활성화
- 일봉 기반 20-session scoring과 5/60-session context
- immutable Raw, typed normalization, point-in-time revision selection, exact dataset pins
- KOSPI200/KOSDAQ150별 독립 score normalization, rank, Top 5
- 저장형 screener, 종목 상세, freshness/provenance/blocker UX
- read-only 시장 데이터 수집, 운영 설치, 관찰 및 QA

### 제외 범위

- ETF11 recommendation_* 또는 owner-beta artifact 의미 변경
- target weight, 주문, 계좌, Paper 자동 적용, Live profile, 실거래
- 10년 개별종목 전략 백테스트, 전 시장 동적 유니버스, 상장폐지 전체 역사
- 분봉·틱·호가, 목표가, 기대수익률, 확률 예측, LLM 뉴스 점수
- 현재 구성 종목을 과거에 소급 적용하는 생존편향 결과
- 승인되지 않은 KIS/OpenDART/KRX/KIND/data.go.kr/SEIBro 네트워크 호출
- 계정 생성, API key 발급, 약관 동의, 결제 또는 권리 구매

### 적용 지침과 확인된 사실

- 루트 AGENTS.md의 KIS/OpenDART deny-by-default 경계가 모든 패키지에 적용된다.
  계좌·주문 표면과 account identifier는 계속 금지다.
- apps/web/AGENTS.md에 따라 Web 변경 전 설치된 Next.js 16 문서를 읽는다.
- ADR-0003은 ETF allocation과 stock candidate를 분리하며, licensed PIT KOSPI200
  membership이 없으면 production publication을 BLOCKED로 유지한다.
- STATUS 2.9 기준 KOSPI200/KOSDAQ150 DB·compute·API·Web·synthetic QA vertical은
  구현됐지만 실제 feed는 비활성이다.
- crates/market-data/src/providers/kis_candidate.rs에는 수급·재무 endpoint 코드가
  존재하지만 루트 AGENTS.md allowlist에는 그 path/TR-ID가 없다. 코드 존재는
  live 호출 승인이 아니다.
- kis_candidate_master는 현재 ZIP snapshot을 보존하지만 announcement/effective/
  available history가 없어 require_candidate_master_pit에서 의도적으로 거부한다.
- OpenDART의 현재 승인 표면은 list/corpCode/company뿐이다. 재무 endpoint는 금지돼
  있고 in-process rustls transport도 현재 host와 호환되지 않는다.
- 이전 운영 경험상 11-image 병렬 Buildx는 OOM으로 Paseo daemon까지 종료시켰다.
  production image는 순차 build 후 공식 cache-only manifest builder로 결속한다.
- Mem0의 관련 decision/project_fact 검색은 2026-09-01 초기화 전까지 quota exceeded다.
  계획은 저장소 ADR, STATUS와 실제 코드에 근거한다.
- 호스트에는 /home/l1nnx/.agents/skills/paseo-delegate/SKILL.md가 존재한다. 실행자는
  시작 직전에 그 현재 내용을 완전히 읽고 따라야 한다.

### 미해결 요구사항

다음은 WP-1 결과를 본 뒤 소유자가 승인해야 한다. 추측으로 메우지 않는다.

1. PIT KOSPI200/KOSDAQ150 membership, sector, market-status의 정확한 공급자와 권리
2. investor flow와 fundamentals의 정확한 공급자, endpoint, revision/availability 의미
3. KIS candidate REST path/TR-ID와 public master download를 allowlist에 추가할지 여부
4. OpenDART 재무 표면 또는 두 번째 TLS stack을 열지, 다른 licensed 공급자를 쓸지 여부
5. Raw/Curated 보존, 내부 표시, 파생 score 표시, 사용자 범위에 대한 entitlement 문구
6. v1 가격 정책: adjusted vendor snapshot 사용 여부와 기업행사 blocker 처리
7. 일별 request budget, backfill 기간, credential 소유·회전·폐기 절차

Gate A가 이 일곱 항목을 닫기 전 WP-3 이후 패키지는 실행하지 않는다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | 공급자·권리·PIT·새 network surface 결정 자체가 산출물이고 잘못 고르면 생존편향 또는 무권리 publication이 된다 | high | 공식 문서 간 충돌, availability/revision 의미 부재, 권리 문구 불명확, transport 불가면 대안 비교를 추가하고 소유자 결정 전 중단 |
| WP-2 | intermediate | 이미 구현된 vertical을 전수 확인하고 기계적 테스트로 기준선을 만들 수 있으나 여러 crate/SQL/Web을 가로지른다 | high | 테스트가 재현되지 않거나 문서와 코드가 다르면 해당 seam은 hard 진단으로 재분류 |
| WP-3 | hard | PIT membership/sector/status는 미래정보와 생존편향 경계이며 현재 KIS master로 증명할 수 없다 | high | effective/available/revision을 공급자가 주지 않거나 identity join이 fuzzy하면 구현 중단 후 WP-1 재개 |
| WP-4 | hard | 가격·수급·재무와 기업행사 시점·정정·pagination·rate를 하나의 fail-closed evidence 계약으로 맞춰야 한다 | high | undocumented pagination, 재무 restatement 부재, corporate-action 모순, allowlist 미승인이면 구현 중단 |
| WP-5 | intermediate | source 계약이 고정된 뒤 기존 candidate pipeline, DB pin, runner에 연결하는 지정 구현이며 통합 테스트가 있다 | medium | 기존 schema가 staged universe activation 또는 exact pins를 표현하지 못하면 migration/architecture를 hard로 승격 |
| WP-6 | intermediate | 기존 API/Web surface를 enabled universe와 실제 blocker/freshness에 맞추는 범위가 명확하고 E2E로 검증 가능하다 | high | API가 provider detail을 노출하거나 disabled universe 호환성 변경이 필요하면 coordinator 판단 요청 |
| WP-7 | intermediate | 기존 release/backfill/healthcheck 패턴을 따르지만 secret, root path, systemd, rollback 위험이 있다 | medium | 기존 installer가 candidate secrets/pins를 안전하게 표현하지 못하거나 restore scope 누락 시 hard 운영 설계로 승격 |
| WP-8 | hard | 독립 검토의 결론이 산출물이며 PIT, rights, RLS, release를 교차 검증해야 한다 | high | P0/P1 또는 검증 불가능 항목이 하나라도 있으면 배포 패키지로 진행하지 않고 수정 패키지를 재계획 |
| WP-9 | hard | production 데이터 취득·등록·배포는 외부 상태와 높은 실패 비용을 갖고 정확한 rollback/QA가 필요하다 | high | provider response drift, throttling, partial Raw, pin 불일치, OOM, 인증/RLS 실패 시 즉시 rollback 또는 fail-closed |
| WP-10 | intermediate | KOSPI 계약과 코드가 일반화됐다는 전제에서 KOSDAQ150은 반복 가능한 활성화다 | medium | 새 권리·mapping·provider가 필요하거나 cross-universe rank/dedupe 문제가 발견되면 hard로 재분류 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 0 | hard | official source와 entitlement/PIT 계약을 결정 가능한 상태로 만든다 | 새 source ADR/runbook 및 data-rights 제안만; 제품 코드 금지 | 없음 | gpt-5.6-sol, high; $paseo-delegate가 실행 시 최종 해석 | exact source matrix, 대안/차단점, owner decision record 초안 | 공식 1차 자료 링크·필드·path/TR-ID·시간·권리 교차검증, git diff --check |
| WP-2 | 0 | intermediate | 현재 candidate vertical과 테스트 기준선, 실제 missing seam을 전수 감사한다 | read-only 코드/SQL/배포 검사와 새 review 문서만 | 없음 | gpt-5.6-terra, medium | current-state seam matrix와 재현 가능한 baseline report | targeted Rust/SQL/Web/QA tests, 0-test 탐지, worktree hash 기록 |
| WP-3 | 1 | hard | 승인된 PIT membership/sector/market-status/issuer identity를 immutable ingest한다 | 해당 provider/normalizer/types/fixtures/tests; 가격·수급·재무 금지 | WP-1, WP-2, Gate A | gpt-5.6-sol, high | KOSPI200 PIT identity bundle과 KOSDAQ-ready generic contract | cutoff/future/correction/gap/duplicate/identity/entitlement tests |
| WP-4 | 2 | hard | 승인된 가격·수급·재무 입력을 exact Raw/Curated 계약으로 만든다 | 해당 provider/normalizer/backfill tests; candidate compute/API 금지 | WP-3 | gpt-5.6-sol, high | 60-session price context, daily flow, PIT fundamentals, action policy evidence | request contract, pagination/rate, revisions, no-secret, malformed/partial/replay tests |
| WP-5 | 3 | intermediate | source bundle을 DB dataset pins, research-worker, candidate-runner에 연결하고 KOSPI만 staged enable한다 | collectors, job-queue candidate, migrations/deploy schema checks | WP-3, WP-4 | gpt-5.6-luna, max; 2회 검증 실패 시 terra medium | end-to-end KOSPI candidate run, exact pins, KOSDAQ disabled state | disposable PostgreSQL ingest→publish→replay, RLS, rollback, stale/block tests |
| WP-6 | 4 | intermediate | enabled universe와 실제 readiness를 API/Web에서 정확히 표현한다 | api-server candidate/screener, OpenAPI/client, apps/web candidate surfaces/tests | WP-5 | gpt-5.6-luna, max; 실패 반복 시 terra medium | KOSPI Top 5, screener, detail, blocker/freshness UX | Rust HTTP tests, Web unit/type/lint, Chromium E2E, Member/RLS tests |
| WP-7 | 4 | intermediate | credential·backfill·health·release·rollback 운영 경로를 만든다 | deploy, scripts/ops, scripts/qa, candidate runbook; 제품 코드 금지 | WP-5 | gpt-5.6-terra, medium | operator-gated plan/check/execute, static checks, rollback/recovery runbook | shell static/self-tests, compose config, secret redaction, disposable restore |
| WP-8 | 5 | hard | 구현을 독립적으로 거부 가능하게 검토한다 | read-only 전체 범위와 새 review report; 수정 금지 | WP-6, WP-7 | gpt-5.6-terra, high | P0/P1/P2/P3 findings, acceptance 또는 rejection | full relevant test matrix, PIT/rights/network/RLS/release traceability |
| WP-9 | 6 | hard | KOSPI200 production 데이터를 봉인·등록·배포하고 직접 QA한다 | 승인된 운영 host/data/DB/release와 증거 문서 | WP-8 acceptance, Gate B | gpt-5.6-sol, high | exact release, KOSPI feed, production QA transcript, rollback evidence | provider preflight, immutable pins, sequential builds, auth/RLS/API/Web/restart/backup QA |
| WP-10 | 7 | intermediate | 관찰 기간 후 KOSDAQ150을 같은 계약으로 별도 활성화한다 | KOSDAQ entitlement/pins/activation/QA와 증거 문서 | WP-9, Gate C | gpt-5.6-terra, medium; 새 source면 sol high | KOSDAQ Top 5, both-universe screener, separate rank/provenance | KOSDAQ ingest/publish/replay, duplicate-context E2E, production health |

Wave 0의 두 패키지는 mutable scope가 다르므로 병렬 가능하다. WP-6과 WP-7도
제품 surface와 운영 파일이 분리돼 병렬 가능하다. 나머지는 shared contract 또는
production 상태를 순서대로 소비하므로 직렬이다. 실행자가 병렬 작업에 isolated Paseo
workspace를 사용하더라도 coordinator가 wave gate에서 한 번에 하나씩 통합한다.

## Worker briefs

### WP-1 — source·권리·PIT 결정 패키지

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: hard. 공식 source, 권리, 시간 의미 선택 자체가 correctness다. confidence high.
- escalation: source가 effective/available/revision을 증명하지 못하거나 약관이 내부 파생
  표시를 허용하는지 불명확하면 코드를 제안하지 말고 blocker와 대안을 보고한다.
- 목표: 여섯 데이터 영역과 issuer identity에 대해 production에 사용할 exact provider
  계약을 소유자가 승인할 수 있는 문서로 만든다.
- 알려진 사실:
  - KIS EOD allowlist는 ETF11 price scope에서 승인돼 있다.
  - kis_candidate.rs의 investor/finance path는 live allowlist 밖이다.
  - KIS master ZIP은 현재 snapshot 증거지만 PIT history를 증명하지 않는다.
  - OpenDART core는 individual-stock issuer identity에는 가치가 있으나 finance endpoint는
    금지이고 rustls transport 문제가 있다.
- 소유 범위:
  - 새 docs/decisions source ADR
  - 새 docs/runbooks candidate source contract
  - 필요 시 config data-rights example 제안
- 금지:
  - provider 호출, credential 사용, key 발급, 제품 코드·기존 migration 변경
  - third-party blog만으로 quota/rights/field를 확정
- 입력: AGENTS.md, ADR-0003/0004, STATUS 2.9/4.4, 기존 provider 코드와 공식 1차 문서.
- 산출물:
  - dataset별 provider/host/method/path/TR-ID/request/response/pagination/rate 표
  - canonical field mapping과 event/effective/available/retrieved/revision 의미
  - retention/Raw/derived display/user audience 권리 표
  - credential/transport/secret redaction과 request budget
  - preferred option, rejected options, unresolved owner decisions
- 검증:
  - 모든 claim에 공식 source locator와 확인 날짜
  - root allowlist와 diff, exact identifiers의 focused fixture test 계획
  - git diff --check
- 필수 보고:
  - 변경 파일과 라인 범위
  - brief와 다르게 처리한 내용과 이유
  - 실행한 검사와 결과
  - 미해결/후속 작업
  - 찾지 못했거나 검증하지 못한 것; 없으면 none

### WP-2 — 현재 vertical 기준선·gap 감사

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: intermediate. 범위는 넓지만 read-only 검사와 기계적 테스트가 있다.
  confidence high.
- escalation: 문서상 완료가 테스트에서 재현되지 않거나 현재 코드를 통해
  credentialed request가 예기치 않게 가능하면 hard incident로 승격한다.
- 목표: existing synthetic vertical 중 재사용 가능한 것과 production에서 실제로
  막힌 seam을 파일·테스트·runtime 단위로 확정한다.
- 소유 범위: 새 docs/reviews baseline report 한 파일. 제품/SQL/배포 파일 수정 금지.
- 검사 대상:
  - migrations 0042~0045와 checksum/rollback
  - market-data candidate contracts/providers/normalizers
  - collector catalog/publish/recovery/health
  - factor/selector/job runner
  - API/OpenAPI/Web/RLS
  - systemd/compose/static/smoke
- 산출물:
  - source별 implemented/tested/runtime-wired/rights-approved/live 상태표
  - required dataset IDs와 exact readiness dependency graph
  - stale, future, missing, replay, correction, cross-universe 동작
  - WP-3~WP-7 예상 파일 목록과 overlap 위험
- 검증:
  - focused cargo tests에 실제 실행 test count 포함
  - migration contract와 candidate QA scripts
  - Web candidate unit/E2E discovery 및 실행
  - git status와 HEAD, 환경 blocker 기록
- 필수 보고 형식은 WP-1과 동일하며 모든 미검증 항목을 none 또는 명시한다.

### WP-3 — PIT identity·universe·sector·market-status ingest

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: hard. cutoff와 생존편향 경계다. confidence high.
- escalation: fuzzy symbol/name join, historical date 추론, announcement/effective/available
  부재, silent current-snapshot fallback이 필요하면 중단하고 Gate A로 되돌린다.
- 목표: 승인된 provider evidence에서 issuer/instrument identity, KOSPI200 membership,
  sector, market status를 typed immutable documents로 만들고 as-of 해석한다.
- 소유 범위:
  - WP-1이 확정한 provider 전용 module과 fixture
  - candidate identity/membership/sector/status type 및 normalizer
  - 해당 market-data tests와 source contract runbook 보강
- 금지:
  - price, investor flow, fundamentals 구현
  - 오늘의 구성·sector를 과거 effective_from으로 소급
  - OpenDART disclosure type을 EOD/candidate에 직접 넣기
- 입력: WP-1 승인 record, WP-2 seam matrix, exact entitlement reference/hash.
- 산출물:
  - exact Raw request metadata와 immutable bytes
  - deterministic issuer↔instrument join
  - announced/effective/available/revision-preserving canonical observations
  - gap/duplicate/correction/delisting/market-halt typed blockers
- 검증:
  - future membership exclusion, correction append-only, interval boundaries
  - duplicate and shifted set fail-closed
  - credential sentinel가 bytes/batch/manifest/log에 없음
  - fixture replay가 byte/dataset identity를 재현
- 필수 보고 형식은 WP-1과 동일하다.

### WP-4 — 가격·수급·재무 ingest와 action 정책

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: hard. provider contract와 point-in-time revisions가 결합된다. confidence high.
- escalation: 새 path/TR-ID가 승인되지 않았거나 fundamentals에 disclosure/revision 시각이
  없고 retrieval time으로 대체해야 한다면 strict PIT로 승격하지 말고 중단한다.
- 목표: KOSPI200 현재 run에 필요한 최소 60-session price context, 거래일별 investor
  flow, PIT fundamentals를 exact Raw/Curated dataset으로 제공한다.
- 소유 범위:
  - 승인된 price/flow/fundamental provider와 typed normalizer
  - pagination/rate/token reuse/retry
  - 60-session backfill와 기업행사/adjustment policy tests
- 금지:
  - 주문/account path, live profile, endpoint 확장 추론
  - 누락 값을 0 또는 forward-fill로 대체
  - unsupported action의 날짜/factor 합성
  - 10년 전 시장 전체 backtest claim
- 입력: WP-1 계약, WP-3 identity/universe, current token manager와 RawStore.
- 산출물:
  - source별 immutable batch/manifest and canonical dataset
  - availability/revision/restatement selection
  - bounded sequential request plan과 resumable backfill
  - adjusted/unadjusted/action 처리의 explicit metadata
- 검증:
  - exact method/path/TR-ID/query/header/page sequence
  - 1 request/sec project ceiling, Retry-After, bounded retry/token reuse
  - malformed JSON/nonzero status/repeated page/partial batch fail-closed
  - 60 sessions, OHLCV, flow class, financial unit/scope/restatement golden tests
  - secret/body/free-form broker message non-disclosure
- 필수 보고 형식은 WP-1과 동일하다.

### WP-5 — dataset pin·DB·worker 통합

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: intermediate. source 계약 뒤에는 기존 vertical 연결 작업이다.
  confidence medium.
- escalation: enabled registry가 KOSPI-only staged rollout을 표현하지 못하거나 migration
  0042~0045 수정이 필요하면 중단한다. 기존 migration은 불변이며 새 migration만 허용한다.
- 목표: WP-3/4의 exact datasets를 하나의 candidate entitlement와 run input identity로
  결속하고 KOSPI200만 production-enabled 상태로 계산·발행한다.
- 소유 범위:
  - data-pipelines/collectors candidate pipeline/sink/worker
  - job-queue candidate schedule/runner
  - 필요한 새 migration과 integration/deploy schema checks
  - runtime dataset/entitlement binding
- 금지:
  - API/Web 변경, production 배포, KOSDAQ150 production enable
  - migration 0042~0045 수정
  - missing source reweight/zero-fill
- 입력: WP-3/4 committed contracts와 fixtures, WP-2 baseline.
- 산출물:
  - common sources + kospi membership exact sealed batch
  - price and source dataset pins in one deterministic input identity
  - KOSPI independent score/run/feed Top 5
  - KOSDAQ disabled but data/schema-compatible activation state
  - stale/blocked/recovery/idempotency health
- 검증:
  - disposable PostgreSQL migration up/no-op/down guard/up
  - Raw→catalog→publish→schedule→compute→publish replay twice
  - RLS, advisory lock, concurrency, correction sequence
  - missing/future/stale/unlicensed source leaves prior feed STALE
  - no recommendation/target/Paper/order table writes
- 필수 보고 형식은 WP-1과 동일하다.

### WP-6 — API·Web 사용자 surface

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: intermediate. existing surface와 deterministic E2E가 있다. confidence high.
- escalation: enabled universe discovery가 API contract를 깨거나 provider/private evidence
  노출이 필요하면 coordinator 결정을 요청한다.
- 목표: 인증 사용자가 production KOSPI200 candidate를 보고, disabled KOSDAQ150이나
  missing/stale data를 오해하지 않게 한다.
- 사전 지침: apps/web/node_modules/next/dist/docs의 관련 routing/data-fetching 문서를
  읽고 Next.js 16 현재 API를 따른다.
- 소유 범위:
  - crates/api-server candidate/screener routes/repos
  - OpenAPI와 generated TypeScript
  - apps/web candidates/screener/stock-analysis components/routes/tests
- 금지:
  - provider Raw/body/credential/internal path 노출
  - buy/sell, target price, probability, global cross-universe rank 문구
  - recommendation_* 또는 Paper 자동 연결
- 입력: WP-5 API/readiness shape와 DB fixtures.
- 산출물:
  - KOSPI feed Top 5와 provenance/freshness
  - tenant-private saved screens
  - flow/fundamental/technical evidence와 3 deterministic scenarios
  - disabled/stale/blocked/no-data typed UX
  - KOSDAQ activation 뒤에도 쓸 enabled universe contract
- 검증:
  - API auth/RLS/cursor/replay tests
  - Web unit, typecheck, lint
  - Chromium Owner/Member/authless, saved-screen isolation, stale/block E2E
  - unknown/disabled universe와 cross-universe fallback 거부
- 필수 보고 형식은 WP-1과 동일하다.

### WP-7 — 운영·backfill·release 준비

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: intermediate. 기존 hardened scripts를 재사용하지만 운영 영향이 있다.
  confidence medium.
- escalation: secret가 env/log/argv에 노출되거나 installer가 exact pin/rollback을
  표현하지 못하면 hard로 승격한다.
- 목표: 코드 변경 없이도 운영자가 plan→check→execute를 명시적으로 수행하고,
  실패 시 partial publication 없이 복구할 수 있는 경로를 만든다.
- 소유 범위:
  - deploy/systemd, deploy/compose, production Dockerfiles
  - scripts/ops candidate source/backfill/install/health/rollback
  - scripts/qa static/smoke
  - candidate production runbook
- 금지:
  - 실제 provider call, production DB write, service restart/deploy
  - secret content 출력 또는 Git 저장
  - legacy static artifact를 alternate activation path로 사용
- 입력: WP-5 binaries/env contract, WP-1 rights/credential contract.
- 산출물:
  - operator confirmation과 exact scope를 요구하는 plan/check/execute
  - read-only secret mounts, UID/mode/no-follow checks
  - resumable 60-session backfill, exact pin registration, health and rollback
  - sequential image build와 cache-only manifest workflow
- 검증:
  - shell static/self-tests와 compose config
  - fake credential sentinel non-disclosure
  - disposable DB/data root partial failure/recovery
  - backup/restore가 새 candidate tables/pins/evidence를 보존
- 필수 보고 형식은 WP-1과 동일하다.

### WP-8 — 독립 acceptance review

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra
- 초기 분류: hard. review conclusion이 배포 게이트다. confidence high.
- escalation: 확인하지 못한 항목을 PASS로 취급하지 않는다. P0/P1은 즉시 rejection,
  core P2는 coordinator가 별도 수정 패키지 없이 면제할 수 없다.
- 목표: source claim에서 UI까지 exact trace를 따라가며 production activation을
  수락하거나 거부한다.
- 소유 범위: read-only 전체 tree와 새 docs/reviews report 하나. 코드 수정 금지.
- 필수 감사:
  - AGENTS network allowlist와 실제 client call graph
  - entitlement/retention/derived display
  - PIT membership, restatement, survivorship, corrections, cutoff
  - no-secret/log/error contracts
  - Raw atomicity, dataset pins, replay/recovery
  - DB migration/RLS/security-definer/cursor
  - ETF recommendation/Paper/Live 비간섭
  - disabled KOSDAQ와 KOSPI-only rollout
- 검증:
  - cargo fmt --all -- --check
  - git diff --check
  - relevant full Rust all-target tests and strict Clippy
  - migration/collector/job/API integration suites
  - Web unit/type/lint/Chromium E2E
  - ops static/self/smoke and disposable backup/restore
- 산출물: finding별 severity, file/line/evidence, verified/not-verified, final verdict.
- 필수 보고 형식은 WP-1과 동일하다.

### WP-9 — KOSPI200 production activation·직접 QA

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra와 승인된 production host
- 초기 분류: hard. 외부 데이터와 production state를 바꾼다. confidence high.
- 실행 전제:
  - WP-8 acceptance
  - Gate B의 별도 production 실행 승인
  - exact release commit/tag/image digests와 tested backup
  - owner-provisioned credentials/entitlements; worker는 발급하지 않는다
- escalation: response drift, rights mismatch, pin mismatch, unexpected action, request
  throttling, partial Raw, OOM, auth/RLS failure면 fail-closed하고 서비스 노출을 열지 않는다.
- 목표: KOSPI200 candidate 데이터를 최소 60 sessions로 준비하고 exact release를 설치,
  사용자 기능을 직접 검증한다.
- 소유 범위: 승인된 read-only provider calls, Raw/Curated/DB pins, release install,
  candidate services, QA evidence와 STATUS/runbook update.
- 금지:
  - KOSDAQ enable, live/order/account/Paper, 미승인 endpoint
  - 병렬 production image build
  - 실패한 batch/pin을 READY로 수동 승격
- 실행 순서:
  1. no-network preflight와 provider request count/dry plan 검토
  2. immutable backup와 rollback target 확인
  3. bounded sequential KOSPI source acquisition/backfill
  4. independent manifest/pin/rights approval check
  5. disposable DB rehearsal
  6. production DB migration 및 exact release 순차 build/install
  7. candidate worker one-shot, replay, daemon health
  8. Funnel/Auth/API/Web/restart/backup QA
- QA:
  - anonymous 401/redirect, authenticated app access
  - KOSPI Top 5, rank/factor/reason/provenance/freshness
  - saved screen tenant isolation
  - stock detail flow/fundamental/technical/scenarios
  - stale/missing/unlicensed path fail-closed
  - same input replay byte/row/idempotency
  - KOSDAQ disabled and ETF11 recommendation unchanged
- 산출물: exact revisions/digests/pins/counts, redacted transcript, rollback status,
  STATUS and runbook evidence.
- 필수 보고 형식은 WP-1과 동일하다.

### WP-10 — KOSDAQ150 단계 활성화

- 작업 디렉터리: /data/worktrees/3puw275b/hungry-zebra와 승인된 production host
- 초기 분류: intermediate. KOSPI generic contract의 반복 적용이다. confidence medium.
- escalation: KOSDAQ source field/rights/identity가 다르거나 새 code path가 필요하면 hard로
  재분류하고 WP-1 형태의 source decision을 먼저 수행한다.
- 실행 전제:
  - KOSPI200이 3개 연속 eligible session에서 성공 또는 정당한 typed blocker를 보임
  - KOSDAQ entitlement와 exact membership pin 승인
  - Gate C의 별도 production 변경 승인
- 목표: KOSDAQ150을 별도 universe로 enable하고 기존 KOSPI 결과와 독립적으로 발행한다.
- 소유 범위: KOSDAQ data acquisition/pins, migration-owner activation, candidate run,
  API/Web/ops QA와 evidence update.
- 금지:
  - 두 universe의 score/rank 합산
  - 동일 종목 dedupe
  - 한 universe 실패로 다른 published feed를 삭제
- 검증:
  - KOSDAQ Top 5, one/both screener, stock detail universe context
  - same instrument가 두 universe에 있으면 두 ranking context 보존
  - KOSDAQ blocker 중 KOSPI feed 유지
  - replay/restart/backup/rollback과 exact release QA
- 산출물: KOSDAQ pins/counts, activation evidence, production QA, STATUS/runbook update.
- 필수 보고 형식은 WP-1과 동일하다.

## Coordinator gates

### 1. Pre-launch checks와 필요한 사용자 결정

1. 실행 직전에 $paseo-delegate SKILL.md를 완전히 읽고 worker launch/monitor/recovery를
   그 스킬에만 맡긴다. native subagent는 사용하지 않는다.
2. 최신 main, worktree cleanliness, 설치 revision, running services를 read-only로 확인한다.
3. WP-1과 WP-2만 Wave 0에서 실행한다.
4. Gate A에서 coordinator가 두 결과를 통합하고 소유자에게 미해결 일곱 항목의 exact
   source/rights/network proposal을 제시한다.
5. 소유자의 명시적 승인 전 WP-3 이후를 launch하지 않는다. 기존 code path는 승인이 아니다.

### 2. Wave별 integration·verification gates

1. Wave 0: WP-1의 source claim과 WP-2의 실제 seam을 대조한다. path/TR-ID, timing,
   entitlement 또는 code state가 다르면 plan을 수정한다.
2. Wave 1: WP-3 결과에서 current snapshot 소급, fuzzy identity, 미래 membership이
   한 건도 없는지 coordinator가 fixtures와 핵심 코드를 직접 확인한다.
3. Wave 2: WP-4의 actual request contract와 Raw atomicity, rate/token/secret tests를
   확인한다. unsupported action/financial revision은 fail-closed여야 한다.
4. Wave 3: disposable DB에서 exact pin, RLS, replay, stale/block, KOSPI-only staged
   enable을 통합 검증한다.
5. Wave 4: WP-6과 WP-7을 하나씩 통합한다. Web 생성물·OpenAPI와 runtime env/static
   checks가 같은 contract를 가리키는지 확인한다.
6. Wave 5: WP-8에 clean candidate를 제공한다. rejection이면 finding별 replacement
   package를 새로 계획하며 기존 WP를 임의 재실행하지 않는다.

### 3. 최종 end-to-end acceptance

Gate B는 다음을 모두 만족하고 사용자가 production 실행을 별도로 승인해야 열린다.

- source/rights/retention/network allowlist 승인 record
- WP-8 P0=0, P1=0, core P2=0
- 모든 required test가 실제 1개 이상 실행되고 PASS
- exact commit/image/pin/backup/rollback rehearsal
- KOSPI200만 enabled, KOSDAQ150 disabled

WP-9 이후 coordinator는 직접 다음을 확인한다.

- provider request count와 endpoint가 승인 계획과 일치
- Raw/Curated/DB/run/UI가 동일 pins를 가리킴
- 인증·RLS·stale/blocker·replay·restart·backup QA
- ETF11 recommendation과 Paper/Live 비간섭
- STATUS.md와 production runbook에 실제 결과만 기록

Gate C는 KOSPI200의 3-session 관찰, KOSDAQ 권리/pin, 별도 production 승인을 요구한다.
WP-10 이후 두 universe의 독립 feed와 one/both screener를 재검증한다.

이 계획은 실행 승인, 새 network surface 승인, credential 사용 승인 또는 production
변경 승인이 아니다. 실행 중 요구사항 누락·충돌이 발견되면 해당 branch를 멈추고
execution graph와 worker brief를 수정한 뒤에도 모든 replacement package를
$paseo-delegate로만 실행한다.
